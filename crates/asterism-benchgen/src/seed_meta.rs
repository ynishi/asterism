//! `seed-meta` — the metadata tier (T-meta), written straight into a
//! bench profile's SQLite database.
//!
//! The L preset is 110,000 assets. At the T-file tier's ~1.5 MB per
//! original that would be 165 GB of disk, so this tier writes **no
//! files**: rows are seeded through the repository ports and the grid is
//! made paintable by pre-filling `thumb_cache` with a seeded 256 px
//! JPEG per asset. What it measures is the cold-load and large-group
//! scroll question, not the import
//! pipeline — that is the T-file tier's job.
//!
//! ## Why the repository ports and not raw SQL
//!
//! The schema moves (V50 at the time of writing). A seeder holding its
//! own `INSERT` statements drifts silently: it keeps working, and the
//! rows it produces stop looking like the rows the app produces —
//! at which point the bench is measuring a corpus the product cannot
//! make. Going through `SqliteAssetRepository::save` and friends means a
//! column added tomorrow is written the way the app writes it, or the
//! seeder fails to compile.
//!
//! The composition root (`init_core`) is deliberately *not* used: it
//! spawns job workers, takes the Tantivy index lock, and enqueues
//! backfills on start — all of which would race the seeding it is
//! supposed to support.
//!
//! ## Where it is allowed to write
//!
//! One place: the bench profile. [`resolve_db_path`] refuses anything
//! else, and the Dogfood home is rejected outright even when named
//! explicitly — a seeder that can reach the user's real library is one
//! typo away from destroying it. `--home` exists for scratch runs
//! (tests, a throwaway directory) and is the only escape.
//!
//! ## Known limitations
//!
//! - The Tantivy index stays empty — search benches are not in scope.
//! - `palette` is not extracted, so the COLOR facet has nothing to
//!   count.
//! - `content_hash` stays `NULL`: it is owned by `set_material_hash`,
//!   not by `save`, so the duplicate report reads the whole corpus as
//!   "not answered yet".
//! - Only the 256 px thumbnail is seeded; opening a detail view misses,
//!   enqueues nothing (no ingest), and finds no original behind the
//!   locator.

use anyhow::{Context, Result, anyhow, bail};
use asterism_core::domain::asset::Asset;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::material::Material;
use asterism_core::domain::persona::Persona;
use asterism_core::domain::repository::{
    AssetRepository, GroupRepository, PersonaRepository, SourceLookupScope, TagRepository,
    ThumbRepository,
};
use asterism_core::domain::value::{
    AssetId, GroupId, Label, Modality, PersonaId, SourceKind, SourceRef, dedup_labels,
};
use asterism_infra::paths;
use asterism_infra::sqlite;
// `SqliteGroupRepository` is the one adapter `repo` does not re-export
// at the top level, so it comes in through its own module.
use asterism_infra::sqlite::repo::group::SqliteGroupRepository;
use asterism_infra::sqlite::repo::{
    SqliteAssetRepository, SqlitePersonaRepository, SqliteTagRepository, SqliteThumbRepository,
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::image_synth::render_thumb_jpeg;
use crate::model::{AssetSpec, SpecStream, group_pools};

/// Thumbnail size seeded into `thumb_cache` — the one the grid asks for.
pub const THUMB_SIZE_PX: u32 = 256;
/// Label the service path attaches on ingest. The repository path does
/// not go through the service, so it is added here: without it the two
/// tiers disagree about what a freshly imported asset looks like, and an
/// `inbox` filter would show one tier and not the other.
pub const INBOX_LABEL: &str = "inbox";
/// Modality every synthetic asset carries (image 100 % in
/// v1). Not registered in the `modality` master — `asset.modality` has
/// no FK, and an unregistered slug is a normal importer state.
pub const BENCH_MODALITY: &str = "image";
/// Persona display names. Fixed, because the bench driver selects a
/// persona by name.
pub const PERSONA_PREFIX: &str = "bench-persona-";
/// Progress line cadence.
const PROGRESS_EVERY: u64 = 1_000;
/// Pseudo file size band for the material layer, in bytes. The tier
/// writes no files, so this stands in for what the T-file tier measures
/// — same 0.9–1.8 MB band the PNG synthesiser targets, so `file_size`
/// sorts and weight signals behave the same on both tiers.
const PSEUDO_SIZE: (u64, u64) = (900_000, 1_800_000);

/// Everything `seed-meta` needs; the CLI layer builds it.
#[derive(Debug, Clone)]
pub struct SeedMetaArgs {
    /// Corpus identity.
    pub seed: u64,
    /// Preset label, recorded in the report (`l` in practice).
    pub preset: &'static str,
    /// How many assets to seed.
    pub count: u64,
    /// Explicit home directory — the scratch escape. `None` resolves the
    /// bench profile.
    pub home: Option<PathBuf>,
    /// Corpus directory the fabricated locators point into. The files do
    /// not have to exist (and for the L preset they never will).
    pub corpus_dir: PathBuf,
}

/// What one seeding run put into the database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeedReport {
    /// Assets saved.
    pub assets: u64,
    /// Personas that existed or were created.
    pub personas: u64,
    /// Groups that existed or were created.
    pub groups: u64,
    /// `asset_tag` links written.
    pub tag_links: u64,
    /// Distinct tag rows touched.
    pub tags: u64,
    /// `asset_bucket` rows actually added by the bulk appends.
    pub group_members: u64,
    /// Thumbnails written to `thumb_cache`.
    pub thumbs: u64,
    /// Assets carrying a trash stamp.
    pub trashed: u64,
    /// Wall-clock duration.
    pub elapsed_ms: u64,
}

/// Resolves the database file to seed, refusing anything that is not a
/// bench (or explicitly named scratch) home.
///
/// The `--home` path is taken literally rather than through
/// [`paths::asterism_home`] so that no process-wide environment variable
/// has to be set for it: mutating the environment is unsound under a
/// multi-threaded test runner, and this is the path the tests use.
pub fn resolve_db_path(home: Option<&Path>) -> Result<PathBuf> {
    match home {
        Some(dir) => {
            let db = dir.join("asterism.db");
            assert_seedable(&db, true)?;
            Ok(db)
        }
        None => {
            // SAFETY: called once, from `main`, before any task or
            // thread that reads the environment exists. `paths` reads
            // these two variables and nothing else selects the profile.
            unsafe {
                std::env::set_var("ASTERISM_PROFILE", "bench");
                // An inherited `ASTERISM_HOME` would silently win over
                // the profile — including one pointing at Dogfood.
                std::env::remove_var("ASTERISM_HOME");
            }
            let db = paths::default_db_path().map_err(|e| anyhow!("{e}"))?;
            assert_seedable(&db, false)?;
            Ok(db)
        }
    }
}

/// The write guard, split out so it is testable without touching the
/// filesystem or the environment.
///
/// Two rules:
///
/// 1. Without `--home`, the resolved path must sit under
///    `profiles/bench`. Anything else means the profile resolution did
///    not land where this command assumes it does, and seeding 110,000
///    synthetic rows into an unknown database is not recoverable by
///    hand.
/// 2. The Dogfood profile is refused either way. It is the user's real
///    library; no flag on this command may reach it.
fn assert_seedable(db_path: &Path, explicit_home: bool) -> Result<()> {
    let shown = db_path.display().to_string();
    let normalised = shown.replace('\\', "/");
    if normalised.contains("profiles/dogfood") {
        bail!(
            "refusing to seed {shown}: that is the Dogfood profile — the real library. \
             seed-meta writes to the bench profile only"
        );
    }
    if !explicit_home && !normalised.contains("profiles/bench") {
        bail!(
            "refusing to seed {shown}: expected a database under profiles/bench. \
             Something else is deciding where the bench profile lives (an inherited \
             ASTERISM_HOME, an unusual HOME); pass --home explicitly for a scratch run"
        );
    }
    Ok(())
}

/// Seeds `args.count` assets, returning what landed.
pub async fn run(args: SeedMetaArgs) -> Result<SeedReport> {
    let db_path = resolve_db_path(args.home.as_deref())?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    eprintln!(
        "benchgen: seed-meta seed={} preset={} count={} db={}",
        args.seed,
        args.preset,
        args.count,
        db_path.display()
    );

    let (isle, driver) = sqlite::open_and_migrate(&db_path)
        .await
        .map_err(|e| anyhow!("cannot open {}: {e}", db_path.display()))?;

    // The driver owns the SQLite thread; it stays alive until the seed
    // is done and is then shut down so queued work drains. The result is
    // kept rather than `?`-ed so a failure mid-seed still shuts down.
    let outcome = seed(&isle, &args).await;
    driver
        .shutdown()
        .await
        .map_err(|e| anyhow!("SQLite shutdown failed: {e}"))?;
    outcome
}

async fn seed(isle: &rusqlite_isle::AsyncIsle, args: &SeedMetaArgs) -> Result<SeedReport> {
    let started = Instant::now();
    let personas_repo = SqlitePersonaRepository::new(isle.clone());
    let groups_repo = SqliteGroupRepository::new(isle.clone());
    let assets_repo = SqliteAssetRepository::new(isle.clone());
    let tags_repo = SqliteTagRepository::new(isle.clone());
    let thumbs_repo = SqliteThumbRepository::new(isle.clone());

    let persona_ids = ensure_personas(&personas_repo).await?;
    // Groups belong to persona 0 alone (see the `model` module docs,
    // "Groups belong to one persona"), so the whole pool table is
    // created under it.
    let group_ids = ensure_groups(&groups_repo, persona_ids[0], args.seed).await?;

    let now = Utc::now();
    let modality = Modality::new(BENCH_MODALITY).map_err(|e| anyhow!("{e}"))?;
    let source_kind = SourceKind::new(SourceKind::FS).map_err(|e| anyhow!("{e}"))?;
    let attribution = AttributionContext::asserted(None, None).map_err(|e| anyhow!("{e}"))?;

    let mut report = SeedReport {
        personas: persona_ids.len() as u64,
        groups: group_ids.len() as u64,
        ..SeedReport::default()
    };
    // One `find_or_create` per vocabulary entry, not per link: the tag
    // vocabulary is 240 names against ~3 links per asset, so caching
    // turns 330,000 round trips into 240.
    let mut tag_cache: HashMap<String, asterism_core::domain::value::TagId> = HashMap::new();
    let mut memberships: HashMap<String, Vec<AssetId>> = HashMap::new();
    let mut processed = 0u64;

    for spec in SpecStream::new(args.seed).take(args.count as usize) {
        let persona_id = *persona_ids
            .get(spec.persona_idx)
            .ok_or_else(|| anyhow!("spec {} names persona {}", spec.index, spec.persona_idx))?;
        let occurred_at = DateTime::from_timestamp_millis(spec.occurred_at_ms)
            .ok_or_else(|| anyhow!("spec {} has an unrepresentable occurred_at", spec.index))?;
        let locator = locator_of(&args.corpus_dir, &spec);
        let size = pseudo_size(&spec);

        let mut source = SourceRef::new(source_kind.clone(), &locator)
            .map_err(|e| anyhow!("spec {}: {e}", spec.index))?;
        source.file_size_bytes = Some(size);
        let material_locator = source.locator.clone();

        // Look the Source value up before minting, the way the ingest
        // path does — which is what makes a re-seed
        // land on the rows it already wrote instead of doubling them.
        // It used to be the `(source_kind, source_locator)` UNIQUE that
        // stopped a second full seed, by failing it; V61 demoted that to
        // a lookup, and a seeder that skipped the lookup would silently
        // seed twice.
        //
        // `Any` rather than `Live`: some of these specs are seeded
        // trashed, and the question here is "does this fixture row
        // exist", not the ingest question "is this record here".
        //
        // Everything after the mint is already per-pair idempotent
        // (`link`, `add_bulk`'s `INSERT OR IGNORE`, the thumb upsert),
        // so a spec whose row is already there re-runs against the id
        // that row already has.
        let held = assets_repo
            .find_by_source(
                &persona_id,
                &source.kind,
                &source.locator,
                SourceLookupScope::Any,
            )
            .await
            .map_err(|e| anyhow!("spec {}: {e}", spec.index))?;
        let asset_id = match held {
            Some(existing) => existing.id,
            None => {
                let mut asset = Asset::new(
                    persona_id,
                    source,
                    Some(modality.clone()),
                    occurred_at,
                    &attribution,
                );
                asset.labels = labels_of(&spec)?;
                asset.rating = spec.rating;
                if spec.trashed {
                    // Stamped at the occurrence, not at `now`: a corpus
                    // whose trash all arrived in the same millisecond
                    // makes the retention scan's ordering vacuous.
                    asset.trashed_at = Some(occurred_at);
                    report.trashed += 1;
                }
                asset
                    .attach_material(Material::primary(material_locator, Some(size), now))
                    .map_err(|e| anyhow!("spec {}: {e}", spec.index))?;
                assets_repo.save(&asset).await.map_err(|e| anyhow!("{e}"))?;
                report.assets += 1;
                asset.id
            }
        };

        for name in &spec.tags {
            let tag_id = match tag_cache.get(name) {
                Some(id) => *id,
                None => {
                    let tag = tags_repo
                        .find_or_create(name)
                        .await
                        .map_err(|e| anyhow!("{e}"))?;
                    tag_cache.insert(name.clone(), tag.id);
                    tag.id
                }
            };
            tags_repo
                .link(&asset_id, &tag_id)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            report.tag_links += 1;
        }

        for name in &spec.groups {
            memberships.entry(name.clone()).or_default().push(asset_id);
        }

        let thumb = render_thumb_jpeg(&spec, THUMB_SIZE_PX)?;
        thumbs_repo
            .upsert(&asset_id, THUMB_SIZE_PX, thumb)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        report.thumbs += 1;

        // Counted over specs handled rather than rows minted: on a
        // re-seed nothing is minted, and a counter that never moved
        // would print on every single iteration.
        processed += 1;
        if processed.is_multiple_of(PROGRESS_EVERY) {
            eprintln!(
                "benchgen: {processed}/{} ({:.1}s)",
                args.count,
                started.elapsed().as_secs_f64()
            );
        }
    }
    report.tags = tag_cache.len() as u64;

    // Membership last, in one bulk append per group: `add_bulk` holds a
    // single transaction and is the path documented to hold at 100k
    // members, which the per-item `add` loop is not.
    for (name, ordered) in &memberships {
        let group_id = group_ids
            .get(name)
            .ok_or_else(|| anyhow!("spec names group {name}, which no pool created"))?;
        report.group_members += groups_repo
            .add_bulk(group_id, ordered, now)
            .await
            .map_err(|e| anyhow!("{e}"))?;
    }

    report.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

/// Find-or-create the six bench personas, returned by index so a spec's
/// `persona_idx` maps straight onto one. Idempotent: a re-run against a
/// seeded profile reuses them by name.
async fn ensure_personas(repo: &SqlitePersonaRepository) -> Result<Vec<PersonaId>> {
    let existing = repo.list().await.map_err(|e| anyhow!("{e}"))?;
    let by_name: HashMap<&str, PersonaId> = existing
        .iter()
        .map(|p| (p.name.as_str(), p.id))
        .collect::<HashMap<_, _>>();

    let mut ids = Vec::with_capacity(crate::model::PERSONA_COUNT);
    for idx in 0..crate::model::PERSONA_COUNT {
        let name = persona_name(idx);
        match by_name.get(name.as_str()) {
            Some(id) => ids.push(*id),
            None => {
                let persona = Persona::new(&name, None).map_err(|e| anyhow!("{e}"))?;
                repo.save(&persona).await.map_err(|e| anyhow!("{e}"))?;
                ids.push(persona.id);
            }
        }
    }
    Ok(ids)
}

/// Find-or-create the whole pool table under one persona, returning a
/// name → id map. Idempotent by `(persona_id, name)`, which is the
/// uniqueness the storage layer already enforces.
async fn ensure_groups(
    repo: &SqliteGroupRepository,
    persona_id: PersonaId,
    seed: u64,
) -> Result<HashMap<String, GroupId>> {
    let existing = repo
        .list(Some(&persona_id))
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let mut ids: HashMap<String, GroupId> = existing
        .into_iter()
        .map(|s| (s.group.name.clone(), s.group.id))
        .collect();

    let now = Utc::now();
    for pool in group_pools(seed) {
        if ids.contains_key(&pool.name) {
            continue;
        }
        let group = repo
            .create(persona_id, pool.name.clone(), None, now)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        ids.insert(pool.name, group.id);
    }
    Ok(ids)
}

/// Display name of persona `idx`.
pub fn persona_name(idx: usize) -> String {
    format!("{PERSONA_PREFIX}{idx}")
}

/// Absolute locator the asset claims as its original. Same layout the
/// T-file tier writes, so a spec means the same file on both tiers —
/// on this one, a file that was never materialised.
pub fn locator_of(corpus_dir: &Path, spec: &AssetSpec) -> String {
    corpus_dir.join(&spec.rel_path).display().to_string()
}

/// Stand-in original size, derived from the spec so a re-seed reports
/// the same bytes.
pub fn pseudo_size(spec: &AssetSpec) -> u64 {
    let span = PSEUDO_SIZE.1 - PSEUDO_SIZE.0;
    PSEUDO_SIZE.0 + (spec.image_seed % (span + 1))
}

/// The spec's labels plus `inbox` (see [`INBOX_LABEL`]).
///
/// Seeded rows go through `dedup_labels` on the same terms as the
/// ingest path: this writes real rows into a profile database, and a
/// spec that already lists `inbox` would otherwise seed a corpus the
/// grid cannot render (duplicate label chip keys).
fn labels_of(spec: &AssetSpec) -> Result<Vec<Label>> {
    let mut labels = Vec::with_capacity(spec.labels.len() + 1);
    for name in &spec.labels {
        labels.push(Label::new(name.clone()).map_err(|e| anyhow!("{e}"))?);
    }
    labels.push(Label::new(INBOX_LABEL).map_err(|e| anyhow!("{e}"))?);
    Ok(dedup_labels(labels))
}

/// Human-readable completion line.
pub fn report_line(report: &SeedReport) -> String {
    format!(
        "benchgen: seeded {} assets / {} personas / {} groups ({} memberships) / \
         {} tag links over {} tags / {} thumbs / {} trashed in {} ms",
        report.assets,
        report.personas,
        report.groups,
        report.group_members,
        report.tag_links,
        report.tags,
        report.thumbs,
        report.trashed,
        report.elapsed_ms
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::asset::{AssetQuery, TrashFilter};

    #[test]
    fn the_guard_refuses_anything_but_a_bench_profile() {
        // The whole point of the command having a guard at all.
        let dogfood = PathBuf::from("/Users/x/.asterism/profiles/dogfood/asterism.db");
        let err = assert_seedable(&dogfood, false).expect_err("dogfood must be refused");
        assert!(err.to_string().contains("Dogfood"), "{err}");
        // Naming it explicitly does not help: `--home` is a scratch
        // escape, not a way past this one.
        let err = assert_seedable(&dogfood, true).expect_err("dogfood must be refused");
        assert!(err.to_string().contains("Dogfood"), "{err}");

        // Without `--home`, a non-bench resolution is refused too — this
        // is what an inherited ASTERISM_HOME would look like.
        let dev = PathBuf::from("/Users/x/.asterism/profiles/dev/asterism.db");
        assert!(assert_seedable(&dev, false).is_err());
        assert!(
            assert_seedable(&dev, true).is_ok(),
            "an explicitly named scratch home is the sanctioned escape"
        );

        let bench = PathBuf::from("/Users/x/.asterism/profiles/bench/asterism.db");
        assert!(assert_seedable(&bench, false).is_ok());
        assert!(assert_seedable(&bench, true).is_ok());
    }

    #[test]
    fn a_scratch_home_resolves_without_touching_the_environment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = resolve_db_path(Some(dir.path())).expect("scratch home");
        assert_eq!(db, dir.path().join("asterism.db"));
    }

    #[test]
    fn derived_values_are_a_function_of_the_spec() {
        let spec = SpecStream::new(42).next().expect("infinite");
        assert_eq!(pseudo_size(&spec), pseudo_size(&spec));
        assert!((PSEUDO_SIZE.0..=PSEUDO_SIZE.1).contains(&pseudo_size(&spec)));
        assert_eq!(
            locator_of(Path::new("/corpus/42-v1"), &spec),
            format!("/corpus/42-v1/{}", spec.rel_path)
        );
        let labels = labels_of(&spec).expect("labels");
        assert_eq!(
            labels.last().map(|l| l.as_str()),
            Some(INBOX_LABEL),
            "the service path's inbox label must be mirrored here"
        );
        assert_eq!(labels.len(), spec.labels.len() + 1);
    }

    /// The one test that actually seeds: a scratch home, a small count,
    /// and every count read back through the same ports the app reads
    /// through. No server, no job workers.
    #[tokio::test]
    async fn seeding_a_scratch_home_fills_every_table_the_grid_reads() {
        const COUNT: u64 = 120;
        let dir = tempfile::tempdir().expect("tempdir");
        let args = SeedMetaArgs {
            seed: 42,
            preset: "l",
            count: COUNT,
            home: Some(dir.path().to_path_buf()),
            corpus_dir: dir.path().join("corpus"),
        };
        let report = run(args).await.expect("seed");
        assert_eq!(report.assets, COUNT);
        assert_eq!(report.thumbs, COUNT);
        assert_eq!(report.personas as usize, crate::model::PERSONA_COUNT);
        assert!(report.tag_links > 0 && report.group_members > 0);

        let specs: Vec<AssetSpec> = SpecStream::new(42).take(COUNT as usize).collect();
        let expected_members: usize = specs.iter().map(|s| s.groups.len()).sum();
        assert_eq!(report.group_members as usize, expected_members);

        // Read back through the ports rather than trusting the report.
        let (isle, driver) = sqlite::open_and_migrate(dir.path().join("asterism.db"))
            .await
            .expect("reopen");
        let assets = SqliteAssetRepository::new(isle.clone());
        let personas = SqlitePersonaRepository::new(isle.clone());
        let groups = SqliteGroupRepository::new(isle.clone());
        let thumbs = SqliteThumbRepository::new(isle.clone());

        assert_eq!(personas.list().await.expect("personas").len(), 6);

        let live = assets
            .list(&AssetQuery {
                limit: 1_000,
                ..AssetQuery::default()
            })
            .await
            .expect("list");
        let trashed_expected = specs.iter().filter(|s| s.trashed).count();
        assert_eq!(
            live.items.len(),
            COUNT as usize - trashed_expected,
            "the trash stamp must survive `save`"
        );
        assert_eq!(report.trashed as usize, trashed_expected);

        let all = assets
            .list(&AssetQuery {
                limit: 1_000,
                trash: TrashFilter::Any,
                ..AssetQuery::default()
            })
            .await
            .expect("list all");
        assert_eq!(all.items.len(), COUNT as usize);
        let card = all.items.first().expect("at least one card");
        assert!(
            card.labels.iter().any(|l| l.as_str() == INBOX_LABEL),
            "every seeded asset carries the inbox label"
        );
        assert_eq!(card.mime.as_deref(), Some("image/png"));
        assert!(card.file_size_bytes.is_some());

        // Group counts: `list` counts live assets only, so compare
        // against the live half of what the specs asked for.
        let persona0 = personas
            .list()
            .await
            .expect("personas")
            .into_iter()
            .find(|p| p.name == persona_name(0))
            .expect("persona 0");
        let summaries = groups.list(Some(&persona0.id)).await.expect("groups");
        let seeded_members: u64 = summaries.iter().map(|s| s.asset_count).sum();
        let live_members = specs
            .iter()
            .filter(|s| !s.trashed)
            .map(|s| s.groups.len() as u64)
            .sum::<u64>();
        assert_eq!(seeded_members, live_members);
        assert!(
            summaries.len() >= group_pools(42).len(),
            "the whole pool table must exist so the bench driver can name a target"
        );

        // Every asset has its grid tile.
        for card in &all.items {
            let blob = thumbs
                .get(&card.id, THUMB_SIZE_PX)
                .await
                .expect("thumb read");
            let blob = blob.unwrap_or_else(|| panic!("asset {} has no 256px thumb", card.id));
            assert!(blob.starts_with(&[0xFF, 0xD8]), "thumb is not a JPEG");
        }

        // The scaffolding is find-or-create: a re-run reuses the six
        // personas and the whole pool table rather than minting a second
        // set. Since V61 the *assets* are too — a second full seed
        // re-arrives at locators the library already holds, the ingest
        // lookup answers, and nothing is minted. It used to collide on
        // `idx_asset_source_unique` and say so, which is why
        // `just bench-reset` exists; that reset is now about starting
        // from a known size, not about getting past a refusal.
        let personas_again = ensure_personas(&personas).await.expect("personas again");
        assert_eq!(personas_again.len(), crate::model::PERSONA_COUNT);
        assert_eq!(personas.list().await.expect("personas").len(), 6);
        let groups_again = ensure_groups(&groups, persona0.id, 42)
            .await
            .expect("groups again");
        assert_eq!(groups_again.len(), group_pools(42).len());
        assert_eq!(
            groups.list(Some(&persona0.id)).await.expect("groups").len(),
            group_pools(42).len()
        );

        driver.shutdown().await.expect("shutdown");
    }
}
