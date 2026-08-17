//! `measure-pursuit` — the membership-read micro-bench (#29).
//!
//! Answers one question with numbers instead of assumptions: **do the
//! pursuit reads stay flat when the library holds the documented
//! 100k+ assets?** The reads under test are the ones the pursuit view
//! composes — `find` + `events_of` + `list_rounds` + `returns_of` —
//! plus the listing's one-window standing read. The design bets on
//! index seeks over the V80 lookup columns; this command is the bet's
//! receipt, and the number that decides whether a materialised
//! projection (job-built) is ever needed.
//!
//! Unlike `measure-import` / `measure-cold`, nothing here observes a
//! server: the reads have no transport yet (that is a later slice),
//! and what needs measuring is the storage access path, not HTTP. The
//! command is therefore fully self-contained — it seeds a throwaway
//! profile in a temp directory and measures through the same
//! repository adapters the application wires, so the numbers are the
//! adapters' numbers, not a lookalike query's. The seeding cuts one
//! corner knowingly: the 100k-row noise floor and the `_trace` return
//! notes are raw batched SQL (volume, not schema fidelity — the noise
//! rows only need to exist and carry no claim), while everything with
//! a domain constructor (pursuits, dispatches, snapshots, events)
//! goes through the ports.
//!
//! The run is warm by construction (the process that seeded the pages
//! measures them); a cold-cache number would need a separate process
//! and is not what the index-vs-scan question turns on.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::forge::dispatch::DispatchJob;
use asterism_core::domain::forge::pursuit::{Pursuit, PursuitEvent, PursuitEventKind};
use asterism_core::domain::repository::{
    DispatchRepository, PursuitRepository, SnapshotRepository,
};
use asterism_core::domain::snapshot::Snapshot;
use asterism_core::domain::value::{AssetId, PursuitId};
use asterism_infra::sqlite;
use asterism_infra::sqlite::repo::{
    SqliteDispatchRepository, SqlitePursuitRepository, SqliteSnapshotRepository,
};
use chrono::Utc;
use clap::Args;
use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

/// Result schema of `measure-pursuit`.
pub const SCHEMA_PURSUIT: &str = "bench-pursuit-view-v1";

#[derive(Debug, Args)]
pub struct MeasurePursuitArgs {
    /// Noise floor: plain asset rows that carry no claim. The default
    /// is the documented steady-state scale.
    #[arg(long, default_value_t = 100_000)]
    pub assets: u64,
    /// Pursuits seeded with rounds, returns and events.
    #[arg(long, default_value_t = 200)]
    pub pursuits: u64,
    /// Dispatch rounds per pursuit.
    #[arg(long, default_value_t = 3)]
    pub rounds_per: u64,
    /// Returns per pursuit, spread across its rounds via the dispatch
    /// join, plus a fixed 3 direct-claim returns on top.
    #[arg(long, default_value_t = 30)]
    pub returns_per: u64,
    /// Result file directory. Defaults to the same out dir the other
    /// measurements write to.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct OpStats {
    samples: usize,
    p50_us: u128,
    p95_us: u128,
    max_us: u128,
}

#[derive(Debug, Serialize)]
struct PursuitBenchResult {
    schema: &'static str,
    at: String,
    assets: u64,
    pursuits: u64,
    rounds_per: u64,
    returns_per: u64,
    returns_of: OpStats,
    view_composite: OpStats,
    list_standings: OpStats,
}

fn stats(mut samples: Vec<u128>) -> OpStats {
    samples.sort_unstable();
    let pick = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    OpStats {
        samples: samples.len(),
        p50_us: pick(0.50),
        p95_us: pick(0.95),
        max_us: *samples.last().expect("non-empty samples"),
    }
}

pub async fn run(args: MeasurePursuitArgs) -> Result<()> {
    let needed = args.pursuits * (args.returns_per + 3) + 1;
    anyhow::ensure!(
        args.assets >= needed,
        "--assets {} cannot carry the fixture: {} pursuits x ({} joined + 3 direct) \
         returns + 1 snapshot member need at least {needed} rows",
        args.assets,
        args.pursuits,
        args.returns_per
    );
    let tmp = tempfile::tempdir().context("temp profile dir")?;
    let (isle, driver) = sqlite::open_and_migrate(tmp.path().join("asterism.db"))
        .await
        .context("open temp profile")?;

    let ctx = AttributionContext::asserted(None, None).expect("empty assertion");
    let now = Utc::now();
    let persona = Uuid::now_v7();
    isle.call(move |conn| {
        conn.execute(
            "INSERT INTO persona (id, name, display_order, archived, created_at, updated_at) \
             VALUES (?1, 'bench', 0, 0, 0, 0)",
            params![persona],
        )
    })
    .await?;
    let persona_id = asterism_core::domain::value::PersonaId::from_uuid(persona);

    // ---- noise floor: raw batched rows, no claims ------------------
    let asset_count = args.assets;
    let seeded_assets: Vec<Uuid> = isle
        .call(move |conn| {
            let tx = conn.transaction()?;
            let mut ids = Vec::with_capacity(asset_count as usize);
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                        modality, occurred_at, created_at, updated_at) \
                     VALUES (?1, ?2, 'fs', ?3, 'image', 0, ?4, ?4)",
                )?;
                for i in 0..asset_count {
                    let id = Uuid::now_v7();
                    stmt.execute(params![id, persona, format!("bench-{id}.png"), i as i64])?;
                    ids.push(id);
                }
            }
            tx.commit()?;
            Ok(ids)
        })
        .await?;

    // ---- pursuit fixtures through the ports ------------------------
    let pursuits = SqlitePursuitRepository::new(isle.clone());
    let dispatches = SqliteDispatchRepository::new(isle.clone());
    let snapshots = SqliteSnapshotRepository::new(isle.clone());

    let member = AssetId::from_uuid(seeded_assets[0]);
    let snapshot = snapshots
        .create_or_reuse(&Snapshot::new(persona_id, vec![member], now)?)
        .await?;

    let mut pursuit_ids: Vec<PursuitId> = Vec::with_capacity(args.pursuits as usize);
    let mut cursor = 0usize;
    for _ in 0..args.pursuits {
        let pursuit = Pursuit::new(persona_id, None, None, None, None, now, &ctx);
        pursuits.create(&pursuit).await?;

        let mut round_ids = Vec::with_capacity(args.rounds_per as usize);
        for _ in 0..args.rounds_per {
            let mut job = DispatchJob::new(
                snapshot.id,
                persona_id,
                "file",
                "write",
                serde_json::json!({}),
                now,
                &ctx,
            )?;
            job.pursuit_id = Some(pursuit.id);
            dispatches.save(&job).await?;
            round_ids.push(job.id);
        }

        // Returns: `_trace` notes over slices of the noise floor —
        // resolved dispatch claims spread across the rounds, plus
        // three direct claims.
        let joined: Vec<Uuid> = seeded_assets[cursor..cursor + args.returns_per as usize].to_vec();
        cursor += args.returns_per as usize;
        let direct: Vec<Uuid> = seeded_assets[cursor..cursor + 3].to_vec();
        cursor += 3;
        let rounds_for_update: Vec<String> = round_ids.iter().map(|d| d.to_string()).collect();
        let pursuit_str = pursuit.id.to_string();
        isle.call(move |conn| {
            let tx = conn.transaction()?;
            {
                let mut stmt = tx.prepare("UPDATE asset SET extra = ?1 WHERE id = ?2")?;
                for (i, id) in joined.iter().enumerate() {
                    let dispatch = &rounds_for_update[i % rounds_for_update.len()];
                    stmt.execute(params![
                        format!(r#"{{"_trace":{{"resolved":true,"dispatch_id":"{dispatch}"}}}}"#),
                        id
                    ])?;
                }
                for id in &direct {
                    stmt.execute(params![
                        format!(
                            r#"{{"_trace":{{"resolved":false,"pursuit_resolved":true,"pursuit_id":"{pursuit_str}"}}}}"#
                        ),
                        id
                    ])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await?;

        for kind in [
            PursuitEventKind::ClosedSatisfied,
            PursuitEventKind::Reopened,
        ] {
            pursuits
                .append_event(&PursuitEvent::new(
                    pursuit.id, persona_id, kind, None, None, now, &ctx,
                )?)
                .await?;
        }
        pursuit_ids.push(pursuit.id);
    }

    // ---- measurements ----------------------------------------------
    let sample_every = (pursuit_ids.len() / 50).max(1);
    let sampled: Vec<PursuitId> = pursuit_ids.iter().step_by(sample_every).copied().collect();

    let mut returns_samples = Vec::new();
    let mut view_samples = Vec::new();
    for id in &sampled {
        let t = Instant::now();
        let returns = pursuits.returns_of(id).await?;
        returns_samples.push(t.elapsed().as_micros());
        anyhow::ensure!(
            returns.len() as u64 == args.returns_per + 3,
            "returns probe answered {} of {} expected members — the fixture or the \
             probe is wrong and the timing below would be a number about nothing",
            returns.len(),
            args.returns_per + 3
        );

        // The view as the service composes it: row + events + rounds
        // + returns.
        let t = Instant::now();
        let _row = pursuits.find(id).await?.expect("seeded pursuit");
        let _events = pursuits.events_of(id).await?;
        let _rounds = dispatches.list_rounds(id).await?;
        let _returns = pursuits.returns_of(id).await?;
        view_samples.push(t.elapsed().as_micros());
    }

    let mut list_samples = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        let listed = pursuits.list(&persona_id, 50).await?;
        let standings = pursuits.latest_event_kinds(&persona_id).await?;
        list_samples.push(t.elapsed().as_micros());
        anyhow::ensure!(listed.len() == 50 && !standings.is_empty(), "list fixture");
    }

    let result = PursuitBenchResult {
        schema: SCHEMA_PURSUIT,
        at: now.to_rfc3339(),
        assets: args.assets,
        pursuits: args.pursuits,
        rounds_per: args.rounds_per,
        returns_per: args.returns_per,
        returns_of: stats(returns_samples),
        view_composite: stats(view_samples),
        list_standings: stats(list_samples),
    };

    let out_dir = args
        .out
        .unwrap_or_else(|| PathBuf::from(crate::measure::DEFAULT_OUT_DIR));
    std::fs::create_dir_all(&out_dir).context("create out dir")?;
    let out_path = out_dir.join(format!("pursuit-view-{}.json", now.format("%Y%m%d-%H%M%S")));
    std::fs::write(&out_path, serde_json::to_vec_pretty(&result)?)
        .with_context(|| format!("write {}", out_path.display()))?;

    println!(
        "measure-pursuit @ {} assets / {} pursuits ({} rounds, {} joined + 3 direct returns each)",
        result.assets, result.pursuits, result.rounds_per, result.returns_per
    );
    println!(
        "  returns_of       p50 {:>6} us  p95 {:>6} us  max {:>6} us",
        result.returns_of.p50_us, result.returns_of.p95_us, result.returns_of.max_us
    );
    println!(
        "  view (composite) p50 {:>6} us  p95 {:>6} us  max {:>6} us",
        result.view_composite.p50_us, result.view_composite.p95_us, result.view_composite.max_us
    );
    println!(
        "  list+standings   p50 {:>6} us  p95 {:>6} us  max {:>6} us",
        result.list_standings.p50_us, result.list_standings.p95_us, result.list_standings.max_us
    );
    println!("  result: {}", out_path.display());

    drop(pursuits);
    drop(dispatches);
    drop(snapshots);
    driver.shutdown().await.ok();
    Ok(())
}
