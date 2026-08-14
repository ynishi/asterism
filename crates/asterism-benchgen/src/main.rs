//! `asterism-benchgen` — the seeded corpus behind the grid/import benches.
//!
//! Bench data cannot be a folder of whatever images happened to be on the
//! machine: the numbers stop being comparable between runs, and a corpus
//! that came out of one generator run carries that run's accidents (one
//! persona, one aspect ratio, no group of interesting size). So the corpus
//! is defined by a seed and regenerated from it — [`model::SpecStream`] is
//! the definition, everything else here is materialisation.
//!
//! Two tiers, because one corpus cannot serve both benches: 110,000 assets
//! at ~1.3 MB each is 165 GB.
//!
//! - **T-file** (`s` = 5,000 / `m` = 12,000): real PNGs on disk, so the
//!   import path (hash + `thumb_gen` jobs) does real decode work.
//! - **T-meta** (`l` = 110,000): specs only. Rows are seeded straight into
//!   the repository by `seed-meta`; `corpus --preset l` writes just
//!   the manifest, so both tiers agree on what corpus `(seed, l)` means.
//!
//! Presets are prefixes of one stream: S ⊂ M ⊂ L for a given seed.
//!
//! Six subcommands — three that build a corpus, three that measure:
//!
//! - `corpus` — materialise the corpus directory (PNGs + manifest).
//! - `seed-meta` — T-meta: rows straight into the bench profile's
//!   database, thumbnails included ([`seed_meta`]).
//! - `load-file` — T-file: the corpus pushed through the running bench
//!   server's HTTP API so the import jobs do real work ([`load_file`]).
//! - `measure-import` — `load-file` plus the wait for the jobs it
//!   enqueued, written up as a result file ([`measure`]).
//! - `measure-cold` — first-listing cost against a just-restarted
//!   server, warm repeat alongside it ([`measure`]).
//! - `measure-pursuit` — the pursuit membership reads over a seeded
//!   100k-asset profile ([`measure_pursuit`]).
//!
//! Every write path is fenced away from the real library: `seed-meta`
//! refuses a database outside `profiles/bench`, everything that speaks
//! HTTP refuses the Dogfood port, and `measure-pursuit` writes only a
//! throwaway temp directory it creates and removes itself. No fence is
//! optional in the direction that matters — there is no flag that
//! points any command at the real library.

mod image_synth;
mod load_file;
mod manifest;
mod measure;
mod measure_pursuit;
mod model;
mod seed_meta;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image_synth::render_png;
use load_file::{DEFAULT_SERVER, LoadFileArgs};
use manifest::{GENERATOR_VERSION, Manifest, ManifestBuilder};
use measure::{DEFAULT_OUT_DIR, MeasureColdArgs, MeasureImportArgs};
use model::{AssetSpec, SpecStream};
use seed_meta::SeedMetaArgs;

/// Specs are materialised in chunks: bounded memory, and the progress line
/// doubles as the rayon batch boundary.
const CHUNK: u64 = 500;

#[derive(Debug, Parser)]
#[command(
    name = "asterism-benchgen",
    version,
    about = "Generate the seeded synthetic corpus used by the Asterism benches"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Materialise a corpus directory (PNGs for s/m, manifest only for l).
    Corpus(CorpusArgs),
    /// Seed the metadata tier straight into the bench profile's database
    /// (no server, no files, thumbnails pre-filled).
    SeedMeta(SeedMetaCliArgs),
    /// Load the file tier into a running bench server over HTTP.
    LoadFile(LoadFileCliArgs),
    /// Measure an import: load the file tier, then wait for the jobs it
    /// enqueued to drain. Reset the bench profile first — this command
    /// will not do it for you.
    MeasureImport(MeasureImportCliArgs),
    /// Measure the first listing after a restart (and a warm repeat).
    /// Restart the bench server first — that is what "cold" means.
    MeasureCold(MeasureColdCliArgs),
    /// Measure the pursuit membership reads (#29) against a seeded
    /// 100k-asset profile. Self-contained: seeds a throwaway temp
    /// profile and measures through the real repository adapters — no
    /// server, no bench profile, no reset dance.
    MeasurePursuit(measure_pursuit::MeasurePursuitArgs),
}

#[derive(Debug, Args)]
struct CorpusArgs {
    /// s = 5,000 files / m = 12,000 files / l = 110,000 specs, no files.
    #[arg(long, value_enum)]
    preset: Preset,
    /// The corpus identity. Change it and you have a different corpus, not a
    /// variation of this one.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Defaults to `~/.asterism-bench-corpus/<seed>-v1`.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Generate fewer assets than the preset declares. For smoke-testing the
    /// generator itself — a corpus produced this way is a prefix, so it is
    /// not the preset and should not be used for a published number.
    #[arg(long, hide = true)]
    count_override: Option<u64>,
}

#[derive(Debug, Args)]
struct SeedMetaCliArgs {
    /// The metadata tier is the `l` preset; the flag exists so a smaller
    /// prefix can be seeded while developing the seeder itself.
    #[arg(long, value_enum, default_value_t = Preset::L)]
    preset: Preset,
    /// The corpus identity — must match the corpus the locators point
    /// into, or the two tiers describe different assets.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Explicit Asterism home. Without it the bench profile is resolved
    /// and anything else is refused; with it, a scratch directory is
    /// seeded instead (tests, throwaway runs).
    #[arg(long)]
    home: Option<PathBuf>,
    /// Corpus directory the fabricated locators point into. Defaults to
    /// the same `~/.asterism-bench-corpus/<seed>-v1` the generator uses,
    /// so the L tier names files the S / M tiers would have written.
    #[arg(long)]
    corpus_dir: Option<PathBuf>,
    /// Seed fewer assets than the preset declares (smoke runs).
    #[arg(long, hide = true)]
    count_override: Option<u64>,
}

#[derive(Debug, Args)]
struct LoadFileCliArgs {
    /// s = 5,000 files / m = 12,000 files. The metadata tier (`l`) has
    /// no files to load — use `seed-meta` for it.
    #[arg(long, value_enum)]
    preset: Preset,
    /// The corpus identity.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Base URL of the running bench server. Port 8989 (Dogfood) is
    /// refused unless `--allow-any-server` is passed.
    #[arg(long, default_value = DEFAULT_SERVER)]
    server: String,
    /// Corpus directory (defaults as for `seed-meta`).
    #[arg(long)]
    corpus_dir: Option<PathBuf>,
    /// Accept a server URL the port guard would otherwise refuse.
    #[arg(long)]
    allow_any_server: bool,
    /// Load fewer assets than the preset declares (smoke runs).
    #[arg(long, hide = true)]
    count_override: Option<u64>,
}

#[derive(Debug, Args)]
struct MeasureImportCliArgs {
    /// s = 5,000 files / m = 12,000 files. The metadata tier (`l`) has
    /// no import to measure.
    #[arg(long, value_enum)]
    preset: Preset,
    /// The corpus identity.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Base URL of the running bench server.
    #[arg(long, default_value = DEFAULT_SERVER)]
    server: String,
    /// Corpus directory (defaults as for `load-file`).
    #[arg(long)]
    corpus_dir: Option<PathBuf>,
    /// Where the result file goes.
    #[arg(long, default_value = DEFAULT_OUT_DIR)]
    out_dir: PathBuf,
    /// Accept a server URL the port guard would otherwise refuse.
    #[arg(long)]
    allow_any_server: bool,
    /// Measure fewer assets than the preset declares (smoke runs).
    #[arg(long, hide = true)]
    count_override: Option<u64>,
}

#[derive(Debug, Args)]
struct MeasureColdCliArgs {
    /// Base URL of the running bench server.
    #[arg(long, default_value = DEFAULT_SERVER)]
    server: String,
    /// Persona display name to filter by; the grid always has one
    /// selected, so measuring without one would not be the same query.
    #[arg(long)]
    persona: Option<String>,
    /// Page size. The desktop grid asks for 200,000 (the server's own
    /// ceiling) and paints the visible rows out of it.
    #[arg(long, default_value_t = 200_000)]
    limit: u64,
    /// Where the result file goes.
    #[arg(long, default_value = DEFAULT_OUT_DIR)]
    out_dir: PathBuf,
    /// Accept a server URL the port guard would otherwise refuse.
    #[arg(long)]
    allow_any_server: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Preset {
    S,
    M,
    L,
}

impl Preset {
    fn count(self) -> u64 {
        match self {
            Preset::S => 5_000,
            Preset::M => 12_000,
            Preset::L => 110_000,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Preset::S => "s",
            Preset::M => "m",
            Preset::L => "l",
        }
    }

    /// The metadata-only tier writes no image files.
    fn writes_files(self) -> bool {
        !matches!(self, Preset::L)
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Corpus(args) => run_corpus(args),
        // The two injection subcommands are async (both the repository
        // ports and the HTTP client are), so the runtime is built here
        // rather than over `main` — `corpus` is pure CPU work and has no
        // use for one.
        Command::SeedMeta(args) => runtime()?.block_on(run_seed_meta(args)),
        Command::LoadFile(args) => runtime()?.block_on(run_load_file(args)),
        Command::MeasureImport(args) => runtime()?.block_on(run_measure_import(args)),
        Command::MeasureCold(args) => runtime()?.block_on(run_measure_cold(args)),
        Command::MeasurePursuit(args) => runtime()?.block_on(measure_pursuit::run(args)),
    }
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot start the tokio runtime")
}

async fn run_seed_meta(args: SeedMetaCliArgs) -> Result<()> {
    let corpus_dir = match args.corpus_dir {
        Some(dir) => dir,
        None => default_out_dir(args.seed)?,
    };
    let report = seed_meta::run(SeedMetaArgs {
        seed: args.seed,
        preset: args.preset.as_str(),
        count: args.count_override.unwrap_or(args.preset.count()),
        home: args.home,
        corpus_dir,
    })
    .await?;
    eprintln!("{}", seed_meta::report_line(&report));
    Ok(())
}

async fn run_load_file(args: LoadFileCliArgs) -> Result<()> {
    anyhow::ensure!(
        args.preset.writes_files(),
        "the {} preset writes no files — seed it with `seed-meta` instead",
        args.preset.as_str()
    );
    let corpus_dir = match args.corpus_dir {
        Some(dir) => dir,
        None => default_out_dir(args.seed)?,
    };
    let report = load_file::run(LoadFileArgs {
        seed: args.seed,
        preset: args.preset.as_str(),
        count: args.count_override.unwrap_or(args.preset.count()),
        corpus_dir,
        server: args.server,
        allow_any_server: args.allow_any_server,
    })
    .await?;
    eprintln!("{}", load_file::report_line(&report));
    Ok(())
}

async fn run_measure_import(args: MeasureImportCliArgs) -> Result<()> {
    let corpus_dir = match args.corpus_dir {
        Some(dir) => dir,
        None => default_out_dir(args.seed)?,
    };
    measure::run_import(MeasureImportArgs {
        seed: args.seed,
        preset: args.preset.as_str(),
        count: args.count_override.unwrap_or(args.preset.count()),
        corpus_dir,
        server: args.server,
        allow_any_server: args.allow_any_server,
        out_dir: args.out_dir,
    })
    .await?;
    Ok(())
}

async fn run_measure_cold(args: MeasureColdCliArgs) -> Result<()> {
    measure::run_cold(MeasureColdArgs {
        server: args.server,
        allow_any_server: args.allow_any_server,
        persona: args.persona.unwrap_or_else(measure::default_persona),
        limit: args.limit,
        out_dir: args.out_dir,
    })
    .await?;
    Ok(())
}

fn run_corpus(args: CorpusArgs) -> Result<()> {
    let out = match args.out {
        Some(dir) => dir,
        None => default_out_dir(args.seed)?,
    };
    let preset = args.preset;
    let count = args.count_override.unwrap_or(preset.count());
    let manifest_path = out.join(format!("manifest-{}.json", preset.as_str()));

    if let Some(existing) = read_manifest(&manifest_path)?
        && existing.covers(args.seed, preset.as_str(), count)
    {
        eprintln!(
            "benchgen: {} already holds seed={} preset={} count={} — nothing to do",
            manifest_path.display(),
            existing.seed,
            existing.preset,
            existing.count
        );
        return Ok(());
    }

    fs::create_dir_all(&out)
        .with_context(|| format!("cannot create corpus dir {}", out.display()))?;
    if preset.writes_files() {
        fs::create_dir_all(out.join("files"))
            .with_context(|| format!("cannot create files dir under {}", out.display()))?;
    }

    eprintln!(
        "benchgen: seed={} preset={} count={count} out={} ({})",
        args.seed,
        preset.as_str(),
        out.display(),
        if preset.writes_files() {
            "writing PNGs"
        } else {
            "manifest only"
        }
    );

    let started = Instant::now();
    let mut stream = SpecStream::new(args.seed);
    let mut builder = ManifestBuilder::new(args.seed, preset.as_str());

    while builder.count() < count {
        let take = CHUNK.min(count - builder.count()) as usize;
        let chunk: Vec<AssetSpec> = stream.by_ref().take(take).collect();

        if preset.writes_files() {
            // Encoding is the whole cost here and every asset is
            // independent, so the parallelism goes across assets rather
            // than inside one render; results fold back in stream order.
            let sizes: Vec<Result<u64>> = chunk
                .par_iter()
                .map(|spec| materialise(&out, spec))
                .collect();
            for (spec, size) in chunk.iter().zip(sizes) {
                let bytes = size?;
                builder.observe(spec, Some(bytes));
            }
        } else {
            for spec in &chunk {
                builder.observe(spec, None);
            }
        }

        eprintln!(
            "benchgen: {}/{count} ({:.1}s)",
            builder.count(),
            started.elapsed().as_secs_f64()
        );
    }

    let manifest = builder.finish(started.elapsed().as_millis() as u64);
    let json = serde_json::to_string_pretty(&manifest).context("cannot serialise manifest")?;
    fs::write(&manifest_path, json)
        .with_context(|| format!("cannot write {}", manifest_path.display()))?;

    report(&manifest, &manifest_path);
    Ok(())
}

/// Write one asset's PNG, or accept the one already on disk.
///
/// The existence check is what makes an interrupted run resumable: a partial
/// corpus continues instead of re-encoding from zero. Files are written to a
/// temporary name and renamed, so an interrupted *write* leaves no truncated
/// PNG that a later run would accept as complete.
fn materialise(out: &Path, spec: &AssetSpec) -> Result<u64> {
    let path = out.join(&spec.rel_path);
    if let Ok(meta) = fs::metadata(&path)
        && meta.len() > 0
    {
        return Ok(meta.len());
    }

    let bytes = render_png(spec)?;
    let tmp = path.with_extension("png.part");
    fs::write(&tmp, &bytes).with_context(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("cannot finalise {}", path.display()))?;
    Ok(bytes.len() as u64)
}

fn read_manifest(path: &Path) -> Result<Option<Manifest>> {
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    // A manifest that cannot be parsed is reported rather than ignored:
    // silently regenerating over an unreadable one is how two generations
    // end up mixed in a single corpus directory.
    let manifest: Manifest = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a benchgen manifest", path.display()))?;
    Ok(Some(manifest))
}

fn default_out_dir(seed: u64) -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME is unset; pass --out explicitly")?;
    Ok(PathBuf::from(home)
        .join(".asterism-bench-corpus")
        .join(format!("{seed}-v1")))
}

fn report(manifest: &Manifest, path: &Path) {
    eprintln!("benchgen: wrote {}", path.display());
    eprintln!(
        "  generator={GENERATOR_VERSION} seed={} preset={} count={} elapsed={}ms",
        manifest.seed, manifest.preset, manifest.count, manifest.elapsed_ms
    );
    if let Some(hist) = &manifest.size_histogram {
        eprintln!(
            "  size MB  <0.5:{} 0.5-0.8:{} 0.8-1.2:{} 1.2-1.6:{} 1.6-2.0:{} >2.0:{}",
            hist.lt_0_5mb,
            hist.mb_0_5_to_0_8,
            hist.mb_0_8_to_1_2,
            hist.mb_1_2_to_1_6,
            hist.mb_1_6_to_2_0,
            hist.gt_2_0mb
        );
    }
    if let Some(median) = manifest.size_median_bytes {
        eprintln!(
            "  size median={median} bytes total={:?}",
            manifest.size_total_bytes
        );
    }
    eprintln!("  persona={:?}", manifest.persona_counts);
    eprintln!(
        "  tags={} labels={} rated={} trashed={}",
        manifest.tag_total, manifest.label_total, manifest.rated_count, manifest.trashed_count
    );
    for (name, n) in manifest.top_groups(5) {
        eprintln!("  group {name}: {n}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_the_designed_scales() {
        assert_eq!(Preset::S.count(), 5_000);
        assert_eq!(Preset::M.count(), 12_000);
        assert_eq!(Preset::L.count(), 110_000);
        assert!(Preset::S.writes_files() && Preset::M.writes_files());
        assert!(
            !Preset::L.writes_files(),
            "the metadata tier must not materialise 110k files"
        );
    }

    #[test]
    fn cli_parses_the_corpus_form() {
        let cli = Cli::try_parse_from([
            "asterism-benchgen",
            "corpus",
            "--preset",
            "m",
            "--seed",
            "7",
            "--count-override",
            "200",
        ])
        .expect("parse");
        let Command::Corpus(args) = cli.command else {
            panic!("expected the corpus subcommand");
        };
        assert_eq!(args.preset, Preset::M);
        assert_eq!(args.seed, 7);
        assert_eq!(args.count_override, Some(200));
        assert!(args.out.is_none());
    }

    #[test]
    fn cli_defaults_of_the_injection_subcommands() {
        // The two defaults that matter are the ones a hurried operator
        // relies on: `seed-meta` means the L preset, and `load-file`
        // means the bench server — never the Dogfood port.
        let cli = Cli::try_parse_from(["asterism-benchgen", "seed-meta"]).expect("parse");
        let Command::SeedMeta(args) = cli.command else {
            panic!("expected the seed-meta subcommand");
        };
        assert_eq!(args.preset, Preset::L);
        assert_eq!(args.seed, 42);
        assert!(args.home.is_none() && args.corpus_dir.is_none());

        let cli = Cli::try_parse_from(["asterism-benchgen", "load-file", "--preset", "s"])
            .expect("parse");
        let Command::LoadFile(args) = cli.command else {
            panic!("expected the load-file subcommand");
        };
        assert_eq!(args.preset, Preset::S);
        assert_eq!(args.server, DEFAULT_SERVER);
        assert!(!args.allow_any_server);
        // `load-file` has no preset that writes no files; `run_load_file`
        // rejects `l` rather than posting 110,000 locators that name
        // nothing on disk.
        assert!(!Preset::L.writes_files());
    }

    #[test]
    fn cli_defaults_of_the_measurement_subcommands() {
        // The defaults an operator relies on mid-run: the bench server
        // (never Dogfood), the persona the corpus files everything
        // under, the grid's own page size, and a result directory
        // inside the gitignored workspace.
        let cli = Cli::try_parse_from(["asterism-benchgen", "measure-import", "--preset", "s"])
            .expect("parse");
        let Command::MeasureImport(args) = cli.command else {
            panic!("expected the measure-import subcommand");
        };
        assert_eq!(args.preset, Preset::S);
        assert_eq!(args.server, DEFAULT_SERVER);
        assert_eq!(args.out_dir, PathBuf::from(DEFAULT_OUT_DIR));
        assert!(!args.allow_any_server && args.corpus_dir.is_none());

        let cli = Cli::try_parse_from(["asterism-benchgen", "measure-cold"]).expect("parse");
        let Command::MeasureCold(args) = cli.command else {
            panic!("expected the measure-cold subcommand");
        };
        assert_eq!(args.server, DEFAULT_SERVER);
        assert_eq!(args.limit, 200_000, "the grid's own page size");
        assert!(args.persona.is_none(), "resolved to bench-persona-0 later");
        assert_eq!(args.out_dir, PathBuf::from(DEFAULT_OUT_DIR));

        // `measure-import` has to name a preset — there is no default
        // scale for a published import number.
        assert!(Cli::try_parse_from(["asterism-benchgen", "measure-import"]).is_err());
    }

    #[test]
    fn default_out_dir_is_seed_scoped() {
        // Two seeds must never share a directory: a corpus is its seed.
        let a = default_out_dir(42).expect("HOME");
        let b = default_out_dir(43).expect("HOME");
        assert_ne!(a, b);
        assert!(a.ends_with("42-v1"));
    }
}
