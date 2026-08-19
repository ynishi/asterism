//! `measure-pursuit` — the pursuit-read micro-bench (#29).
//!
//! Answers one question with numbers instead of assumptions: **do the
//! pursuit reads stay flat when the library holds the documented
//! 100k+ assets?** The reads under test are the ones the pursuit view
//! composes — `find` + `events_of` + `txs_of` — plus the one-window
//! standing read. The design bets on index seeks over the pursuit
//! tables' own indexes; this command is the bet's receipt, and the
//! number that decides whether a materialised projection (job-built)
//! is ever needed.
//!
//! Unlike `measure-import` / `measure-cold`, nothing here observes a
//! server: the reads have no transport yet (that is a later slice),
//! and what needs measuring is the storage access path, not HTTP. The
//! command is therefore fully self-contained — it seeds a throwaway
//! profile in a temp directory and measures through the same
//! repository adapters the application wires, so the numbers are the
//! adapters' numbers, not a lookalike query's. The seeding cuts one
//! corner knowingly: the 100k-row noise floor is raw batched SQL
//! (volume, not schema fidelity — the noise rows only need to exist),
//! while everything with a domain constructor (pursuits, events) goes
//! through the ports.
//!
//! The run is warm by construction (the process that seeded the pages
//! measures them); a cold-cache number would need a separate process
//! and is not what the index-vs-scan question turns on.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::forge::pursuit::{Pursuit, PursuitEvent, PursuitEventKind};
use asterism_core::domain::forge::repository::PursuitRepository;
use asterism_core::domain::forge::value::PursuitId;
use asterism_infra::sqlite;
use asterism_infra::sqlite::repo::SqlitePursuitRepository;
use chrono::Utc;
use clap::Args;
use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

/// Result schema of `measure-pursuit`.
///
/// `v2` because the shape changed rather than the measurement drifted:
/// the returns lane is gone from the codebase, so its timing and the
/// two fixture knobs that fed it are absent from the result. A reader
/// holding a `v1` file is holding numbers about reads that no longer
/// exist, and the tag is how it can tell.
pub const SCHEMA_PURSUIT: &str = "bench-pursuit-view-v2";

#[derive(Debug, Args)]
pub struct MeasurePursuitArgs {
    /// Noise floor: plain asset rows the pursuit reads must stay flat
    /// against. The default is the documented steady-state scale.
    #[arg(long, default_value_t = 100_000)]
    pub assets: u64,
    /// Pursuits seeded with events.
    #[arg(long, default_value_t = 200)]
    pub pursuits: u64,
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
    anyhow::ensure!(
        args.assets >= 1,
        "--assets {} seeds no noise floor for the reads to stay flat against",
        args.assets
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
    isle.call(move |conn| {
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', ?3, 'image', 0, ?4, ?4)",
            )?;
            for i in 0..asset_count {
                let id = Uuid::now_v7();
                stmt.execute(params![id, persona, format!("bench-{id}.png"), i as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await?;

    // ---- pursuit fixtures through the ports ------------------------
    let pursuits = SqlitePursuitRepository::new(isle.clone());

    let mut pursuit_ids: Vec<PursuitId> = Vec::with_capacity(args.pursuits as usize);
    for _ in 0..args.pursuits {
        let pursuit = Pursuit::new(persona_id, None, None, None, None, now, &ctx);
        pursuits.create(&pursuit).await?;

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

    let mut view_samples = Vec::new();
    for id in &sampled {
        // The reads a pursuit view costs, measured one port at a time.
        let t = Instant::now();
        let _row = pursuits.find(id).await?.expect("seeded pursuit");
        let _events = pursuits.events_of(id).await?;
        let _txs = pursuits.txs_of(id).await?;
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
        "measure-pursuit @ {} assets / {} pursuits",
        result.assets, result.pursuits
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
    driver.shutdown().await.ok();
    Ok(())
}
