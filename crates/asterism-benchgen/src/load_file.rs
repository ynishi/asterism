//! `load-file` — the file tier (T-file), pushed through the real HTTP
//! surface of a running bench server.
//!
//! The S / M presets exist to measure the *import* path: hashing every
//! byte and generating thumbnails is what an import costs, and it is the
//! number the reference workload (5,000 files ≈ 20 minutes) is stated in.
//! So this command deliberately does **not** write rows the way
//! [`crate::seed_meta`] does — it posts to `asterism-server` and lets the
//! same jobs the app runs do the work.
//!
//! ## Why five passes
//!
//! `AddAssetCommand` carries persona / locator / modality / labels and
//! nothing else of what the cardinality model produces: tags, groups and
//! ratings are separate verbs, and there is no combined endpoint.
//! So one corpus becomes:
//!
//! 1. personas — find-or-create by name
//! 2. `POST /asterism/assets/add-batch` (batched, [`BATCH_SIZE`])
//! 3. `POST /asterism/tags/attach-batch`
//! 4. `POST /asterism/groups/create` + `/asterism/groups/batch-membership`
//! 5. `POST /asterism/assets/update-meta-batch` (rating)
//! 6. `POST /asterism/assets/trash` per trashed asset (2 % of the corpus)
//!
//! ## Which server
//!
//! The bench profile serves on 28989. Port 8989 is Dogfood — the user's
//! real library — and [`guard_server`] refuses it outright, because
//! "point the loader at the wrong port" and "add 12,000 synthetic assets
//! to the library you actually use" are the same keystroke.
//!
//! ## What a re-run does
//!
//! Nothing clean. `(source_kind, locator)` is unique, so a second run
//! reports every item as failed rather than duplicating it — v1's
//! operating model is one load into a freshly reset bench profile
//! (`just bench-reset`), and the failure count in the report is how a
//! re-run announces itself.

use anyhow::{Context, Result, anyhow, bail};
use asterism_contract::command::{
    AddAssetBatchCommand, AddAssetBatchResult, AddAssetCommand, AttachTagBatchCommand,
    AttachTagBatchResult, AttachTagCommand, BatchGroupMembershipCommand, CreateGroupCommand,
    GroupMembershipEntry, RegisterPersonaCommand, TrashAssetCommand, UpdateAssetMetaBatchCommand,
    UpdateAssetMetaBatchResult, UpdateAssetMetaCommand,
};
use asterism_contract::dto::{GroupDto, PersonaDto};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::manifest::Manifest;
use crate::model::{AssetSpec, SpecStream};
use crate::seed_meta::{BENCH_MODALITY, INBOX_LABEL, locator_of, persona_name};

/// Default bench server (`DataProfile::Bench::default_http_port`).
pub const DEFAULT_SERVER: &str = "http://127.0.0.1:28989";
/// The port the Dogfood app serves on. Never a load target.
pub const DOGFOOD_PORT: u16 = 8989;
/// Items per `add-batch` / `attach-batch` / `update-meta-batch` request.
///
/// Large enough that the per-request overhead disappears against the
/// per-item work, small enough that one failed request costs a bounded
/// amount of progress and the JSON body stays in the low MB.
pub const BATCH_SIZE: usize = 200;

/// Everything `load-file` needs; the CLI layer builds it.
#[derive(Debug, Clone)]
pub struct LoadFileArgs {
    /// Corpus identity.
    pub seed: u64,
    /// Preset label (`s` / `m`).
    pub preset: &'static str,
    /// How many assets to load.
    pub count: u64,
    /// Corpus directory holding `manifest-<preset>.json` and `files/`.
    pub corpus_dir: PathBuf,
    /// Base URL of the running bench server.
    pub server: String,
    /// Opt out of the Dogfood-port refusal. Exists so a non-default
    /// deployment is reachable at all; it does not make 8989 a target.
    pub allow_any_server: bool,
}

/// What one load put into the server.
///
/// Serialisable because `measure-import` writes it into the run's
/// result file as the *registration* half of
/// the import cost — the half that finishes when the server has
/// accepted every row, as opposed to the drain half that finishes when
/// the jobs those rows enqueued are done.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadReport {
    /// Assets accepted by `add-batch`.
    pub added: u64,
    /// Items `add-batch` rejected (a re-run reports every item here).
    pub add_failed: u64,
    /// Tag links accepted.
    pub tag_links: u64,
    /// Tag links rejected.
    pub tag_failed: u64,
    /// Groups that existed or were created.
    pub groups: u64,
    /// Membership pairs sent.
    pub group_pairs: u64,
    /// Ratings written.
    pub rated: u64,
    /// Ratings rejected.
    pub rating_failed: u64,
    /// Assets moved to the trash.
    pub trashed: u64,
    /// Wall-clock duration.
    pub elapsed_ms: u64,
    /// Per-pass wall clock, in the order the passes run. A single
    /// total cannot say whether an import spent its time hashing
    /// assets or attaching 15,000 tag links one batch at a time, and
    /// the reference workload (5,000 files ≈ 20 minutes) is a number
    /// about the first pass only.
    pub personas_ms: u64,
    pub add_ms: u64,
    pub tag_ms: u64,
    pub group_ms: u64,
    pub rating_ms: u64,
    pub trash_ms: u64,
}

/// Rejects a load target that is not a bench server.
///
/// Split from the run so the refusal is testable without a socket: it is
/// the only thing standing between a mistyped port and 12,000 synthetic
/// assets in the user's real library.
pub fn guard_server(url: &str, allow_any_server: bool) -> Result<()> {
    let port = port_of(url)
        .ok_or_else(|| anyhow!("cannot read a port out of {url:?}; pass an explicit host:port"))?;
    if port == DOGFOOD_PORT && !allow_any_server {
        bail!(
            "refusing to load into {url}: port {DOGFOOD_PORT} is the Dogfood app — the real \
             library. The bench server listens on 28989 ({DEFAULT_SERVER})"
        );
    }
    Ok(())
}

/// Port of an `http(s)://host:port[/...]` URL. Returns `None` when the
/// URL names no port, which is deliberately an error upstream: an
/// implicit 80 / 443 tells us nothing about which profile answers.
fn port_of(url: &str) -> Option<u16> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split('/').next()?;
    let host_port = authority.rsplit('@').next()?;
    let port = host_port.rsplit(':').next()?;
    port.parse().ok()
}

/// Checks that the corpus on disk is the one being asked for, and that
/// every file the load is about to name exists.
///
/// A partial corpus is the failure this prevents: `add-batch` accepts a
/// locator whether or not the file is there, so a missing half would
/// land as rows that hash-fail asynchronously — an import bench measured
/// on a corpus that was never fully generated, with nothing in the
/// number to say so.
pub fn verify_corpus(args: &LoadFileArgs) -> Result<Manifest> {
    let manifest_path = args
        .corpus_dir
        .join(format!("manifest-{}.json", args.preset));
    let text = std::fs::read_to_string(&manifest_path).with_context(|| {
        format!(
            "cannot read {} — generate the corpus first (just bench-corpus {})",
            manifest_path.display(),
            args.preset
        )
    })?;
    let manifest: Manifest = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a benchgen manifest", manifest_path.display()))?;
    if !manifest.covers(args.seed, args.preset, args.count) {
        bail!(
            "{} holds seed={} generator={} preset={} count={}, which does not cover \
             seed={} preset={} count={}",
            manifest_path.display(),
            manifest.seed,
            manifest.generator_version,
            manifest.preset,
            manifest.count,
            args.seed,
            args.preset,
            args.count
        );
    }

    let missing: Vec<String> = SpecStream::new(args.seed)
        .take(args.count as usize)
        .filter(|spec| !args.corpus_dir.join(&spec.rel_path).is_file())
        .map(|spec| spec.rel_path)
        .collect();
    if !missing.is_empty() {
        let shown: Vec<&String> = missing.iter().take(10).collect();
        bail!(
            "{} files are missing from {} (first: {:?}) — the corpus is partial; \
             re-run the generator before loading it",
            missing.len(),
            args.corpus_dir.display(),
            shown
        );
    }
    Ok(manifest)
}

/// The `add-batch` item for one spec.
///
/// Kept as a pure function so the request shape is testable without a
/// server: the attribution triple is left unset (this loader states
/// nothing about who anything is by), and `labels` mirrors the metadata
/// tier — spec labels plus the `inbox` the service path adds on ingest.
pub fn add_item(
    spec: &AssetSpec,
    corpus_dir: &Path,
    persona_id: &str,
    size: u64,
) -> AddAssetCommand {
    let mut labels = spec.labels.clone();
    labels.push(INBOX_LABEL.to_string());
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: "fs".to_string(),
        locator: locator_of(corpus_dir, spec),
        modality: Some(BENCH_MODALITY.to_string()),
        occurred_at_ms: spec.occurred_at_ms,
        session_id: None,
        external_session_key: None,
        // The corpus states no outside name for a row it invented.
        external_key: None,
        bundle_id: None,
        labels,
        register_note: None,
        platform: None,
        file_size_bytes: Some(size),
        duration_ms: None,
        // Left unstated even though this loader **knows** them — the
        // generator picked the canvas (`image_synth`) and the spec is
        // still holding it. Passing them would give the bench corpus
        // non-null dimensions cheaply; not doing it keeps the corpus
        // shaped like a library nobody has measured, which is what the
        // metadata tier is for. Wiring them is its own small change.
        width_px: None,
        height_px: None,
        extra_json: None,
        cover_hint: None,
        auto_organize_base_dir: None,
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        // The loader declares no duplicate strategy and asserts no
        // digest: a benchmark corpus is loaded to be measured, and both
        // fields would put a claim about the run into the rows it
        // produces.
        on_duplicate: None,
        declared_content_hash: None,
        album_meta: Default::default(),
    }
}

/// Splits a slice into [`BATCH_SIZE`]-sized requests.
///
/// A named function rather than an inline `chunks` call because the
/// index arithmetic that maps a batch response back onto the specs it
/// came from is what makes tags / groups / ratings land on the right
/// assets; the boundary case is worth a test.
pub fn batches<T>(items: &[T]) -> impl Iterator<Item = &[T]> {
    items.chunks(BATCH_SIZE.max(1))
}

/// Thin HTTP client over the endpoints this loader needs.
///
/// The importer SDK's `ApiClient` covers `add-batch` and health but none
/// of the other four passes (tags / groups / meta / trash), so this is a
/// sibling on the same crate and the same feature set rather than a
/// second HTTP stack.
///
/// Shared with [`crate::measure`] for the same reason: one client for
/// the crate, so "which server, decoded how, failing with what
/// message" has one answer.
pub struct Api {
    base_url: String,
    inner: reqwest::Client,
}

impl Api {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            inner: reqwest::Client::new(),
        }
    }

    /// `path` carries any query string — the caller builds it.
    pub async fn get<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .inner
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;
        Self::decode(resp, &url).await
    }

    async fn post<B: Serialize, R: DeserializeOwned>(&self, path: &str, body: &B) -> Result<R> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .inner
            .post(&url)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url} failed"))?;
        Self::decode(resp, &url).await
    }

    async fn decode<R: DeserializeOwned>(resp: reqwest::Response, url: &str) -> Result<R> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("{url}: HTTP {status}: {body}");
        }
        resp.json::<R>()
            .await
            .with_context(|| format!("{url}: response decode failed"))
    }
}

/// Loads the corpus into the server named by `args`.
pub async fn run(args: LoadFileArgs) -> Result<LoadReport> {
    guard_server(&args.server, args.allow_any_server)?;
    let manifest = verify_corpus(&args)?;
    eprintln!(
        "benchgen: load-file seed={} preset={} count={} server={} corpus={} (generator={})",
        args.seed,
        args.preset,
        args.count,
        args.server,
        args.corpus_dir.display(),
        manifest.generator_version
    );

    let started = Instant::now();
    let api = Api::new(&args.server);
    let mut report = LoadReport::default();

    let persona_ids = ensure_personas(&api).await?;
    let specs: Vec<AssetSpec> = SpecStream::new(args.seed)
        .take(args.count as usize)
        .collect();
    report.personas_ms = started.elapsed().as_millis() as u64;
    let mut pass_start = Instant::now();

    // Pass 1 — the assets themselves. `asset_ids[i]` is the id of
    // `specs[i]`, or `None` when the server rejected that item; every
    // later pass skips the `None`s rather than shifting into them.
    let mut asset_ids: Vec<Option<String>> = Vec::with_capacity(specs.len());
    for batch in batches(&specs) {
        let items = batch
            .iter()
            .map(|spec| {
                let path = args.corpus_dir.join(&spec.rel_path);
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                add_item(spec, &args.corpus_dir, &persona_ids[spec.persona_idx], size)
            })
            .collect();
        let result: AddAssetBatchResult = api
            .post(
                "/asterism/assets/add-batch",
                &AddAssetBatchCommand {
                    items,
                    auto_organize_base_dir: None,
                },
            )
            .await?;
        for (idx, id) in result.succeeded.iter().enumerate() {
            if id.is_empty() {
                asset_ids.push(None);
                if let Some(reason) = result.failed.get(idx) {
                    eprintln!("benchgen: add rejected: {reason}");
                }
            } else {
                asset_ids.push(Some(id.clone()));
            }
        }
        report.added += result.success_count;
        report.add_failed += result.failure_count;
        eprintln!(
            "benchgen: added {}/{} ({:.1}s)",
            report.added,
            args.count,
            started.elapsed().as_secs_f64()
        );
    }
    if report.add_failed > 0 {
        eprintln!(
            "benchgen: WARNING {} items were rejected — a partially loaded profile is not a \
             corpus. Reset the bench profile and load once.",
            report.add_failed
        );
    }

    report.add_ms = pass_start.elapsed().as_millis() as u64;
    pass_start = Instant::now();

    // Pass 2 — tags.
    let tag_items: Vec<AttachTagCommand> = specs
        .iter()
        .zip(&asset_ids)
        .filter_map(|(spec, id)| id.as_ref().map(|id| (spec, id)))
        .flat_map(|(spec, id)| {
            spec.tags.iter().map(move |name| AttachTagCommand {
                asset_id: id.clone(),
                name: name.clone(),
            })
        })
        .collect();
    for batch in batches(&tag_items) {
        let result: AttachTagBatchResult = api
            .post(
                "/asterism/tags/attach-batch",
                &AttachTagBatchCommand {
                    items: batch.to_vec(),
                },
            )
            .await?;
        report.tag_links += result.success_count;
        report.tag_failed += result.failure_count;
    }

    report.tag_ms = pass_start.elapsed().as_millis() as u64;
    pass_start = Instant::now();

    // Pass 3 — groups, all under persona 0 (the model files nobody else).
    let group_ids = ensure_groups(&api, &persona_ids[0], args.seed).await?;
    report.groups = group_ids.len() as u64;
    let mut attach: Vec<GroupMembershipEntry> = Vec::new();
    for (spec, id) in specs.iter().zip(&asset_ids) {
        let Some(id) = id.as_ref() else { continue };
        for name in &spec.groups {
            let group_id = group_ids
                .get(name)
                .ok_or_else(|| anyhow!("spec names group {name}, which no pool created"))?;
            attach.push(GroupMembershipEntry {
                asset_id: id.clone(),
                group_id: group_id.clone(),
            });
        }
    }
    for batch in batches(&attach) {
        let _: serde_json::Value = api
            .post(
                "/asterism/groups/batch-membership",
                &BatchGroupMembershipCommand {
                    attach: batch.to_vec(),
                    detach: Vec::new(),
                },
            )
            .await?;
        report.group_pairs += batch.len() as u64;
    }

    report.group_ms = pass_start.elapsed().as_millis() as u64;
    pass_start = Instant::now();

    // Pass 4 — ratings.
    let rated: Vec<UpdateAssetMetaCommand> = specs
        .iter()
        .zip(&asset_ids)
        .filter_map(|(spec, id)| match (spec.rating, id) {
            (Some(rating), Some(id)) => Some(UpdateAssetMetaCommand {
                asset_id: id.clone(),
                labels: None,
                register_note: None,
                cover: None,
                title: None,
                rating: Some(rating),
                modality: None,
                bundle_id: None,
            }),
            _ => None,
        })
        .collect();
    for batch in batches(&rated) {
        let result: UpdateAssetMetaBatchResult = api
            .post(
                "/asterism/assets/update-meta-batch",
                &UpdateAssetMetaBatchCommand {
                    items: batch.to_vec(),
                },
            )
            .await?;
        report.rated += result.success_count;
        report.rating_failed += result.failure_count;
    }

    report.rating_ms = pass_start.elapsed().as_millis() as u64;
    pass_start = Instant::now();

    // Pass 5 — trash. 2 % of the corpus and no batch verb, so per-asset.
    for (spec, id) in specs.iter().zip(&asset_ids) {
        let (true, Some(id)) = (spec.trashed, id.as_ref()) else {
            continue;
        };
        let _: serde_json::Value = api
            .post(
                "/asterism/assets/trash",
                &TrashAssetCommand {
                    asset_id: id.clone(),
                    comment: None,
                },
            )
            .await?;
        report.trashed += 1;
    }

    report.trash_ms = pass_start.elapsed().as_millis() as u64;
    report.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

/// Find-or-create the six bench personas over HTTP, returned by index.
async fn ensure_personas(api: &Api) -> Result<Vec<String>> {
    let existing: Vec<PersonaDto> = api.get("/asterism/personas").await?;
    let mut ids = Vec::with_capacity(crate::model::PERSONA_COUNT);
    for idx in 0..crate::model::PERSONA_COUNT {
        let name = persona_name(idx);
        match existing.iter().find(|p| p.name == name) {
            Some(found) => ids.push(found.id.clone()),
            None => {
                let created: PersonaDto = api
                    .post(
                        "/asterism/personas/register",
                        &RegisterPersonaCommand {
                            name: name.clone(),
                            pack_id: None,
                        },
                    )
                    .await?;
                ids.push(created.id);
            }
        }
    }
    Ok(ids)
}

/// Find-or-create the pool table under one persona, name → id.
///
/// `GET /asterism/groups` is the find half; create is only attempted for
/// names it did not return, because `(persona_id, name)` is unique and a
/// blind create would fail the whole load on the second run.
async fn ensure_groups(api: &Api, persona_id: &str, seed: u64) -> Result<BTreeMap<String, String>> {
    let existing: Vec<asterism_contract::dto::GroupSummaryDto> = api
        .get(&format!("/asterism/groups?persona_id={persona_id}"))
        .await?;
    let mut ids: BTreeMap<String, String> = existing
        .into_iter()
        .map(|s| (s.group.name, s.group.id))
        .collect();

    for pool in crate::model::group_pools(seed) {
        if ids.contains_key(&pool.name) {
            continue;
        }
        let created: GroupDto = api
            .post(
                "/asterism/groups/create",
                &CreateGroupCommand {
                    persona_id: persona_id.to_string(),
                    name: pool.name.clone(),
                    description: None,
                },
            )
            .await?;
        ids.insert(pool.name, created.id);
    }
    Ok(ids)
}

/// Human-readable completion line.
pub fn report_line(report: &LoadReport) -> String {
    format!(
        "benchgen: loaded {} assets ({} rejected) / {} tag links ({} failed) / \
         {} groups ({} membership pairs) / {} rated ({} failed) / {} trashed in {} ms",
        report.added,
        report.add_failed,
        report.tag_links,
        report.tag_failed,
        report.groups,
        report.group_pairs,
        report.rated,
        report.rating_failed,
        report.trashed,
        report.elapsed_ms
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ManifestBuilder;

    fn args(dir: &Path) -> LoadFileArgs {
        LoadFileArgs {
            seed: 42,
            preset: "s",
            count: 4,
            corpus_dir: dir.to_path_buf(),
            server: DEFAULT_SERVER.to_string(),
            allow_any_server: false,
        }
    }

    #[test]
    fn the_dogfood_port_is_not_a_load_target() {
        let err = guard_server("http://127.0.0.1:8989", false).expect_err("8989 must be refused");
        assert!(err.to_string().contains("Dogfood"), "{err}");
        assert!(
            guard_server("http://127.0.0.1:8989", true).is_ok(),
            "the explicit override exists for non-default deployments"
        );
        assert!(guard_server(DEFAULT_SERVER, false).is_ok());
        assert!(guard_server("http://127.0.0.1:18989", false).is_ok());
        // No port at all: refused rather than guessed, because an
        // implicit 80 says nothing about which profile answers.
        assert!(guard_server("http://asterism.local", false).is_err());
        assert_eq!(port_of("http://127.0.0.1:28989/x"), Some(28_989));
    }

    #[test]
    fn batches_split_on_the_declared_boundary() {
        let items: Vec<u32> = (0..BATCH_SIZE as u32 * 2 + 1).collect();
        let sizes: Vec<usize> = batches(&items).map(<[u32]>::len).collect();
        assert_eq!(sizes, vec![BATCH_SIZE, BATCH_SIZE, 1]);
        // Every item appears exactly once and in order — the property
        // the response-index mapping depends on.
        let flat: Vec<u32> = batches(&items).flatten().copied().collect();
        assert_eq!(flat, items);
        assert_eq!(batches::<u32>(&[]).count(), 0);
    }

    #[test]
    fn a_request_item_is_a_function_of_its_spec() {
        let spec = SpecStream::new(42).next().expect("infinite");
        let a = add_item(&spec, Path::new("/corpus"), "persona-0", 1_234);
        let b = add_item(&spec, Path::new("/corpus"), "persona-0", 1_234);
        assert_eq!(
            serde_json::to_string(&a).expect("json"),
            serde_json::to_string(&b).expect("json")
        );
        assert_eq!(a.locator, format!("/corpus/{}", spec.rel_path));
        assert_eq!(a.source_kind, "fs");
        assert_eq!(a.modality.as_deref(), Some(BENCH_MODALITY));
        assert_eq!(a.occurred_at_ms, spec.occurred_at_ms);
        assert_eq!(a.file_size_bytes, Some(1_234));
        assert_eq!(a.labels.last().map(String::as_str), Some(INBOX_LABEL));
        assert_eq!(a.labels.len(), spec.labels.len() + 1);
        // This loader states nothing about who anything is by; the
        // batch handler rejects a batch whose items disagree, and
        // "unrecorded" is the honest answer for synthetic data.
        assert!(a.author_kind.is_none() && a.author_subject.is_none() && a.operator_ai.is_none());
    }

    #[test]
    fn a_partial_corpus_is_refused_before_anything_is_posted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let args = args(dir.path());

        // No manifest at all.
        let err = verify_corpus(&args).expect_err("missing manifest");
        assert!(err.to_string().contains("bench-corpus"), "{err}");

        // Manifest present, files absent.
        let mut builder = ManifestBuilder::new(42, "s");
        let specs: Vec<AssetSpec> = SpecStream::new(42).take(4).collect();
        for spec in &specs {
            builder.observe(spec, Some(1_000_000));
        }
        let manifest = builder.finish(1);
        std::fs::write(
            dir.path().join("manifest-s.json"),
            serde_json::to_string(&manifest).expect("json"),
        )
        .expect("write manifest");
        let err = verify_corpus(&args).expect_err("missing files");
        assert!(err.to_string().contains("missing"), "{err}");

        // Files present: accepted.
        std::fs::create_dir_all(dir.path().join("files")).expect("files dir");
        for spec in &specs {
            std::fs::write(dir.path().join(&spec.rel_path), b"not really a png").expect("write");
        }
        verify_corpus(&args).expect("a complete corpus is accepted");

        // A manifest for a different seed does not stand in for this one.
        let mismatched = LoadFileArgs {
            seed: 43,
            ..args.clone()
        };
        assert!(verify_corpus(&mismatched).is_err());
        // Nor does one that covers fewer assets than asked for.
        let longer = LoadFileArgs { count: 5, ..args };
        assert!(verify_corpus(&longer).is_err());
    }
}
