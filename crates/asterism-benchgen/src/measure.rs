//! `measure-import` / `measure-cold` — the two non-scroll measurements
//! of the bench driver.
//!
//! Both commands *observe* a running bench server and write one result
//! file. Neither of them puts the machine into the state it measures:
//!
//! - `measure-import` does not reset the database. A load into a
//!   profile that already holds rows is not an import measurement, and
//!   a subcommand that quietly deletes a profile to make its own number
//!   valid is the wrong tool to hand a hurried operator. The recipe doc
//!   says `just bench-reset` first; the load's own rejection count says
//!   so afterwards.
//! - `measure-cold` does not restart the server. "Cold" is a property
//!   of the process that answers, so the only honest way to produce it
//!   is for the operator to restart the backend and then run this.
//!
//! What they do share with `load-file` is the port guard: 8989 is the
//! Dogfood app and never a bench target.
//!
//! ## Why the import measurement is two numbers
//!
//! `add-batch` returns when the rows exist, and that is not when the
//! import is over: every asset enqueues a `MaterialHash` (reads every
//! byte) and `ThumbGen` jobs, and those are what the reference workload —
//! 5,000 files, about 20 minutes — is actually measuring. So the result
//! carries `registration` (the load's own passes) and `drain` (polling
//! `GET /asterism/jobs/depth` until nothing is pending or running),
//! with a memory sample per poll so item 5 (resident memory) comes out
//! of the same run rather than a second one.
//!
//! That sample is two readings, not one. `ps` RSS alone cannot answer
//! item 5 on macOS: it counts file-backed pages the kernel is free to
//! drop, so a run that maps a large file reads as a run that consumed
//! it. The first S run made that concrete — a 7,103 MB RSS peak against
//! a 7.43 GiB corpus, close enough to the corpus size that the figure
//! had to be split before it could be called anything. Each poll
//! therefore also takes `footprint(1)`, whose `phys_footprint` is what
//! the kernel charges the process and whose dirty/clean split says
//! which of the two a peak was ([`Footprint`]).

use anyhow::{Context, Result, anyhow, bail};
use asterism_contract::dto::{AssetIndexPageDto, PerfDto, PersonaDto};
use asterism_infra::jobs::JobsDepth;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::load_file::{Api, LoadFileArgs, LoadReport, guard_server, verify_corpus};
use crate::manifest::Manifest;
use crate::seed_meta::persona_name;

/// Result schema of `measure-import`.
///
/// v2 adds the `footprint` side of each memory sample. The v1 numbers
/// are not comparable to v2's: v1 published `ps` RSS alone, which on
/// macOS is not the figure that answers "how much memory is this
/// costing" (see [`Footprint`]).
pub const SCHEMA_IMPORT: &str = "bench-import-v2";
/// Result schema of `measure-cold`.
pub const SCHEMA_COLD: &str = "bench-cold-v1";
/// Default result directory (gitignored, `workspace/`).
pub const DEFAULT_OUT_DIR: &str = "workspace/bench-results";
/// Drain poll cadence. Cheap enough at this interval precisely because
/// `/asterism/jobs/depth` skips the per-kind `json_extract` pass.
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Upper bound on the drain wait. Reaching it is recorded in the result
/// (`timed_out`) rather than raised as an error: a drain that did not
/// finish in ninety minutes is a finding, and the timeline collected up
/// to that point is the evidence for it.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(90 * 60);
/// Cadence of the stderr progress line.
pub const PROGRESS_EVERY: Duration = Duration::from_secs(30);
/// Process names the RSS sampler keeps. Case-sensitive, which is also
/// what keeps the driver itself (`asterism-benchgen`) out of its own
/// measurement.
pub const RSS_PROCESS_MATCHES: [&str; 3] = ["Asterism", "asterism-ui", "asterism-server"];

// ---------------------------------------------------------------- import

/// Everything `measure-import` needs; the CLI layer builds it.
#[derive(Debug, Clone)]
pub struct MeasureImportArgs {
    pub seed: u64,
    pub preset: &'static str,
    pub count: u64,
    pub corpus_dir: PathBuf,
    pub server: String,
    pub allow_any_server: bool,
    pub out_dir: PathBuf,
}

/// The corpus this run was measured against — the identity every
/// published number has to carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusIdentity {
    pub seed: u64,
    pub preset: String,
    pub generator_version: String,
    pub count: u64,
}

impl CorpusIdentity {
    pub fn of(manifest: &Manifest, count: u64) -> Self {
        Self {
            seed: manifest.seed,
            preset: manifest.preset.clone(),
            generator_version: manifest.generator_version.clone(),
            count,
        }
    }
}

/// The `footprint(1)` side of one memory sample.
///
/// `ps` RSS counts every resident page, including file-backed ones: a
/// mapped file that the kernel can drop under pressure lands in RSS at
/// full size and reads as memory the process is costing. The first S
/// run measured a 7,103 MB RSS peak against a 7.43 GiB corpus — close
/// enough to the corpus size that the figure had to be split before it
/// could be called a leak. `phys_footprint` is what Activity Monitor
/// shows and what the kernel charges to the process; `dirty` vs
/// `clean` says whether the pages are ours or the file cache's; and
/// `mapped_file` names the file-backed part outright.
///
/// `peak` matters because the poll is every five seconds: a spike
/// between two polls is invisible to the sampled series but not to the
/// kernel's own high-water mark.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Footprint {
    pub phys_footprint_kb: u64,
    pub phys_footprint_peak_kb: u64,
    /// `TOTAL` row: pages that cannot be reclaimed without swapping.
    pub dirty_kb: u64,
    /// `TOTAL` row: pages the kernel can drop and re-read from disk.
    pub clean_kb: u64,
    /// The `mapped file` category, dirty + clean.
    pub mapped_file_kb: u64,
}

/// One process' memory at one poll, from both `ps` and `footprint`.
///
/// `footprint` is optional because losing it must not lose the drain
/// timing with it — same trade as the sampler as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemSample {
    pub pid: i32,
    pub rss_kb: u64,
    pub comm: String,
    pub footprint: Option<Footprint>,
}

/// One drain poll: queue depth plus what the app was holding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainTick {
    /// Milliseconds since the drain wait started.
    pub t_ms: u64,
    pub pending: u64,
    pub running: u64,
    pub done: u64,
    pub failed: u64,
    /// Renamed from v1's `rss`: the sample is no longer RSS alone.
    pub mem: Vec<MemSample>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainReport {
    pub started_at_ms: i64,
    /// `None` when the wait hit [`DRAIN_TIMEOUT`].
    pub drained_at_ms: Option<i64>,
    pub elapsed_ms: u64,
    pub timed_out: bool,
    pub timeline: Vec<DrainTick>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportResult {
    pub schema: String,
    pub manifest: CorpusIdentity,
    pub server: String,
    pub registration: LoadReport,
    pub drain: DrainReport,
}

/// Runs the whole import measurement: load, then wait for the queue.
pub async fn run_import(args: MeasureImportArgs) -> Result<PathBuf> {
    ensure_file_tier(args.preset)?;
    guard_server(&args.server, args.allow_any_server)?;
    let load_args = LoadFileArgs {
        seed: args.seed,
        preset: args.preset,
        count: args.count,
        corpus_dir: args.corpus_dir.clone(),
        server: args.server.clone(),
        allow_any_server: args.allow_any_server,
    };
    // Read first, so a corpus mismatch is refused before anything is
    // posted — and so the result file can name the corpus even though
    // `load_file::run` verifies it again on its own way in.
    let manifest = verify_corpus(&load_args)?;

    let api = Api::new(&args.server);
    let registration = crate::load_file::run(load_args).await?;
    eprintln!("{}", crate::load_file::report_line(&registration));

    let drain = drain_wait(&api).await?;
    let result = ImportResult {
        schema: SCHEMA_IMPORT.to_string(),
        manifest: CorpusIdentity::of(&manifest, args.count),
        server: args.server.clone(),
        registration,
        drain,
    };
    let path = write_result(&args.out_dir, &format!("import-{}", args.preset), &result)?;
    eprintln!(
        "benchgen: registration {} ms / drain {} ms{} → {}",
        result.registration.elapsed_ms,
        result.drain.elapsed_ms,
        if result.drain.timed_out {
            " (TIMED OUT — the queue was still working)"
        } else {
            ""
        },
        path.display()
    );
    Ok(path)
}

/// Whether the queue owes any more work.
///
/// `done` and `failed` are terminal, so they do not hold the wait open;
/// a failed job is a finding for the result file to carry, not a reason
/// to poll for ninety minutes. Split out as a function because it is
/// the loop's only exit condition and a bench that never terminates and
/// a bench that terminates instantly are the same defect.
pub fn drained(depth: &JobsDepth) -> bool {
    depth.pending + depth.running == 0
}

async fn drain_wait(api: &Api) -> Result<DrainReport> {
    let started = Instant::now();
    let started_at_ms = Utc::now().timestamp_millis();
    let mut timeline: Vec<DrainTick> = Vec::new();
    let mut last_progress = Instant::now();
    let mut warned_ps = false;
    let mut warned_footprint = false;

    eprintln!("benchgen: waiting for the job queue to drain (poll 5 s, cap 90 min)");
    let (drained_at_ms, timed_out) = loop {
        let depth: JobsDepth = api
            .get("/asterism/jobs/depth")
            .await
            .context("cannot read the job queue depth — is the bench server running?")?;
        timeline.push(DrainTick {
            t_ms: started.elapsed().as_millis() as u64,
            pending: depth.pending,
            running: depth.running,
            done: depth.done,
            failed: depth.failed,
            mem: sample_memory(&mut warned_ps, &mut warned_footprint),
        });
        if drained(&depth) {
            break (Some(Utc::now().timestamp_millis()), false);
        }
        if started.elapsed() >= DRAIN_TIMEOUT {
            eprintln!(
                "benchgen: WARNING drain timeout after {:.0}s with pending={} running={}",
                started.elapsed().as_secs_f64(),
                depth.pending,
                depth.running
            );
            break (None, true);
        }
        if last_progress.elapsed() >= PROGRESS_EVERY {
            eprintln!(
                "benchgen: drain {:.0}s pending={} running={} done={} failed={}",
                started.elapsed().as_secs_f64(),
                depth.pending,
                depth.running,
                depth.done,
                depth.failed
            );
            last_progress = Instant::now();
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    };

    Ok(DrainReport {
        started_at_ms,
        drained_at_ms,
        elapsed_ms: started.elapsed().as_millis() as u64,
        timed_out,
        timeline,
    })
}

/// Memory of the app processes: `ps` finds them, `footprint` explains
/// them.
///
/// A failure on either side degrades rather than ending the run: the
/// memory figure is one of six measurements, and losing the drain
/// timing because a tool was unavailable would be the wrong trade. A
/// `ps` failure yields an empty sample; a `footprint` failure yields
/// samples whose `footprint` is `None`, which is distinct from a
/// measured zero. Each failure is reported once — a warning per
/// five-second poll would bury the progress lines.
fn sample_memory(warned_ps: &mut bool, warned_footprint: &mut bool) -> Vec<MemSample> {
    let mut samples = match std::process::Command::new("ps")
        .args(["-axo", "pid,rss,comm"])
        .output()
    {
        Ok(out) if out.status.success() => parse_ps(&String::from_utf8_lossy(&out.stdout)),
        other => {
            if !*warned_ps {
                *warned_ps = true;
                let why = match other {
                    Ok(out) => format!("exit {}", out.status),
                    Err(err) => err.to_string(),
                };
                eprintln!(
                    "benchgen: WARNING RSS sampling unavailable ({why}); timeline mem is empty"
                );
            }
            Vec::new()
        }
    };
    for sample in &mut samples {
        sample.footprint = sample_footprint(sample.pid, warned_footprint);
    }
    samples
}

/// One process' `footprint(1)` reading, or `None` if it could not be
/// taken.
///
/// Measured at 0.22 s against a 2.9 GB process, which is why it is
/// affordable once per process per five-second poll.
fn sample_footprint(pid: i32, warned: &mut bool) -> Option<Footprint> {
    match std::process::Command::new("/usr/bin/footprint")
        .args(["-p", &pid.to_string()])
        .output()
    {
        Ok(out) if out.status.success() => {
            let parsed = parse_footprint(&String::from_utf8_lossy(&out.stdout));
            if parsed.is_none() && !*warned {
                *warned = true;
                eprintln!(
                    "benchgen: WARNING footprint output not understood; timeline carries RSS only"
                );
            }
            parsed
        }
        other => {
            if !*warned {
                *warned = true;
                let why = match other {
                    Ok(out) => format!("exit {}", out.status),
                    Err(err) => err.to_string(),
                };
                eprintln!(
                    "benchgen: WARNING footprint unavailable ({why}); timeline carries RSS only"
                );
            }
            None
        }
    }
}

/// Extracts the app processes out of `ps -axo pid,rss,comm` output.
///
/// A pure function over the text because the parsing is where this can
/// go quietly wrong: the header line, a command path containing spaces,
/// and a `comm` that is a full path rather than a bare name all produce
/// an empty result instead of an error if handled carelessly, and an
/// empty result reads as "the app used no memory".
pub fn parse_ps(output: &str) -> Vec<MemSample> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, rest) = line.split_once(char::is_whitespace)?;
            let pid: i32 = pid.parse().ok()?;
            let rest = rest.trim_start();
            let (rss, comm) = rest.split_once(char::is_whitespace)?;
            let rss_kb: u64 = rss.parse().ok()?;
            let comm = comm.trim();
            RSS_PROCESS_MATCHES
                .iter()
                .any(|needle| comm.contains(needle))
                .then(|| MemSample {
                    pid,
                    rss_kb,
                    comm: comm.to_string(),
                    footprint: None,
                })
        })
        .collect()
}

/// Extracts the published figures out of `footprint -p <pid>` output.
///
/// Pure function over the text for the same reason [`parse_ps`] is:
/// every failure mode here is silent. `footprint` mixes units inside
/// one table (`0 B`, `752 KB`, `2903 MB`), puts the category name last
/// on a row whose own name contains a space (`mapped file`), and
/// reports the two headline numbers in a trailing `Auxiliary data:`
/// block rather than in the table at all.
///
/// `None` when `phys_footprint` is absent: a parse miss that returned
/// a `Footprint::default()` would publish as "this process used no
/// memory", which is exactly the reading the whole change exists to
/// prevent.
pub fn parse_footprint(output: &str) -> Option<Footprint> {
    let mut fp = Footprint::default();
    let mut seen_phys = false;

    for line in output.lines() {
        let line = line.trim();
        // Peak first: the plain key is a prefix of the peak one.
        if let Some(rest) = line.strip_prefix("phys_footprint_peak:") {
            if let Some(kb) = sizes_kb(rest).first() {
                fp.phys_footprint_peak_kb = *kb;
            }
        } else if let Some(rest) = line.strip_prefix("phys_footprint:") {
            if let Some(kb) = sizes_kb(rest).first() {
                fp.phys_footprint_kb = *kb;
                seen_phys = true;
            }
        } else if let Some(cols) = category_row(line, "TOTAL") {
            fp.dirty_kb = cols.first().copied().unwrap_or(0);
            fp.clean_kb = cols.get(1).copied().unwrap_or(0);
        } else if let Some(cols) = category_row(line, "mapped file") {
            // Dirty + clean: the question this answers is how much of
            // the resident total is a file rather than ours.
            fp.mapped_file_kb =
                cols.first().copied().unwrap_or(0) + cols.get(1).copied().unwrap_or(0);
        }
    }

    seen_phys.then_some(fp)
}

/// The size columns of a `footprint` table row, when the row is the
/// named category. `None` for every other row, so that a category
/// whose name merely contains another's cannot be read as it.
fn category_row(line: &str, category: &str) -> Option<Vec<u64>> {
    let head = line.strip_suffix(category)?;
    // The region count trails the sizes with no unit after it, so the
    // pair walk in `sizes_kb` stops before consuming it.
    Some(sizes_kb(head))
}

/// Every `<number> <unit>` pair in the text, normalised to KB.
///
/// Tokens that are not part of a pair are skipped rather than treated
/// as a size, which is what keeps the trailing region count out of the
/// result.
fn sizes_kb(text: &str) -> Vec<u64> {
    let toks: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < toks.len() {
        match (toks[i].parse::<f64>(), unit_to_kb(toks[i + 1])) {
            (Ok(value), Some(per_unit)) if value >= 0.0 => {
                out.push((value * per_unit) as u64);
                i += 2;
            }
            _ => i += 1,
        }
    }
    out
}

/// KB per unit, or `None` when the token is not a unit at all.
fn unit_to_kb(unit: &str) -> Option<f64> {
    match unit {
        "B" => Some(1.0 / 1024.0),
        "KB" => Some(1.0),
        "MB" => Some(1024.0),
        "GB" => Some(1024.0 * 1024.0),
        "TB" => Some(1024.0 * 1024.0 * 1024.0),
        _ => None,
    }
}

// ------------------------------------------------------------------ cold

/// Everything `measure-cold` needs.
#[derive(Debug, Clone)]
pub struct MeasureColdArgs {
    pub server: String,
    pub allow_any_server: bool,
    /// Persona display name. Resolved against the server's own list, so
    /// a typo names the personas that do exist instead of silently
    /// measuring the whole library.
    pub persona: String,
    pub limit: u64,
    pub out_dir: PathBuf,
}

/// The server-side breakdown of one `list_index`, lifted out of
/// `GET /asterism/perf`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerfSample {
    pub occurred_at_ms: i64,
    pub duration_ms: i64,
    pub rows: Option<u64>,
    pub query_ms: Option<u64>,
    pub group_map_ms: Option<u64>,
    pub count_ms: Option<u64>,
}

/// One timed call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProbe {
    /// Wall clock measured by this process — the number the operator
    /// experiences, transport and JSON decode included.
    pub wall_ms: u64,
    pub items: usize,
    pub total: Option<u64>,
    /// `None` when the server persisted no perf record; see
    /// [`ColdResult::perf_note`].
    pub perf: Option<PerfSample>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColdResult {
    pub schema: String,
    pub server: String,
    pub server_git_sha: Option<String>,
    pub persona: String,
    pub persona_id: String,
    pub limit: u64,
    pub cold: ListProbe,
    pub warm: ListProbe,
    /// Set when neither probe found a server-side breakdown, with the
    /// reason. Silence here would read as "the query took no time".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perf_note: Option<String>,
}

/// The listing the desktop grid performs, as a query string.
///
/// Same shape as `App.svelte`'s `currentFilter()`: the live side of the
/// trash, one persona, no facet, from the top, `limit = 200_000`. The
/// fields that builder sends as `null` are omitted rather than spelled
/// out — `ListAssetsQuery` is `#[serde(default)]`, so an absent key and
/// a null are the same request, and `serde_urlencoded` cannot express a
/// null anyway.
///
/// Persona ids are UUIDs, so they need no escaping here.
pub fn index_query(persona_id: &str, limit: u64) -> String {
    format!("/asterism/assets/index?persona_id={persona_id}&trash=live&offset=0&limit={limit}")
}

/// Newest `list_index` database-phase record in a perf listing.
///
/// The listing is newest-first and each call writes two records — the
/// database phase (which carries the breakdown) and the domain mapping
/// (which does not). Picking the newest record outright would take the
/// mapping one about half the time and report a `query_ms` of nothing.
pub fn newest_db_phase(rows: &[PerfDto]) -> Option<PerfSample> {
    rows.iter().find_map(|row| {
        let attrs: serde_json::Value = serde_json::from_str(row.attrs_json.as_deref()?).ok()?;
        if attrs.get("phase").and_then(serde_json::Value::as_str) != Some("database") {
            return None;
        }
        let num = |key: &str| attrs.get(key).and_then(serde_json::Value::as_u64);
        Some(PerfSample {
            occurred_at_ms: row.occurred_at_ms,
            duration_ms: row.duration_ms,
            rows: num("rows"),
            query_ms: num("query_ms"),
            group_map_ms: num("group_map_ms"),
            count_ms: num("count_ms"),
        })
    })
}

/// Cold then warm, one call each.
pub async fn run_cold(args: MeasureColdArgs) -> Result<PathBuf> {
    guard_server(&args.server, args.allow_any_server)?;
    let api = Api::new(&args.server);

    let health: serde_json::Value = api
        .get("/asterism/health")
        .await
        .context("cannot reach the bench server — start it with `just bench-headless`")?;
    let git_sha = health
        .get("git_sha")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    let personas: Vec<PersonaDto> = api.get("/asterism/personas").await?;
    let persona = personas
        .iter()
        .find(|p| p.name == args.persona)
        .ok_or_else(|| {
            let names: Vec<&str> = personas.iter().map(|p| p.name.as_str()).collect();
            anyhow!(
                "no persona named {:?} on {} — the server holds {:?}. Seed the corpus first \
                 (just bench-seed-l / just bench-load)",
                args.persona,
                args.server,
                names
            )
        })?;

    eprintln!(
        "benchgen: measure-cold persona={} ({}) limit={} server={}",
        persona.name, persona.id, args.limit, args.server
    );
    let cold = probe_index(&api, &persona.id, args.limit).await?;
    eprintln!("benchgen: cold {} ms ({} items)", cold.wall_ms, cold.items);
    let warm = probe_index(&api, &persona.id, args.limit).await?;
    eprintln!("benchgen: warm {} ms ({} items)", warm.wall_ms, warm.items);

    // The perf stream is development-only (`StreamPolicy::dev_only`,
    // `Env::is_dev`), and the bench profile is not `dev` — so under
    // `just bench-headless` the server-side breakdown is genuinely
    // absent rather than missing by accident. Saying so in the file is
    // the difference between "no breakdown" and "a breakdown of zero".
    let perf_note = (cold.perf.is_none() && warm.perf.is_none()).then(|| {
        "no perf record: GET /asterism/perf persists only under the dev profile \
         (Stream::Perf is dev_only), so a bench-profile server records none. \
         wall_ms is the whole measurement here."
            .to_string()
    });
    if let Some(note) = &perf_note {
        eprintln!("benchgen: WARNING {note}");
    }

    let result = ColdResult {
        schema: SCHEMA_COLD.to_string(),
        server: args.server.clone(),
        server_git_sha: git_sha,
        persona: persona.name.clone(),
        persona_id: persona.id.clone(),
        limit: args.limit,
        cold,
        warm,
        perf_note,
    };
    let path = write_result(&args.out_dir, "cold", &result)?;
    eprintln!("benchgen: wrote {}", path.display());
    Ok(path)
}

async fn probe_index(api: &Api, persona_id: &str, limit: u64) -> Result<ListProbe> {
    // Bound the perf lookup to this call: the endpoint is newest-first,
    // but a previous run's record would otherwise be a valid answer.
    let since_ms = Utc::now().timestamp_millis();
    let started = Instant::now();
    let page: AssetIndexPageDto = api.get(&index_query(persona_id, limit)).await?;
    let wall_ms = started.elapsed().as_millis() as u64;
    let perf: Vec<PerfDto> = api
        .get(&format!(
            "/asterism/perf?op=list_index&since_ms={since_ms}&limit=32"
        ))
        .await?;
    Ok(ListProbe {
        wall_ms,
        items: page.items.len(),
        total: page.total,
        perf: newest_db_phase(&perf),
    })
}

// ----------------------------------------------------------------- shared

/// Result file name: UTC stamp first, so a directory listing is a
/// chronology.
pub fn result_name(stamp: &str, kind: &str) -> String {
    format!("{stamp}-{kind}.json")
}

fn write_result<T: Serialize>(out_dir: &Path, kind: &str, value: &T) -> Result<PathBuf> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("cannot create {}", out_dir.display()))?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let path = out_dir.join(result_name(&stamp, kind));
    let json = serde_json::to_string_pretty(value).context("cannot serialise the result")?;
    std::fs::write(&path, json).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

/// Default persona for `measure-cold` — the first of the six the
/// cardinality model produces, and the one that owns every group.
pub fn default_persona() -> String {
    persona_name(0)
}

/// Refuses a preset that has no files to import.
pub fn ensure_file_tier(preset: &str) -> Result<()> {
    if preset == "l" {
        bail!("the l preset writes no files — there is no import to measure; use `seed-meta`");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perf_row(attrs: &str, duration_ms: i64) -> PerfDto {
        PerfDto {
            id: "00000000-0000-0000-0000-000000000000".into(),
            occurred_at_ms: 1_700_000_000_000,
            env: "bench".into(),
            event: "perf.list_index".into(),
            op: "list_index".into(),
            duration_ms,
            attrs_json: Some(attrs.to_string()),
            correlation_id: None,
        }
    }

    #[test]
    fn drain_ends_only_when_nothing_is_pending_or_running() {
        assert!(drained(&JobsDepth::default()));
        // Terminal states do not hold the wait open — a failed job is
        // for the result file to carry, not something to wait out.
        assert!(drained(&JobsDepth {
            done: 5_000,
            failed: 3,
            ..JobsDepth::default()
        }));
        assert!(!drained(&JobsDepth {
            pending: 1,
            ..JobsDepth::default()
        }));
        assert!(!drained(&JobsDepth {
            running: 1,
            done: 4_999,
            ..JobsDepth::default()
        }));
    }

    #[test]
    fn ps_output_yields_the_app_processes_only() {
        // Real shape: right-aligned columns, a header, `comm` as a full
        // path that contains spaces, and plenty of unrelated processes.
        let out = "  PID    RSS COMM\n\
             \x20  1   9384 /sbin/launchd\n\
             \x20812 512340 /Applications/Asterism.app/Contents/MacOS/Asterism\n\
             \x20813  40120 /Users/x/album/target/debug/asterism-server\n\
             \x20814  30000 /Users/x/My Apps/asterism-ui\n\
             \x20999  10000 /usr/bin/ssh\n";
        let samples = parse_ps(out);
        assert_eq!(
            samples.iter().map(|s| s.pid).collect::<Vec<_>>(),
            vec![812, 813, 814]
        );
        assert_eq!(samples[0].rss_kb, 512_340);
        // A path with a space is kept whole; truncating it at the first
        // space would drop the name the filter matched on.
        assert_eq!(samples[2].comm, "/Users/x/My Apps/asterism-ui");
        // `ps` carries no footprint; the sampler fills it per pid.
        assert!(samples.iter().all(|s| s.footprint.is_none()));
        // The header is not a process, and neither is the driver.
        assert!(parse_ps("  PID    RSS COMM\n").is_empty());
        assert!(parse_ps("  777  20000 target/release/asterism-benchgen\n").is_empty());
        assert!(parse_ps("").is_empty());
    }

    /// The shape of a real `footprint -p` dump, with the numbers set to
    /// the case the change exists for: a resident total dominated by a
    /// mapped file, against a much smaller charge to the process.
    const FOOTPRINT_OUT: &str = "\
======================================================================\n\
Asterism [812]: 64-bit    Footprint: 6939 MB (16384 bytes per page)\n\
======================================================================\n\
\n\
  Dirty      Clean  Reclaimable    Regions    Category\n\
    ---        ---          ---        ---    ---\n\
 752 KB        0 B          0 B          1    MALLOC_NANO\n\
 6.2 GB      18 MB          0 B        420    mapped file\n\
    0 B     528 KB          0 B         49    __TEXT\n\
    ---        ---          ---        ---    ---\n\
 480 MB     6.6 GB          0 B        383    TOTAL\n\
\n\
Auxiliary data:\n\
    phys_footprint: 491520 KB\n\
    phys_footprint_peak: 7273472 KB\n";

    #[test]
    fn footprint_output_separates_the_charge_from_the_file_cache() {
        let fp = parse_footprint(FOOTPRINT_OUT).expect("the headline number is there");

        // The two headline numbers differ, so a prefix match that let
        // `phys_footprint_peak:` fall through to the `phys_footprint:`
        // arm would be visible here rather than agreeing by accident.
        assert_eq!(fp.phys_footprint_kb, 491_520);
        assert_eq!(fp.phys_footprint_peak_kb, 7_273_472);

        // Units are mixed within one table, so each row is normalised
        // rather than read as a bare number.
        assert_eq!(fp.dirty_kb, 480 * 1024);
        assert_eq!(fp.clean_kb, (6.6 * 1024.0 * 1024.0) as u64);
        // `mapped file` is dirty + clean: 6.2 GB + 18 MB.
        assert_eq!(
            fp.mapped_file_kb,
            (6.2 * 1024.0 * 1024.0) as u64 + 18 * 1024
        );

        // The region count trails the sizes with no unit of its own; if
        // it were read as a size, `dirty_kb` would be 383 here.
        assert_ne!(fp.dirty_kb, 383);
        // `MALLOC_NANO` is a row we do not publish, and its 752 KB must
        // not leak into a row we do.
        assert_ne!(fp.dirty_kb, 752);
    }

    #[test]
    fn footprint_without_the_headline_is_not_a_zeroed_reading() {
        // A dump cut short before `Auxiliary data:` still has a full
        // table. Publishing that as `Footprint::default()` would read
        // as "this process used no memory" — the exact misreading the
        // split was made to prevent.
        let truncated = FOOTPRINT_OUT
            .split("Auxiliary data:")
            .next()
            .expect("the table half");
        assert!(parse_footprint(truncated).is_none());
        assert!(parse_footprint("").is_none());
        assert!(parse_footprint("footprint: no such process 99999\n").is_none());
    }

    #[test]
    fn the_database_phase_is_the_record_with_the_breakdown() {
        // Newest first, and the newest record is the mapping phase —
        // the one that carries no `query_ms`.
        let rows = vec![
            perf_row(
                r#"{"op":"list_index","phase":"domain_mapping","duration_ms":4}"#,
                4,
            ),
            perf_row(
                r#"{"op":"list_index","phase":"database","duration_ms":1980,"rows":110000,
                     "query_ms":1900,"group_map_ms":60,"count_ms":20}"#,
                1_980,
            ),
        ];
        let sample = newest_db_phase(&rows).expect("the database phase is there");
        assert_eq!(sample.duration_ms, 1_980);
        assert_eq!(sample.rows, Some(110_000));
        assert_eq!(sample.query_ms, Some(1_900));
        assert_eq!(sample.group_map_ms, Some(60));
        assert_eq!(sample.count_ms, Some(20));

        // Nothing to lift: the bench profile persists no perf rows, so
        // "empty" has to be a `None` rather than a zeroed sample.
        assert!(newest_db_phase(&[]).is_none());
        assert!(newest_db_phase(&[perf_row(r#"{"op":"list_index"}"#, 3)]).is_none());
    }

    #[test]
    fn the_cold_query_is_the_one_the_grid_sends() {
        let q = index_query("p-1", 200_000);
        assert_eq!(
            q,
            "/asterism/assets/index?persona_id=p-1&trash=live&offset=0&limit=200000"
        );
        // The live side is explicit. Omitting it would still mean live
        // today, but the trash view is one word away and a silent
        // default is not what a published number should rest on.
        assert!(q.contains("trash=live"));
    }

    #[test]
    fn results_round_trip_through_json() {
        let import = ImportResult {
            schema: SCHEMA_IMPORT.into(),
            manifest: CorpusIdentity {
                seed: 42,
                preset: "s".into(),
                generator_version: "0.2.0".into(),
                count: 5_000,
            },
            server: "http://127.0.0.1:28989".into(),
            registration: LoadReport {
                added: 5_000,
                elapsed_ms: 61_000,
                add_ms: 42_000,
                ..LoadReport::default()
            },
            drain: DrainReport {
                started_at_ms: 1_700_000_000_000,
                drained_at_ms: Some(1_700_000_600_000),
                elapsed_ms: 600_000,
                timed_out: false,
                timeline: vec![DrainTick {
                    t_ms: 0,
                    pending: 10_000,
                    running: 4,
                    done: 0,
                    failed: 0,
                    mem: vec![MemSample {
                        pid: 812,
                        rss_kb: 512_340,
                        comm: "Asterism".into(),
                        footprint: Some(Footprint {
                            phys_footprint_kb: 480_000,
                            phys_footprint_peak_kb: 7_275_000,
                            dirty_kb: 470_000,
                            clean_kb: 42_000,
                            mapped_file_kb: 40_000,
                        }),
                    }],
                }],
            },
        };
        let text = serde_json::to_string(&import).expect("json");
        assert_eq!(
            serde_json::from_str::<ImportResult>(&text).expect("round trip"),
            import
        );

        let cold = ColdResult {
            schema: SCHEMA_COLD.into(),
            server: "http://127.0.0.1:28989".into(),
            server_git_sha: Some("abc1234".into()),
            persona: default_persona(),
            persona_id: "p-1".into(),
            limit: 200_000,
            cold: ListProbe {
                wall_ms: 2_100,
                items: 110_000,
                total: Some(110_000),
                perf: None,
            },
            warm: ListProbe {
                wall_ms: 480,
                items: 110_000,
                total: Some(110_000),
                perf: Some(PerfSample {
                    occurred_at_ms: 1_700_000_000_000,
                    duration_ms: 400,
                    rows: Some(110_000),
                    query_ms: Some(380),
                    group_map_ms: Some(15),
                    count_ms: Some(5),
                }),
            },
            perf_note: Some("no perf record".into()),
        };
        let text = serde_json::to_string(&cold).expect("json");
        assert_eq!(
            serde_json::from_str::<ColdResult>(&text).expect("round trip"),
            cold
        );
        assert_eq!(default_persona(), "bench-persona-0");
    }

    #[test]
    fn the_metadata_tier_has_no_import_to_measure() {
        assert!(ensure_file_tier("l").is_err());
        assert!(ensure_file_tier("s").is_ok() && ensure_file_tier("m").is_ok());
        // Same port refusal as the loader: these commands talk to a
        // server too, and 8989 is the real library.
        assert!(guard_server("http://127.0.0.1:8989", false).is_err());
        assert!(guard_server("http://127.0.0.1:28989", false).is_ok());
    }

    #[test]
    fn result_files_sort_chronologically() {
        assert_eq!(
            result_name("20260805T091500Z", "import-s"),
            "20260805T091500Z-import-s.json"
        );
        let mut names = [
            result_name("20260805T101500Z", "cold"),
            result_name("20260805T091500Z", "cold"),
        ];
        names.sort();
        assert!(names[0].starts_with("20260805T0915"));
    }
}
