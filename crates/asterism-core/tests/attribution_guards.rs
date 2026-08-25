//! Scans that hold the parts of the attribution rule no type can.
//!
//! Two shapes are already closed by the compiler:
//! a service mutation cannot be written without receiving an
//! `AttributionContext`, and a context cannot be built without naming
//! the channel it arrived through. What the compiler cannot say is
//! *which* constructor a given crate is entitled to name, and whether a
//! newly added service verb joined the set that has to receive one at
//! all. Both are read off the source text here.
//!
//! # Reading source as text
//!
//! Every scan below drops whole-line comments (`//`, `///`, `//!`) and
//! reads the rest verbatim. Two consequences are worth knowing before
//! writing in the scanned files:
//!
//! - a trailing comment on a line of code is read *as* code, so a
//!   forbidden token named there fails the guard. Put such mentions on
//!   their own comment line (which is how the mentions in this file's
//!   own subjects are written).
//! - block comments (`/* … */`) are not understood. None of the scanned
//!   trees contains one.
//!
//! Reading text rather than a syntax tree is the trade this file makes:
//! a scan cannot be fooled by a shape the type system was going to
//! allow, and it costs no build dependency, but it sees names rather
//! than meanings.
//!
//! # Why every negative guard carries a positive anchor
//!
//! A scan for something that must *not* appear passes just as happily
//! when it is pointed at nothing at all — a renamed directory, a moved
//! module, an empty file list. Absence and blindness produce the same
//! green. Each guard below therefore also asserts a token that must be
//! present in the same corpus, so the two stay distinguishable.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- corpus

/// The workspace root, derived from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/asterism-core sits two levels below the workspace root")
        .to_path_buf()
}

/// Every `.rs` file under `dir`, sorted, failing loudly if the
/// directory itself has moved (an empty corpus is the failure mode the
/// positive anchors exist for, and a missing directory is its cause).
fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    assert!(
        dir.is_dir(),
        "scan target is missing — did the tree move? {}",
        dir.display()
    );
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries =
            fs::read_dir(&current).unwrap_or_else(|e| panic!("read {}: {e}", current.display()));
        for entry in entries {
            let path = entry.expect("readable directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Numbered source lines with whole-line comments removed.
fn code_lines(path: &Path) -> Vec<(usize, String)> {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .map(|(index, line)| (index + 1, line.to_string()))
        .collect()
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Every `path:line` where `token` occurs, ignoring occurrences that
/// carry one of `exempt` immediately in front of them.
fn occurrences(root: &Path, files: &[PathBuf], token: &str, exempt: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for path in files {
        for (number, line) in code_lines(path) {
            let mut scanned = line;
            for prefix in exempt {
                scanned = scanned.replace(&format!("{prefix}{token}"), "");
            }
            if scanned.contains(token) {
                found.push(format!("{}:{number}", relative(root, path)));
            }
        }
    }
    found
}

// ------------------------------------------------- negative guards (adapters)

#[test]
fn the_server_crate_cannot_speak_for_the_owners_surface() {
    let root = workspace_root();
    let files = rust_sources(&root.join("crates/asterism-server/src"));

    // Positive anchor: the HTTP and MCP surfaces exist to turn a
    // caller's stated fields into an assertion. A corpus without that
    // is not the adapter, and the guard below would be measuring
    // nothing.
    assert!(
        !occurrences(&root, &files, "asserted(", &[]).is_empty(),
        "the server crate translates caller-stated attribution; finding \
         no such translation means this scan is not looking at it"
    );

    let claimed = occurrences(&root, &files, "owner_surface(", &[]);
    assert!(
        claimed.is_empty(),
        "this crate serves remote callers, whose attribution is a claim \
         and nothing more. Owner-ness is a property of the desktop app's \
         own IPC, so naming that channel here would let an HTTP request \
         write a row indistinguishable from the owner's own: {claimed:?}"
    );
}

#[test]
fn the_tauri_crate_cannot_assert_an_attribution() {
    let root = workspace_root();
    // The Rust side literally: the Svelte half of this crate contains
    // no constructor calls at all, so scanning it would be green by
    // vacancy.
    let files = rust_sources(&root.join("crates/asterism-ui/src-tauri/src"));

    // Positive anchor: every mutation command names the owner's surface.
    assert!(
        !occurrences(&root, &files, "owner_surface(", &[]).is_empty(),
        "the desktop commands name the owner's surface; finding none \
         means this scan is not looking at them"
    );

    let asserted = occurrences(&root, &files, "asserted(", &[]);
    assert!(
        asserted.is_empty(),
        "a request that arrived here arrived through the owner's own \
         controls; treating it as a self-assertion would throw away the \
         one thing this surface knows for certain: {asserted:?}"
    );
}

#[test]
fn no_adapter_crate_rebuilds_a_stored_attribution() {
    let root = workspace_root();
    let mut files = rust_sources(&root.join("crates/asterism-server/src"));
    files.extend(rust_sources(&root.join("crates/asterism-ui/src-tauri/src")));

    // Positive anchor and the one exemption in one: `Author::from_columns`
    // reads the *wire* pair a command carries, which is exactly what an
    // adapter is for. It cannot carry a channel — `PersistedAttribution`
    // is the type that can, and that one is a database fact.
    assert!(
        !occurrences(&root, &files, "Author::from_columns(", &[]).is_empty(),
        "an adapter reads the author pair off the command; finding no \
         such read means this scan is not looking at the adapters"
    );

    let restored = occurrences(&root, &files, "from_columns(", &["Author::"]);
    assert!(
        restored.is_empty(),
        "`PersistedAttribution::from_columns` is public so the repository \
         can read a row back; a triple assembled anywhere else is a claim \
         wearing a fact's type, and it would carry any channel it liked: \
         {restored:?}"
    );

    let reified = occurrences(&root, &files, "from_persisted(", &[]);
    assert!(
        reified.is_empty(),
        "restoring an attribution from stored columns belongs to the two \
         paths that have columns to restore from — the repository's \
         hydration and the dispatch reify — both inside asterism-core: \
         {reified:?}"
    );
}

#[test]
fn no_command_carries_the_channel_inward() {
    let root = workspace_root();
    let path = root.join("crates/asterism-contract/src/command.rs");

    let mut structs: Vec<String> = Vec::new();
    let mut carrying: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for (number, line) in code_lines(&path) {
        if let Some(tail) = line.split("pub struct ").nth(1) {
            let name: String = tail
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.ends_with("Command") {
                structs.push(name.clone());
                current = Some(name);
            }
        }
        if current.is_none() {
            continue;
        }
        if line == "}" {
            current = None;
            continue;
        }
        let trimmed = line.trim();
        let field = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
        if field.starts_with("via:")
            || field.starts_with("attributed_via:")
            || field.contains("AttributionChannel")
            || field.contains("AttributionContext")
        {
            let command = current.as_deref().unwrap_or("<unknown>");
            carrying.push(format!("{command} (command.rs:{number})"));
        }
    }

    assert!(
        structs.iter().any(|name| name == "AddAssetCommand"),
        "the command surface should be in this file; scanning it found \
         {} command structs and not the one every write path uses",
        structs.len()
    );
    assert!(
        carrying.is_empty(),
        "the channel is derived from which entry point ran, never stated. \
         It travels outward on `AssetDto.attributed_via` and nowhere \
         inward — a command field would make it assertable and undo the \
         one distinction authentication will need: {carrying:?}"
    );
}

// ------------------------------------------------- the receiving population

/// A `pub fn` / `pub async fn` read off the source.
#[derive(Debug)]
struct PublicFn {
    file: String,
    module: String,
    name: String,
    line: usize,
    is_async: bool,
    args: String,
}

impl PublicFn {
    /// `module::fn` — the qualified pair the allowlists are keyed by.
    /// A bare function name would be ambiguous across services, and an
    /// ambiguous key silently exempts every namesake: `create`,
    /// `delete` and `list` each name three or more different verbs in
    /// this layer.
    fn key(&self) -> String {
        format!("{}::{}", self.module, self.name)
    }

    fn site(&self) -> String {
        format!("{} ({}:{})", self.key(), self.file, self.line)
    }
}

/// Reads the public functions out of one file, stopping at the file's
/// own `#[cfg(test)]` module (recognised by sitting at column zero —
/// the `#[cfg(test)]` that gates a single item is indented).
fn public_fns(root: &Path, path: &Path) -> Vec<PublicFn> {
    let module = path
        .file_stem()
        .expect("a source file has a stem")
        .to_string_lossy()
        .to_string();
    let file = relative(root, path);
    let lines = code_lines(path);
    let end = lines
        .iter()
        .position(|(_, line)| line.trim_end() == "#[cfg(test)]" && !line.starts_with(' '))
        .unwrap_or(lines.len());

    let mut found = Vec::new();
    for (offset, (number, line)) in lines[..end].iter().enumerate() {
        let trimmed = line.trim_start();
        let (is_async, tail) = if let Some(tail) = trimmed.strip_prefix("pub async fn ") {
            (true, tail)
        } else if let Some(tail) = trimmed.strip_prefix("pub fn ") {
            (false, tail)
        } else {
            continue;
        };
        let name: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();

        // The argument list, gathered by parenthesis depth so that a
        // signature broken over several lines reads the same as one on
        // a single line.
        let mut args = String::new();
        let mut depth = 0usize;
        let mut started = false;
        'signature: for (_, text) in &lines[offset..] {
            for ch in text.chars() {
                if ch == '(' {
                    depth += 1;
                    started = true;
                    if depth == 1 {
                        continue;
                    }
                } else if ch == ')' {
                    depth -= 1;
                    if depth == 0 {
                        break 'signature;
                    }
                }
                if started {
                    args.push(ch);
                }
            }
            args.push(' ');
        }

        found.push(PublicFn {
            file: file.clone(),
            module: module.clone(),
            name,
            line: *number,
            is_async,
            args,
        });
    }
    found
}

/// `module::fn` pairs in `application/` that take no `AttributionContext`,
/// each with the reason it is not a write that gets attributed.
///
/// Three kinds of thing live here and nothing else should: reads,
/// writes of something *derived* from a
/// record rather than of the record itself, and sweeps. A verb that
/// writes an entity belongs on the other side of this list — and when
/// it is genuinely unclear, the cheap mistake is to take the argument
/// and ignore it, not to be exempted and forget.
///
/// The length of this list is a reading of the layer, not an
/// inconvenience to be managed: a new entry is a new verb that writes
/// without saying whose write it is.
const CONTEXT_EXEMPT: &[(&str, &str)] = &[
    // ---- line_service: reads (#63)
    (
        "line_service::get",
        "read — one line, history included, because the rules the model \
         holds are about the chain",
    ),
    (
        "line_service::states",
        "read — what is on the line, folded out of its history on the spot",
    ),
    (
        "line_service::strategies",
        "read — the rules a line can be pointed at, built from the rules \
         this instance carries so that somebody can choose one",
    ),
    (
        "line_service::list",
        "read — every line there is, unscoped, because a line carries \
         no owner for this layer to scope by",
    ),
    // ---- thread_service: reads (#102)
    (
        "thread_service::get",
        "read — one conversation whole, every correction to every \
         message included",
    ),
    (
        "thread_service::about",
        "read — every conversation hanging off one thing, which is \
         more than one because two people can start separate ones",
    ),
    // ---- pursuit_service: reads (#63)
    (
        "pursuit_service::get",
        "read — one line of work with every pass on it",
    ),
    (
        "pursuit_service::collisions",
        "read — what this work writes that the line has moved since, \
         derived from the two logs on every call and stored nowhere",
    ),
    (
        "pursuit_service::of_line",
        "read — every piece of work against a line, ended work included",
    ),
    (
        "pursuit_service::children",
        "read — the work filed under a larger piece of work, one level",
    ),
    (
        "pursuit_service::behind",
        "read — what the line has recorded since this work was cut from it",
    ),
    // ---- provenance_service
    (
        "disclosure_service::record_for",
        "read — assembles what an asset discloses out of rows it only reads",
    ),
    (
        "disclosure_service::apply_to",
        "writes a file, not a row — the artefact it stamps is outside the \
         library (an export's copy, or something returned from downstream), \
         and no aggregate here changes. What made the asset is the \
         attribution this would carry, and that is already recorded on the \
         asset; re-stating it as the author of the stamping would attribute \
         the file to whoever re-applied a disclosure they did not make",
    ),
    // ---- asset_service: reads
    ("asset_service::list", "read — one page of the grid"),
    (
        "asset_service::tag_suggestions_of",
        "read — what the bound model proposed, rulings included; the \
         rulings themselves go through accept/reject, which carry a \
         context",
    ),
    (
        "asset_service::visual_model_status",
        "read — which model this process bound, if any",
    ),
    ("asset_service::list_index", "read — the light index page"),
    (
        "asset_service::hydrate_cards",
        "read — fills cards for ids the index already returned",
    ),
    ("asset_service::search", "read — full-text query"),
    (
        "asset_service::search_ids",
        "read — rank order hint (ids only)",
    ),
    ("asset_service::sample", "read — random picks"),
    ("asset_service::list_sessions", "read — session listing"),
    ("asset_service::list_tag_counts", "read — sidebar counts"),
    (
        "asset_service::list_persona_asset_counts",
        "read — sidebar counts",
    ),
    (
        "asset_service::list_modality_asset_counts",
        "read — sidebar counts",
    ),
    (
        "asset_service::list_format_asset_counts",
        "read — sidebar counts",
    ),
    (
        "asset_service::list_color_asset_counts",
        "read — sidebar counts",
    ),
    (
        "asset_service::list_duplicate_groups",
        "read — content-hash duplicates",
    ),
    (
        "asset_service::list_duplicate_conflicts",
        "read — the duplicate questions still open",
    ),
    ("asset_service::list_groups", "read — group listing"),
    (
        "asset_service::list_group_links",
        "read — group link listing",
    ),
    ("asset_service::list_dirs", "read — dir listing"),
    (
        "asset_service::groups_of_asset",
        "read — the groups one asset is filed in",
    ),
    (
        "asset_service::asset_texts",
        "read — the indexed text of an asset",
    ),
    (
        "asset_service::assert_visible",
        "read — a visibility check, and the answer to it",
    ),
    ("asset_service::detail", "read — the detail view"),
    (
        "asset_service::original_file",
        "read — resolves the original's path for serving",
    ),
    ("asset_service::edges_of", "read — constellation edges"),
    (
        "asset_service::constellation_of",
        "read — the neighbourhood of one asset",
    ),
    (
        "asset_service::provenance_of",
        "read — the recorded provenance claim",
    ),
    ("asset_service::lineage_of", "read — the derivation chain"),
    // ---- asset_service: derived artefacts and sweeps
    (
        "asset_service::video_preview",
        "reports where a transcoded rendition stands and enqueues one \
         when it is missing; the rendition is derived from the original \
         and belongs to no author",
    ),
    (
        "asset_service::enqueue_thumb_gen",
        "hands a thumbnail render to the job engine; the cached blob is \
         derived from the asset, not a record anyone wrote",
    ),
    (
        "asset_service::rebuild_edges",
        "enqueues an edge recomputation; edges are derived from the \
         assets they connect",
    ),
    (
        "asset_service::rebuild_sessions",
        "enqueues the session reconciliation pass; sessions are derived \
         at query time from the assets already recorded",
    ),
    (
        "asset_service::rebuild_index",
        "enqueues the full-text index backfill; the index is a \
         projection of text already stored",
    ),
    (
        "asset_service::rescan_duplicates",
        "enqueues a re-derivation of duplicate conflicts from digests \
         already stored; a conflict is a consequence of two rows holding \
         the same bytes, which is a fact about the corpus rather than an \
         assertion anybody made. The verb never folds — the pass runs as \
         `DetectionOrigin::Backfill` — so nothing it does is a write on \
         somebody's behalf (rule 4)",
    ),
    (
        "asset_service::pull_head",
        "enqueues the install of a pulled head artifact (#132 phase \
         3); the head is derived state keyed by its label — verified \
         against the encoder, never attributed to whoever pulled it, \
         the same reasoning as train_head below (rule 4)",
    ),
    (
        "asset_service::train_head",
        "enqueues a tag-head training run (#132); the head is derived \
         state — its inputs are the rulings people already made (each \
         one attributed where it was made) and cached vectors, and its \
         rows are keyed by head ref, never by who asked for the \
         attempt (rule 4)",
    ),
    (
        "asset_service::remeasure_dims",
        "enqueues a re-read of the named artefacts; pixel dimensions are \
         derived from the bytes. A person triggers it, but what lands is \
         what the file says — nobody asserts a resolution, so there is no \
         subject to attribute it to (rule 4)",
    ),
    (
        "asset_service::remeasure_dims_batch",
        "the library-scale sibling of `remeasure_dims`; same derivation, \
         same absence of a subject",
    ),
    (
        "asset_service::reresolve_unresolved",
        "sweep — re-runs resolution over claims already recorded, and \
         carries each note's existing channel forward. That the app was \
         running is not a subject a write can be attributed to (rule 4)",
    ),
    // ---- other services: reads
    ("session_service::get", "read — one session by id"),
    ("modality_service::list", "read — the modality registry"),
    (
        "series_strategy_service::list",
        "read — the registered series rules",
    ),
    ("thread_service::list", "read — thread listing"),
    ("thread_service::find", "read — one thread by id"),
    ("thread_service::list_messages", "read — message listing"),
    (
        "snapshot_service::list_containing",
        "read — the snapshots an asset appears in",
    ),
    ("snapshot_service::get_snapshot", "read — one snapshot"),
    (
        "snapshot_service::snapshot_members",
        "read — snapshot members",
    ),
    ("asset_comment_service::list", "read — comments on an asset"),
    (
        "material_mark_service::list_by_asset",
        "read — the marks in an asset's material",
    ),
    (
        "material_layer_service::list_by_asset",
        "read — the bands of marks over an asset's material",
    ),
    (
        "material_layer_service::list_chapters",
        "read — the sections one band declares",
    ),
    (
        "material_layer_service::list_views",
        "read — the same bands as `list_by_asset`, each with its \
         sections, shaped for the wire",
    ),
    (
        "material_layer_service::list_chapter_marks",
        "read — `list_chapters` shaped for the wire",
    ),
    ("persona_service::list", "read — persona listing"),
    ("persona_service::get_theme", "read — one persona's theme"),
    (
        "persona_service::get_profile",
        "read — one persona's profile",
    ),
    ("app_setting_service::list", "read — the resolved settings"),
    ("app_setting_service::get", "read — one resolved setting"),
    ("dispatch_service::get", "read — one dispatch job"),
    ("dispatch_service::list", "read — dispatch job listing"),
    (
        "sort_context::build_sort_context",
        "read — assembles the keys a listing query sorts on",
    ),
    (
        "fold_redirect::redirect",
        "read — asks which of a caller's stored ids were folded, and \
         answers with the ids they now name",
    ),
    (
        "fold_redirect::hydrate_named",
        "read — the redirect above followed by the hydration its caller \
         wanted",
    ),
    // ---- derived writes
    (
        "thumb_service::put",
        "cache write — a rendered thumbnail is derived from an asset; \
         it records no author because nobody wrote it",
    ),
    ("thumb_service::get", "read — a cached thumbnail"),
    (
        "thumb_service::get_many",
        "read — a screenful of cached thumbnails in one round trip",
    ),
    (
        "query_group_service::evaluate_and_materialize",
        "derived write — materialises the membership a saved query \
         already implies. The rows belong to the query, not to whoever \
         happened to trigger the refresh",
    ),
    (
        "material_layer_service::default_annotation_layer",
        "derived write — opens the band a note is about to land in, not \
         the note. A layer row carries no author column of its own; the \
         record being attributed is the mark, and `material_mark_service::\
         post` takes the context for it",
    ),
    (
        "material_layer_service::imported_structure_layer",
        "derived write — opens the band a reading of the material lands \
         in. The band belongs to the file, not to whoever triggered the \
         read, which is the same reason the chapters in it carry no author",
    ),
];

#[test]
fn every_application_mutation_receives_an_attribution_context() {
    let root = workspace_root();
    let files = rust_sources(&root.join("crates/asterism-core/src/application"));
    let functions: Vec<PublicFn> = files
        .iter()
        .flat_map(|path| public_fns(&root, path))
        .collect();

    // Positive anchor: a parser that reads nothing produces no
    // offenders either.
    assert!(
        functions.iter().any(|f| f.key() == "asset_service::add"),
        "the population is read off the source; not finding `asset_service::add` \
         in {} public functions means the parse, not the layer, is wrong",
        functions.len()
    );

    for (key, reason) in CONTEXT_EXEMPT {
        assert!(
            key.contains("::"),
            "exemptions are keyed by `module::fn`; a bare name exempts \
             every namesake in the layer: {key}"
        );
        assert!(
            !reason.trim().is_empty(),
            "{key} is exempt without saying why it is not a write the \
             rule attributes"
        );
    }
    let exempt: BTreeSet<&str> = CONTEXT_EXEMPT.iter().map(|(key, _)| *key).collect();
    assert_eq!(
        exempt.len(),
        CONTEXT_EXEMPT.len(),
        "an exemption is listed twice; two reasons for one verb means \
         one of them is not the reason"
    );

    let mut unattributed: Vec<String> = Vec::new();
    let mut unused: BTreeSet<&str> = exempt.clone();
    for function in functions.iter().filter(|f| f.is_async) {
        if function.args.contains("AttributionContext") {
            continue;
        }
        let key = function.key();
        if exempt.contains(key.as_str()) {
            unused.remove(key.as_str());
        } else {
            unattributed.push(function.site());
        }
    }

    assert!(
        unattributed.is_empty(),
        "these application verbs write without receiving an \
         AttributionContext. Take one as a required argument next to \
         (never inside) the command — or, if the verb reads, derives or \
         sweeps, add it to CONTEXT_EXEMPT with the reason: {unattributed:#?}"
    );
    assert!(
        unused.is_empty(),
        "these exemptions match nothing any more — the verb was renamed, \
         removed, or now takes a context. A stale exemption is a hole \
         waiting for the next namesake: {unused:#?}"
    );
}

/// Modules under `application/` that the synchronous-verb rule does not
/// apply to, with the reason.
const SYNC_EXEMPT_MODULES: &[(&str, &str)] = &[(
    "mapping",
    "not a service — domain ↔ DTO conversion over values already in \
     hand. It holds no port and reaches no repository, so none of its \
     functions can write anything to attribute",
)];

/// Individual synchronous `pub fn`s that are neither constructors nor
/// covered by a module exemption, with the reason.
const SYNC_EXEMPT_FNS: &[(&str, &str)] = &[(
    "query_group_invalidation::notify_persona",
    "enqueues a refresh for the query groups a persona's change made \
     stale. Fire-and-forget notification about derived membership, and \
     synchronous because it only hands work to the queue (making it \
     async is a separate question from attribution)",
)];

/// Names a synchronous `pub fn` in `application/` may have.
const CONSTRUCTOR_NAMES: &[&str] = &["new", "with_env"];

#[test]
fn the_application_layer_keeps_no_synchronous_public_verbs() {
    let root = workspace_root();
    let files = rust_sources(&root.join("crates/asterism-core/src/application"));
    let functions: Vec<PublicFn> = files
        .iter()
        .flat_map(|path| public_fns(&root, path))
        .collect();

    // Positive anchor: constructors are synchronous and every service
    // has one, so a parse that sees no synchronous function is broken.
    assert!(
        functions
            .iter()
            .any(|f| !f.is_async && f.key() == "asset_service::new"),
        "no synchronous constructor found among {} public functions — \
         the parse, not the layer, is wrong",
        functions.len()
    );

    for (key, reason) in SYNC_EXEMPT_FNS {
        assert!(
            !reason.trim().is_empty(),
            "{key} is exempt without a reason"
        );
    }
    for (module, reason) in SYNC_EXEMPT_MODULES {
        assert!(
            !reason.trim().is_empty(),
            "module {module} is exempt without a reason"
        );
        assert!(
            functions.iter().any(|f| f.module == *module),
            "module exemption {module} matches nothing in application/"
        );
    }

    let exempt_modules: BTreeSet<&str> = SYNC_EXEMPT_MODULES.iter().map(|(m, _)| *m).collect();
    let exempt_fns: BTreeSet<&str> = SYNC_EXEMPT_FNS.iter().map(|(key, _)| *key).collect();
    let mut unused: BTreeSet<&str> = exempt_fns.clone();

    let mut synchronous: Vec<String> = Vec::new();
    for function in functions.iter().filter(|f| !f.is_async) {
        if exempt_modules.contains(function.module.as_str())
            || CONSTRUCTOR_NAMES.contains(&function.name.as_str())
        {
            continue;
        }
        if exempt_fns.contains(function.key().as_str()) {
            unused.remove(function.key().as_str());
        } else {
            synchronous.push(function.site());
        }
    }

    assert!(
        synchronous.is_empty(),
        "a synchronous public verb in this layer sits outside the \
         population the previous guard walks, which only reads \
         `pub async fn`. That is how a write would come to be exempt \
         without anybody deciding it should be: {synchronous:#?}"
    );
    assert!(
        unused.is_empty(),
        "these synchronous exemptions match nothing any more: {unused:#?}"
    );
}

// ------------------------------------------------- the Tauri mutation subset

// The tauri mutation-surface count (`TAURI_MUTATION_COMMANDS` and its
// test) moved to `asterism-ui`'s own tests in #159: it reads that
// crate's `src/commands.rs`, and the `-changed` gates run a crate's
// tests only when that crate changes — from here, the guard was silent
// exactly when its subject moved (#154). Do not bring it back.
