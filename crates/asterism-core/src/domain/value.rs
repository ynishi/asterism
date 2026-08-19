//! Value objects: id / slug / text newtypes plus `Visibility`, `SourceRef`,
//! `Progress`, and `Page<T>`.
//!
//! Design notes:
//!
//! - **Surrogate ids are UUID v7.** Natural keys (for example `pack_id` from
//!   an external persona pack) live in separate fields with unique
//!   constraints.
//! - **`Modality` and `SourceKind` are open slugs.** Asterism is a
//!   general-purpose grid product; adding a new consumer must be a data
//!   change, not a breaking enum change. Well-known slugs are exposed as
//!   associated constants.
//! - **`Visibility` powers the enforcement of visibility filters.** The
//!   decision function (`visible_to`) lives here; SQL translation is the
//!   adapter's job in `asterism-infra`.

use crate::domain::source_locator::SourceLocator;
use crate::error::DomainError;
use uuid::Uuid;

/// Declares a UUID v7 surrogate id newtype.
macro_rules! define_uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            /// Mints a fresh id (UUID v7 is time-sortable).
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Rehydrates an id read back from persistence.
            pub fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the underlying UUID.
            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

// Shared with `domain::forge::value`, which declares the forge's own ids
// with the same shape. `macro_rules!` is textually scoped, so the
// sibling module cannot reach it without this.
pub(crate) use define_uuid_id;

define_uuid_id!(
    /// The catalogue's handle on a forge round.
    ///
    /// A dispatch filed under a pursuit has to carry which one — but
    /// the catalogue does not know what a pursuit is,
    /// and a type it cannot name is a type it cannot depend on. So the
    /// stamp is held opaquely here and the forge puts its own
    /// [`PursuitId`](crate::domain::forge::value::PursuitId) over it,
    /// converting at the boundary.
    ///
    /// The field, the column, and the sidecar key all stay
    /// `pursuit_id`: that is what the id refers to, and three names for
    /// one value would cost more than the one this type buys. What
    /// changes is the direction of the dependency, which is the part a
    /// compiler can hold.
    CorrelationId
);

define_uuid_id!(
    /// Surrogate id for `Persona`. The natural key (external pack id) is
    /// [`PackId`].
    PersonaId
);
define_uuid_id!(
    /// Surrogate id for `Asset`.
    AssetId
);
define_uuid_id!(
    /// Surrogate id for `Tag`.
    TagId
);
define_uuid_id!(
    /// Surrogate id for this Asterism instance — the single
    /// [`InstanceIdentity`](crate::domain::instance::InstanceIdentity)
    /// row a profile database carries. Minted once, when the profile is
    /// first migrated to the schema that holds it, and never reissued:
    /// it is what
    /// [`Author::Owner`](crate::domain::attribution::Author::Owner)
    /// ultimately refers to.
    InstanceId
);
define_uuid_id!(
    /// Surrogate id for `Group` (user-curated set of assets).
    GroupId
);
define_uuid_id!(
    /// Surrogate id for `Dir` (sidebar organisation folder).
    DirId
);
define_uuid_id!(
    /// Surrogate id for `ConstellationEdge`.
    EdgeId
);
define_uuid_id!(
    /// Surrogate id for `Job`.
    JobId
);
define_uuid_id!(
    /// Surrogate id for a `Snapshot` — the immutable, content-addressed
    /// freeze of an ordered asset set (a git-tree
    /// analogue). Producers dedupe on
    /// `(persona_id, content_hash)`, so the same members in the same
    /// order collapse onto one id that dispatch / promote history
    /// references.
    SnapshotId
);
define_uuid_id!(
    /// Surrogate id for a `DispatchJob` — one exporter invocation
    /// against a Snapshot. Reified derived Assets carry it as
    /// `session_id` so per-dispatch siblings cluster on the grid.
    DispatchId
);
define_uuid_id!(
    /// Surrogate id for an `AssetComment` — one entry in an Asset's
    /// comment thread. UUID v7 keeps the natural chronological
    /// ordering matching `created_at`.
    AssetCommentId
);
define_uuid_id!(
    /// Surrogate id for a `MaterialMark` — one mark in an Asset's
    /// material (today a point or interval on its playback timeline).
    /// Ordering of marks is by position in that material, not by mint
    /// time, so the v7 timestamp carries no meaning here beyond being a
    /// stable tie-break.
    MaterialMarkId
);
define_uuid_id!(
    /// Surrogate id for a `MaterialLayer` — one band of marks over an
    /// Asset's material (the imported chapter list, a user's own pass
    /// over it, a machine's). Layers are ordered by their `ord` column
    /// rather than by mint time, so the v7 timestamp carries no meaning
    /// here beyond being a stable tie-break.
    MaterialLayerId
);
define_uuid_id!(
    /// Surrogate id for a `ChapterMark` — one entry in a structure
    /// layer's chapter list. Ordering is by the layer's own `ord` and
    /// then by position on the timeline, so the v7 timestamp is again a
    /// tie-break and not the reading order.
    ChapterMarkId
);
define_uuid_id!(
    /// Surrogate id for a `DuplicateConflict` — one raised "are these
    /// two the same thing?" question. Surrogate rather than the pair
    /// itself because the pair is the *unique key* and an id is what a
    /// resolution verb names.
    DuplicateConflictId
);
define_uuid_id!(
    /// Surrogate id for a `Thread` — the app-level container that
    /// collects `Message`s from both the UI (human) and the HTTP
    /// channel (Claude Code / agents). Anchored to `AppGlobal`,
    /// `Snapshot`, `QueryGroup`, or `Card`. UUID v7 keeps the
    /// creation order natural.
    ThreadId
);
define_uuid_id!(
    /// Surrogate id for a `Message` — one entry appended to a
    /// [`Thread`]. UUID v7 preserves chronological ordering that
    /// matches `created_at`.
    MessageId
);
define_uuid_id!(
    /// Surrogate id for a
    /// [`Strategy`](crate::domain::series::Strategy) — one rule for
    /// reading "made the same way" out of a material's metadata.
    ///
    /// Surrogate rather than the rule's name because the name is a
    /// label a person edits and the id is what a derived row is filed
    /// under: `material_series(material_id, strategy_id, key)` is keyed
    /// by this, so renaming a Strategy leaves every key it derived
    /// exactly where it was. It is also what keeps two Strategies that
    /// happen to derive the same key from being read as one grouping.
    StrategyId
);
// The forge's own ids — `PursuitId` and the nine around it — are
// declared in [`domain::forge::value`](crate::domain::forge::value), so
// that this module names no forge type. What the catalogue holds of
// them is `CorrelationId` above.

/// Declares a non-empty text newtype (returns `Validation` if the value is
/// blank after trimming).
macro_rules! define_text_vo {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// Builds the value, rejecting empty/whitespace input.
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(DomainError::Validation(format!(
                        "{} must not be empty",
                        stringify!($name)
                    )));
                }
                Ok(Self(value))
            }

            /// Returns the underlying string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

define_text_vo!(
    /// Natural key from an external persona pack (unique when present).
    PackId
);
define_text_vo!(
    /// Session identifier attached to a dialogue asset — after the
    /// V26 / V27 migration wave this string is the hyphenated UUID
    /// of the owning [`Session`](crate::domain::session::Session)
    /// entity's primary key (`session.id`). The importer no longer
    /// stores the raw external key here; that role belongs to the
    /// separate [`ExternalSessionKey`] value object, which the
    /// application layer resolves to a `Session.id` via
    /// `SessionService::find_or_create_by_external_key`.
    ///
    /// The VO is deliberately kept as a text newtype (not a UUID id)
    /// for this subtask: the write / read paths that materialise
    /// `Asset.session_id` still round-trip a `TEXT` column, and the
    /// application-layer rewrite that consumes the `Session` entity
    /// is P1b work. The invariant `session_id IS NOT NULL →
    /// modality = 'dialogue'` is enforced by the V27 DB CHECK.
    SessionId
);

/// Non-empty external session identifier accepted by
/// [`ExternalSessionKey::new`] / [`BundleId::new`]. Matches the
/// `[\w./:-]{1,256}` shape used at importer boundaries (Claude Code
/// session UUIDs, JSONL file stems, persona-journal composite ids,
/// tape file stems).
fn is_external_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '-')
}

/// Declares an external-id newtype matching `[\w./:-]{1,256}`.
///
/// `ExternalSessionKey` (importer-supplied session key) and `BundleId`
/// (constellation-edge grouping key on non-dialogue modalities) share
/// the same grammar; the two are separate types so a call site cannot
/// silently confuse one for the other.
macro_rules! define_external_id_vo {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// Builds the value, requiring `[\w./:-]{1,256}` (non-empty).
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                let valid = !value.is_empty()
                    && value.len() <= 256
                    && value.chars().all(is_external_id_char);
                if !valid {
                    return Err(DomainError::Validation(format!(
                        "{} must match [\\w./:-]{{1,256}}: {value:?}",
                        stringify!($name)
                    )));
                }
                Ok(Self(value))
            }

            /// Returns the underlying string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

define_external_id_vo!(
    /// The raw session identifier an importer hands in — for example
    /// the Claude Code session UUID, a persona-journal composite id
    /// like `"persona-journal/aya/state"`, or a tape file stem. The
    /// application layer maps each unique `(persona_id, external_key)`
    /// to a `Session.id` via `SessionService::find_or_create_by_external_key`,
    /// so re-imports converge on the same row.
    ExternalSessionKey
);

define_external_id_vo!(
    /// Open grouping key used by the constellation-edge builder for
    /// non-dialogue modalities (tape / journal / image / future
    /// slot). Distinct from [`SessionId`] because Session is now a
    /// Dialog-modality-only 1st-class entity; `bundle_id` keeps the
    /// pre-existing "time_proximity edge grouping key" role alive
    /// under its own name. Modality-agnostic.
    BundleId
);
define_text_vo!(
    /// Free-form annotation attached to an asset.
    ///
    /// Labels intentionally accept anything short (status hints, category
    /// notes, secondary modality tags) rather than restricting to a fixed
    /// vocabulary — the empirical usage in real footprints is heterogeneous.
    Label
);

/// Drops repeats from a label list, keeping the **first** occurrence of
/// each value and the order of what is left.
///
/// # Why first-wins rather than sort-and-dedup
///
/// The head of the list is load-bearing. `sort_eval::first_user_label`
/// reads the first entry that is not an internal (`persona:` /
/// `journal_kind:`) prefix and uses it as the Label sort key — the UI
/// comparator does the same in `card-cmp.ts` (`firstUserLabel`). A
/// dedup that reorders would silently re-sort the grid, so the only
/// safe shape is "keep the first, drop the later copies". The fold path
/// already states the same rule for the same reason
/// (`MergeRule::UnionJson` in `asterism-infra`: "the keeper's entries in
/// their own order, then whatever the headstone had and it did not").
///
/// # What a repeat breaks
///
/// The label chips are rendered from a Svelte keyed `{#each}`. Two
/// equal labels in one list are two equal keys, which is a runtime
/// error (`each_key_duplicate`) that takes down the whole virtual list,
/// not a cosmetic double chip — reported 2026-07-20 from the running
/// app, naming `assistant`. Note what that is evidence of: Svelte
/// refusing a duplicate key, not a stored row anyone has read back. No
/// repeat has turned up in storage since (293 rows in the dogfood
/// profile, none carrying one), so this guards a shape that is inferred
/// from the error rather than sampled from the data. The UI keys its
/// chips defensively for the same reason; this function is what keeps
/// one from being written.
///
/// Cheap enough to call on every write path: label lists are a handful
/// of entries, and the allocation is one `HashSet` of the same size.
pub fn dedup_labels(labels: Vec<Label>) -> Vec<Label> {
    let mut seen: std::collections::HashSet<Label> =
        std::collections::HashSet::with_capacity(labels.len());
    let mut kept = Vec::with_capacity(labels.len());
    for label in labels {
        if seen.insert(label.clone()) {
            kept.push(label);
        }
    }
    kept
}

define_text_vo!(
    /// Raw keyword extracted by the auto-tag pipeline; a `Tag` may be
    /// materialised from it.
    Keyword
);
define_text_vo!(
    /// Text shown on the grid card for an asset (produced by the CoverGen
    /// job with a modality-specific template).
    CoverText
);
define_text_vo!(
    /// Short annotation about the asset's register / tone; the presentation
    /// layer decides how to render it.
    RegisterNote
);

/// Declares a slug newtype matching `[a-z0-9_-]{1,64}`.
///
/// `Modality` and `SourceKind` use this shape: the value space stays open,
/// well-known slugs are exposed as associated constants, and adding a new
/// consumer is a data change (no enum breakage).
macro_rules! define_slug_vo {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// Builds the value, requiring `[a-z0-9_-]{1,64}`.
            pub fn new(slug: impl Into<String>) -> Result<Self, DomainError> {
                let slug = slug.into();
                let valid = !slug.is_empty()
                    && slug.len() <= 64
                    && slug
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
                if !valid {
                    return Err(DomainError::Validation(format!(
                        "{} must match [a-z0-9_-]{{1,64}}: {slug:?}",
                        stringify!($name)
                    )));
                }
                Ok(Self(slug))
            }

            /// Returns the underlying slug.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

define_slug_vo!(
    /// Primary modality slug for an asset (open slug).
    ///
    /// Well-known values live as associated constants. Localised display
    /// names are the presentation layer's concern; the domain holds only the
    /// slug.
    Modality
);
define_slug_vo!(
    /// Ingest source slug for an asset (open slug).
    ///
    /// Source-specific metadata that does not warrant a first-class column
    /// lives in `Asset::extra`.
    SourceKind
);

impl Modality {
    // `dialogue` / `session` are gone (asset-model v4, V38): "is a
    // conversation message" is containment (`container_id`), "is a
    // container" is `AssetRole::Collection` — neither is a semantic
    // classification, so neither is a modality.
    /// Work product (design docs, specs, code artefacts, and so on).
    pub const WORK_PRODUCT: &'static str = "work_product";
    /// Terminal transcript / Persona Tape (`.tape`, `.cast`, `.log`).
    pub const TAPE: &'static str = "tape";
    /// Tick log from a periodic cycle.
    pub const TICK_LOG: &'static str = "tick_log";
    /// Memory / long-term note.
    pub const MEMORY: &'static str = "memory";
    /// Register / mood-state note.
    pub const STATE: &'static str = "state";
    /// Emotional-tilt marker.
    pub const EMO: &'static str = "emo";
    /// Dream / subconscious fragment.
    pub const NON_REM: &'static str = "non_rem";

    /// Well-known modality slugs. Used as fallback material where an
    /// exhaustive match would otherwise be tempting; the value space stays
    /// open (a user-created slug is valid without a code change).
    pub fn well_known() -> [&'static str; 7] {
        [
            Self::WORK_PRODUCT,
            Self::TAPE,
            Self::TICK_LOG,
            Self::MEMORY,
            Self::STATE,
            Self::EMO,
            Self::NON_REM,
        ]
    }
}

impl SourceKind {
    /// Direct ingest from the local filesystem.
    pub const FS: &'static str = "fs";
    /// Ingested via an external persona-pack primitive.
    pub const PERSONA_PACK: &'static str = "persona-pack";
    /// Ingested via an external persona-journal primitive.
    pub const PERSONA_JOURNAL: &'static str = "persona-journal";
    /// Prefix used for `source_kind`s materialised by the outbound
    /// dispatch pipeline. Concatenated with an exporter slug to form
    /// values like `"dispatch-comfy"` / `"dispatch-file"` /
    /// `"dispatch-http"`. Use [`SourceKind::for_dispatch`] instead of
    /// formatting the prefix by hand — the factory validates the
    /// resulting grammar and rejects exporter slugs that would break
    /// [`SourceKind::new`].
    pub const DISPATCH_PREFIX: &'static str = "dispatch-";

    /// Builds the `source_kind` that
    /// `asterism_core::application::forge::DispatchService::reify` writes on
    /// each derived Asset row for a dispatch that was handled by
    /// `exporter_slug`.
    ///
    /// The exporter slug is concatenated with
    /// [`DISPATCH_PREFIX`](SourceKind::DISPATCH_PREFIX) and the whole
    /// value is fed through [`SourceKind::new`], so an exporter slug
    /// carrying characters outside `[a-z0-9_-]` — e.g. the `:`
    /// separator style used in an earlier draft — is rejected here
    /// instead of surfacing as an infrastructure error deep inside
    /// reify.
    ///
    /// This factory is the canonical construction site for the
    /// dispatch-side slug; hand-rolled `format!("dispatch:{...}")` /
    /// `format!("dispatch-{...}")` call sites are a regression trap
    /// (the 2026-07-19 smoke Test Red on `dispatch:file` was exactly
    /// this shape).
    pub fn for_dispatch(exporter_slug: &str) -> Result<Self, DomainError> {
        // Empty / whitespace-only slugs slip past `SourceKind::new`
        // (`"dispatch-"` on its own satisfies `[a-z0-9_-]{1,64}`) but
        // convey no useful origin — reject them here so the failure
        // surfaces at the actual mistake site.
        if exporter_slug.trim().is_empty() {
            return Err(DomainError::Validation(
                "SourceKind::for_dispatch: exporter slug must not be empty".into(),
            ));
        }
        Self::new(format!("{}{}", Self::DISPATCH_PREFIX, exporter_slug))
    }
}

// `ContentKind` lived here: a closed enum the Modality master pointed
// at, holding the behaviour the semantic axis was allowed to decide.
// Asset-model v4 moved that behaviour to where the facts are — the
// material's mime (`crate::domain::render::render_policy`) and
// `AssetRole` — and what survived was a single yes/no, "does this read
// as a terminal transcript?". A closed set of two is a bool, so the
// master carries `terminal` directly (V44). If a second display mode
// ever earns its place, it returns as a slug column pointing at a
// closed enum, the shape `CoverTemplate` uses.

/// Media-render variant selected by a [`ContentKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// No inline media (text-like kinds).
    None,
    /// Still-image render.
    Image,
    /// Video player.
    Video,
    /// Audio player (+ waveform).
    Audio,
}

impl MediaKind {
    /// Slug this crosses the wire as.
    ///
    /// The same rule `role` and `author_kind` follow (`dto.rs`): a
    /// closed set reaches the DTO as its stored token. Without one the
    /// UI had to re-derive the answer from the mime string, and the
    /// two implementations drifted — `render_policy` gained an unnamed
    /// `image/*` subtype arm that five `startsWith("image/")` sites in
    /// the frontend knew nothing about.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

/// Preview mode selected by a [`ContentKind`] for the QuickLook overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// No dedicated text preview (media kinds render inline instead).
    None,
    /// Sniff the text shape (markdown / code / plain) at preview time.
    TextSniff,
    /// Terminal transcript preview.
    Term,
}

/// What an asset's bytes are, parsed once at the mapping boundary.
///
/// ## Why this is a type and not a `String`
///
/// The set of formats this app *acts on* is closed — the two lists that
/// used to state it ([`KNOWN_IMAGE_MIMES`] / [`KNOWN_VIDEO_MIMES`] in
/// `material`) even carried tripwire tests to keep them closed. What
/// they could not do is make the call sites ask. With the mime held as
/// a string, "is this an image" was answered by a `starts_with` at each
/// site that cared, which meant a site that *should* have cared and did
/// not was invisible:
///
/// - `index_rebuild` read every asset's bytes as lossy UTF-8 and put
///   them in the full-text index, because the enqueue (`AssetService`)
///   and the reader (`SourceTextReader`) both took a locator with no
///   format attached and neither wrote the check [measured 2026-08-05: a
///   5,000-file PNG corpus indexed whole].
/// - Before that, a PNG tEXt note was filed as `image/png` and the
///   thumbnailer was handed `shot.png#workflow` as a path — fixed by
///   adding a branch to [`guess_mime`](crate::domain::material::guess_mime),
///   which left the next site to make the same omission.
///
/// Two instances of one shape is the shape's fault. Parsed here, the
/// question is `match`ed rather than spelled, and a new variant makes
/// every site that must decide fail to compile.
///
/// ## Why unknown values parse instead of erroring
///
/// [`AssetRole::parse`] rejects what it does not know, and should: that
/// column holds slugs this codebase writes. A mime does not — V37
/// backfilled it from extensions in SQL, and any future importer may
/// carry its own. Rejecting would turn one unrecognised row into a
/// failed mapping, so an unknown value is *kept verbatim* in the
/// `Other` arm of its family. Round-tripping matters: the string is
/// what an HTTP `Content-Type` sends back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MimeType {
    /// A raster still (`image/*`).
    Image(ImageFormat),
    /// A moving picture (`video/*`).
    Video(VideoFormat),
    /// Sound (`audio/*`).
    Audio(AudioFormat),
    /// Any `text/*`. Which text it is has never changed a decision, so
    /// the subtype rides along verbatim rather than as its own set.
    Text(Box<str>),
    /// A parsed value in no family the app acts on (`application/pdf`,
    /// `font/woff2`, …).
    Other(Box<str>),
}

/// The `image/*` formats [`guess_mime`](crate::domain::material::guess_mime)
/// produces, plus anything else that arrived as `image/*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageFormat {
    /// PNG. Chunked, so a reading can tell the pixels from the `tEXt` a
    /// generator wrote beside them — the separation the walking axes are
    /// defined over. Whether a build reads it that way is a fact about
    /// that build's registry of
    /// [`ArtefactProbe`](crate::domain::probe::ArtefactProbe)s rather
    /// than about this variant.
    Png,
    /// JPEG.
    Jpeg,
    /// GIF.
    Gif,
    /// WebP.
    Webp,
    /// HEIC (the iPhone default).
    Heic,
    /// HEIF.
    Heif,
    /// AVIF.
    Avif,
    /// TIFF.
    Tiff,
    /// Windows bitmap.
    Bmp,
    /// An `image/*` subtype this codebase does not name. Still an
    /// image: it tiles and renders like one, and dropping it into
    /// [`MimeType::Other`] would silently stop both.
    Other(Box<str>),
}

/// The `video/*` formats, plus anything else that arrived as `video/*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoFormat {
    /// MP4 — plays natively in the packaged webview.
    Mp4,
    /// QuickTime (`.mov`) — plays natively.
    Quicktime,
    /// VP9 by default, which the packaged webview cannot decode.
    Webm,
    /// Matroska (`.mkv`) — the container is rejected outright.
    Matroska,
    /// AVI (`video/x-msvideo`).
    Msvideo,
    /// A `video/*` subtype this codebase does not name.
    Other(Box<str>),
}

/// The `audio/*` formats, plus anything else that arrived as `audio/*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioFormat {
    /// MP3 (`audio/mpeg`).
    Mpeg,
    /// WAV.
    Wav,
    /// AAC in an MP4 container (`.m4a`).
    Mp4,
    /// An `audio/*` subtype this codebase does not name.
    Other(Box<str>),
}

impl MimeType {
    /// `text/plain` — what every textual source this codebase names
    /// resolves to, including a record addressed inside a container.
    pub fn text_plain() -> Self {
        Self::Text("text/plain".into())
    }

    /// Parses a media type, normalising as it goes.
    ///
    /// Normalisation (drop the `;` parameters, trim, lowercase) used to
    /// live in exactly one consumer (`content_region::normalise_mime`),
    /// so `IMAGE/PNG; charset=binary` was a PNG to the content walker
    /// and to nothing else. Doing it here makes that the parsed form,
    /// which is the only form the domain sees.
    pub fn parse(raw: &str) -> Self {
        let normalised = raw
            .split(';')
            .next()
            .unwrap_or(raw)
            .trim()
            .to_ascii_lowercase();
        match normalised.as_str() {
            "image/png" => Self::Image(ImageFormat::Png),
            "image/jpeg" => Self::Image(ImageFormat::Jpeg),
            "image/gif" => Self::Image(ImageFormat::Gif),
            "image/webp" => Self::Image(ImageFormat::Webp),
            "image/heic" => Self::Image(ImageFormat::Heic),
            "image/heif" => Self::Image(ImageFormat::Heif),
            "image/avif" => Self::Image(ImageFormat::Avif),
            "image/tiff" => Self::Image(ImageFormat::Tiff),
            "image/bmp" => Self::Image(ImageFormat::Bmp),
            "video/mp4" => Self::Video(VideoFormat::Mp4),
            "video/quicktime" => Self::Video(VideoFormat::Quicktime),
            "video/webm" => Self::Video(VideoFormat::Webm),
            "video/x-matroska" => Self::Video(VideoFormat::Matroska),
            "video/x-msvideo" => Self::Video(VideoFormat::Msvideo),
            "audio/mpeg" => Self::Audio(AudioFormat::Mpeg),
            "audio/wav" => Self::Audio(AudioFormat::Wav),
            "audio/mp4" => Self::Audio(AudioFormat::Mp4),
            other => {
                // The family still decides behaviour even when the
                // subtype is unknown, so it is read before giving up.
                if other.starts_with("image/") {
                    Self::Image(ImageFormat::Other(other.into()))
                } else if other.starts_with("video/") {
                    Self::Video(VideoFormat::Other(other.into()))
                } else if other.starts_with("audio/") {
                    Self::Audio(AudioFormat::Other(other.into()))
                } else if other.starts_with("text/") {
                    Self::Text(other.into())
                } else {
                    Self::Other(other.into())
                }
            }
        }
    }

    /// The stored / transmitted token, per the rule `AssetRole` and
    /// `author_kind` already follow: a closed set reaches the wire as
    /// the token it is stored as.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Image(f) => f.as_str(),
            Self::Video(f) => f.as_str(),
            Self::Audio(f) => f.as_str(),
            Self::Text(raw) | Self::Other(raw) => raw,
        }
    }

    /// Which inline player the detail view uses.
    pub fn media(&self) -> MediaKind {
        match self {
            Self::Image(_) => MediaKind::Image,
            Self::Video(_) => MediaKind::Video,
            Self::Audio(_) => MediaKind::Audio,
            Self::Text(_) | Self::Other(_) => MediaKind::None,
        }
    }

    /// Whether a cached raster can be generated for this.
    pub fn thumbnailable(&self) -> bool {
        matches!(self, Self::Image(_) | Self::Video(_))
    }

    /// Whether these bytes are text to be read into the body cache and
    /// the full-text index.
    ///
    /// The check `index_rebuild` did not have. Anything not `text/*`
    /// answers `false`, including formats that merely *contain* text
    /// (PDF, PNG tEXt): extracting those is a decoder's job, and
    /// reading the container's raw bytes as lossy UTF-8 is not it.
    pub fn body_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    /// Whether the detail player needs a transcoded rendition rather
    /// than the original.
    ///
    /// Shares its set with [`VideoFormat::needs_external_frame_grab`]
    /// by construction — the two used to be separate copies of the same
    /// three literals in `render` and `thumb_ffmpeg`, which drift the
    /// moment one gains a format.
    pub fn needs_video_preview(&self) -> bool {
        matches!(self, Self::Video(f) if f.webview_cannot_play())
    }

    /// Whether a container of these bytes can declare a chapter list.
    ///
    /// Video and audio, and the boundary is not about which containers
    /// happen to implement the feature: a chapter divides a **playback
    /// timeline**, and these are the two families that have one. A PDF
    /// has an outline and a PNG has nothing, and neither division is
    /// addressed by
    /// [`TimelineSpan`](crate::domain::material_mark::TimelineSpan).
    ///
    /// Read by the `ChapterScan` enqueue and by the handler that picks
    /// the job up, so the two agree by construction — the property the
    /// thumbnail path had to be rewritten to get, after enqueue and
    /// handler carried separate copies of the same rule.
    pub fn carries_chapters(&self) -> bool {
        matches!(self, Self::Video(_) | Self::Audio(_))
    }
}

impl std::fmt::Display for MimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ImageFormat {
    /// The stored token.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Heic => "image/heic",
            Self::Heif => "image/heif",
            Self::Avif => "image/avif",
            Self::Tiff => "image/tiff",
            Self::Bmp => "image/bmp",
            Self::Other(raw) => raw,
        }
    }
}

impl VideoFormat {
    /// The stored token.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Mp4 => "video/mp4",
            Self::Quicktime => "video/quicktime",
            Self::Webm => "video/webm",
            Self::Matroska => "video/x-matroska",
            Self::Msvideo => "video/x-msvideo",
            Self::Other(raw) => raw,
        }
    }

    /// Formats the packaged webview refuses [measured 2026-07-31, WKWebView
    /// 605.1.15]: WebM because VP9 never decodes in the DOM, Matroska
    /// because the container is rejected outright, AVI likewise. VP8
    /// WebM would play, but the mime cannot tell VP8 from VP9, so all
    /// WebM takes the rendition path — a spurious transcode is cheap, a
    /// crossed-out player is not.
    pub fn webview_cannot_play(&self) -> bool {
        matches!(self, Self::Webm | Self::Matroska | Self::Msvideo)
    }

    /// Whether a frame grab needs external ffmpeg rather than the
    /// native extractor. The same set, for the same reason: these are
    /// the containers the system frameworks do not open.
    pub fn needs_external_frame_grab(&self) -> bool {
        self.webview_cannot_play()
    }
}

impl AudioFormat {
    /// The stored token.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Mpeg => "audio/mpeg",
            Self::Wav => "audio/wav",
            Self::Mp4 => "audio/mp4",
            Self::Other(raw) => raw,
        }
    }
}

/// The template `cover_gen` applies to derive card cover text. A
/// [`Modality`] may override the kind default with one of these via its
/// `cover_template` column; the seed sets the three special-cased
/// templates on `dialogue` / `work_product` / `tape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverTemplate {
    /// Generic fallback — the first meaningful line (heading markers
    /// stripped). The default for every [`ContentKind`].
    FirstLine,
    /// Dialogue transcript — the first one or two non-empty lines.
    Dialogue,
    /// Work product — the title, optionally with the first body line.
    WorkProduct,
    /// Terminal Tape — the first prompt line (`❯`), else the first line.
    Tape,
}

impl CoverTemplate {
    /// Slug stored in the `modality.cover_template` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FirstLine => "first_line",
            Self::Dialogue => "dialogue",
            Self::WorkProduct => "work_product",
            Self::Tape => "tape",
        }
    }

    /// Parses a template slug (rejects unknown values with a validation
    /// error so a bad `cover_template` override surfaces on the write
    /// path rather than silently degrading a cover at job time).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "first_line" => Ok(Self::FirstLine),
            "dialogue" => Ok(Self::Dialogue),
            "work_product" => Ok(Self::WorkProduct),
            "tape" => Ok(Self::Tape),
            other => Err(DomainError::Validation(format!(
                "unknown cover template: {other:?}"
            ))),
        }
    }
}

/// Structural role of an asset: a curatable item carrying its own
/// physical originals, or a container whose content is its members.
///
/// Asset-model v4: the container
/// question used to ride on the `modality` slug (`'session'`), which
/// conflated user classification with structure. `role` is the closed
/// structural axis; `container_id` (membership) and `modality`
/// (semantic classification) stay orthogonal to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssetRole {
    /// A single curatable item — owns its
    /// [`Material`](crate::domain::material::Material) payloads.
    #[default]
    Item,
    /// A container (the Session shape): no material of its own — its
    /// content is the assets pointing at it via `container_id`.
    Collection,
}

impl AssetRole {
    /// Slug stored in the `asset.role` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Collection => "collection",
        }
    }

    /// Parses a role slug (rejects unknown values so a bad column value
    /// surfaces at the mapping boundary instead of degrading silently).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "item" => Ok(Self::Item),
            "collection" => Ok(Self::Collection),
            other => Err(DomainError::Validation(format!(
                "unknown asset role: {other:?}"
            ))),
        }
    }
}

/// Whether an asset may be folded into another, or has been ruled a
/// separate thing by a person.
///
/// The duplicate axis answers "same bytes", which in a collection built
/// from generated and re-imported work is not the same question as
/// "same asset". `Keep` is how that human
/// ruling survives: the pair keeps matching on content forever, and
/// raising the conflict again every time would turn the resolution
/// queue into a thing people stop reading.
///
/// Closed set, stored as the `asset.fold_policy` slug. The database
/// carries the same set as a column-level CHECK (V49) — measured to
/// survive `ALTER TABLE ADD COLUMN`, unlike the table-level constraint
/// V47's attribution columns had to do without — so this parse is the
/// second reader of the rule rather than its only enforcement.
///
/// **Not the same axis as [`OnDuplicate`]**, and the two are kept
/// apart deliberately. `fold_policy` is a fact about *this row* that a
/// person established by looking at a conflict that already happened;
/// `on_duplicate` is an instruction, declared before any conflict
/// exists, about how a future one should be handled. Everything else
/// follows from that: they are written by different actors (the
/// resolution verb versus the registering caller), read at different
/// moments (when a fold is considered versus when a fingerprint
/// lands), and mean different things when unset — `Auto` is a real
/// answer ("nobody has ruled"), while an absent `on_duplicate` is a
/// question nobody answered. Merging them into one column would force
/// one of those readings onto the other and lose the difference
/// between "we decided to keep these apart" and "nobody said".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FoldPolicy {
    /// Nobody has ruled on this row: a content match may raise a
    /// conflict, and a fold may act on it.
    #[default]
    Auto,
    /// A person decided this row is its own thing. Content matches
    /// against it stop being raised, and no fold takes it.
    Keep,
}

impl FoldPolicy {
    /// Slug stored in the `asset.fold_policy` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Keep => "keep",
        }
    }

    /// Parses a policy slug. An unknown value is a corrupt row rather
    /// than a reason to assume `Auto`: guessing here would silently
    /// un-rule a pair somebody had ruled on, which is the one outcome
    /// the column exists to prevent.
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "auto" => Ok(Self::Auto),
            "keep" => Ok(Self::Keep),
            other => Err(DomainError::Validation(format!(
                "unknown fold policy: {other:?}"
            ))),
        }
    }
}

/// What should happen if this asset turns out to hold bytes another
/// asset already holds — declared when the asset is registered.
///
/// The declaration has to live on the row because the question it
/// answers is asked long after the caller is gone: `add` returns
/// without reading a byte, and the fingerprint that can raise a
/// conflict is computed later by the `MaterialHash` job. A strategy
/// held in memory would not survive the gap, which is the same reason
/// `dispatch_job.operator_ai` (V48) is a column rather than a runner
/// argument.
///
/// Absence is not [`Ask`](Self::Ask). `None` means nobody declared
/// anything for this registration, and the detector resolves it against
/// the wider defaults (the resolution ladder puts the request layer
/// above an importer / lane setting, above a persona default). Only the
/// **request layer** exists today: this value is the whole of the
/// declaration surface, and the two layers under it are unimplemented,
/// so an undeclared row currently falls to the one default there is.
/// Writing `'ask'` in at registration would erase the difference
/// between a caller that asked for confirmation and a caller that said
/// nothing — and the second is the one whose meaning changes the day a
/// lane default exists.
///
/// See [`FoldPolicy`] for why the durable outcome of a resolution is a
/// separate column rather than a fourth value here.
///
/// Closed set, stored as the `asset.on_duplicate` slug, with the same
/// set repeated as a column-level CHECK (V50) — the shape V49 measured
/// to survive `ALTER TABLE ADD COLUMN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDuplicate {
    /// Register normally, record the match, and leave the decision to a
    /// person: the conflict goes on the queue for confirmation.
    Ask,
    /// Fold into the existing asset without asking — for lanes that
    /// re-import the same material on purpose and want one row.
    Fold,
    /// Keep both rows and only record that the bytes matched — for
    /// lanes that produce identical material deliberately.
    Separate,
}

impl OnDuplicate {
    /// Slug stored in the `asset.on_duplicate` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Fold => "fold",
            Self::Separate => "separate",
        }
    }

    /// Parses a strategy slug. Rejects the unknown rather than falling
    /// back to [`Ask`](Self::Ask), on the same grounds as
    /// [`FoldPolicy::parse`]: a value outside the closed set can only
    /// come from a hand-edited row, and reading it as "ask" would turn a
    /// corrupt row into a plausible-looking instruction.
    ///
    /// Note there is no `parse("")` / `parse(null)` case — absence never
    /// reaches here, because it is carried as `Option::None` all the way
    /// from the column.
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "ask" => Ok(Self::Ask),
            "fold" => Ok(Self::Fold),
            "separate" => Ok(Self::Separate),
            other => Err(DomainError::Validation(format!(
                "unknown duplicate strategy: {other:?}"
            ))),
        }
    }
}

/// Reference to the real source of truth for an asset.
///
/// Invariant: Asterism never writes back to `locator`; only the metadata /
/// index / cover columns owned by Asterism itself are mutated.
///
/// # Locator shapes
///
/// Asterism is local-first: the originals it browses are files on this
/// disk, referenced in place. Besides the absolute path (the common
/// case), two non-file shapes are ordinary internal record forms — a
/// fragment (`session.jsonl#<id>`) addressing one record inside a
/// container file on this disk (its text is read back through
/// [`SourceTextReader`](crate::domain::repository::SourceTextReader)),
/// and a caller-minted logical name (`chat/<id>/msg-1`) for something
/// that never had a file.
///
/// A remote URL is accepted at registration and stored as a fact about
/// origin, but nothing dereferences it — no code path fetches over the
/// network. Original-file serving refuses it with `409 Conflict`
/// ([`SourceLocator::local_path`] answering `None`), content hashing
/// records the permanent
/// [`UNHASHABLE`](crate::domain::content_hash::UNHASHABLE) marker, and
/// no thumbnail is produced.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceRef {
    /// Ingest source (open slug).
    pub kind: SourceKind,
    /// Where the original artefact is, taken apart. Which of the four
    /// shapes it is decides what may be done with it, so the question is
    /// a `match` rather than a string test at each consumer — see
    /// [`SourceLocator`].
    pub locator: SourceLocator,
    /// Size of the original artefact (used as a weight signal in the grid).
    pub file_size_bytes: Option<u64>,
    /// Human-readable name of the platform the asset originated from.
    pub platform: Option<String>,
}

impl SourceRef {
    /// Builds a `SourceRef` from a locator still spelled as text — the
    /// **wire** boundary, where the parse happens exactly once. Fails on
    /// the one string nothing will take (the blank one), which is also
    /// the one thing a NOT NULL TEXT column must not hold.
    ///
    /// The spelling read here is the caller's, not the column's: a path,
    /// `<container>#<record>`, or a URL. A row coming *back* out of the
    /// database is built through
    /// [`SourceLocator::try_from`](crate::domain::source_locator::SourceLocator),
    /// which reads the tagged storage form and nothing else.
    pub fn new(kind: SourceKind, locator: impl AsRef<str>) -> Result<Self, DomainError> {
        Ok(Self::of_locator(
            kind,
            SourceLocator::from_wire(locator.as_ref())?,
        ))
    }

    /// Builds a `SourceRef` from a locator that is already a value —
    /// a producer holding the pieces, or a caller that parsed at its own
    /// boundary and must not re-render to a string on the way here.
    pub fn of_locator(kind: SourceKind, locator: SourceLocator) -> Self {
        Self {
            kind,
            locator,
            file_size_bytes: None,
            platform: None,
        }
    }
}

/// Subject requesting a view of an asset (used to enforce visibility).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Viewer {
    /// The owner of the Asterism instance; sees everything.
    #[default]
    Owner,
    /// A restricted subject, identified by the same token used in the
    /// sharing list of `Visibility::Restricted`.
    Subject(String),
}

/// Visibility of an asset.
///
/// External input formats (for example an `__recipients__` field from a
/// vault primitive) are the concern of the ingest layer; the domain only
/// stores the derived sharing list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Visibility {
    /// Default: visible to the owner and to every subject.
    #[default]
    Open,
    /// Restricted: visible to the owner and to every subject listed in
    /// `sharing`; hidden from all other subject views.
    Restricted {
        /// Subject ids allowed to view this asset (owner access is always
        /// implicit).
        sharing: Vec<String>,
    },
}

impl Visibility {
    /// Whether the asset should be visible to `viewer`. The query layer
    /// translates this predicate into SQL.
    pub fn visible_to(&self, viewer: &Viewer) -> bool {
        match (self, viewer) {
            (_, Viewer::Owner) => true,
            (Visibility::Open, _) => true,
            (Visibility::Restricted { sharing }, Viewer::Subject(subject)) => {
                sharing.iter().any(|s| s == subject)
            }
        }
    }
}

/// Classification axis for a `Tag` (channel). Optional in v1 — tags may be
/// created without an axis and classified later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelAxis {
    /// Time period (for example a month or a season).
    Period,
    /// Counterpart (a person or a persona).
    Counterpart,
    /// Mood / emotional register.
    Mood,
    /// Scene / activity context.
    Scene,
    /// Originating platform.
    Platform,
    /// Modality-derived axis.
    Modality,
}

impl ChannelAxis {
    /// Slug representation shared by the DB schema and the DTO layer.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Period => "period",
            Self::Counterpart => "counterpart",
            Self::Mood => "mood",
            Self::Scene => "scene",
            Self::Platform => "platform",
            Self::Modality => "modality",
        }
    }

    /// Parses a slug (rejects unknown values with a validation error).
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "period" => Ok(Self::Period),
            "counterpart" => Ok(Self::Counterpart),
            "mood" => Ok(Self::Mood),
            "scene" => Ok(Self::Scene),
            "platform" => Ok(Self::Platform),
            "modality" => Ok(Self::Modality),
            other => Err(DomainError::Validation(format!(
                "unknown channel axis: {other:?}"
            ))),
        }
    }
}

/// Job progress payload; the `ProgressEmitter` forwards it to the UI.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Progress {
    /// Items processed so far.
    pub current: u64,
    /// Total item count when known, otherwise `None` (indeterminate).
    pub total: Option<u64>,
    /// Short human-readable status message.
    pub message: Option<String>,
}

/// A paginated result set.
///
/// Repository ports return this type for hot-path listings so the physical
/// representation can be swapped later (for example a columnar / SoA layout)
/// without touching the port signature.
#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    /// Items in the current page.
    pub items: Vec<T>,
    /// Requested offset (echoed back).
    pub offset: u64,
    /// Requested limit (echoed back).
    pub limit: u64,
    /// Total number of matching rows if the adapter computed it.
    pub total: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(values: &[&str]) -> Vec<Label> {
        values.iter().map(|v| Label::new(*v).unwrap()).collect()
    }

    fn label_strings(labels: &[Label]) -> Vec<&str> {
        labels.iter().map(|l| l.as_str()).collect()
    }

    #[test]
    fn dedup_labels_keeps_the_first_copy_and_the_order_around_it() {
        // The fixture disagrees with both defaults it could be confused
        // for: the repeat is *not* adjacent (so a `Vec::dedup` would
        // leave it in), and the surviving order `["b", "a"]` is not the
        // sorted one (so a sort-then-dedup shows up as a failure rather
        // than passing by coincidence).
        let deduped = dedup_labels(labels(&["b", "a", "b"]));
        assert_eq!(
            label_strings(&deduped),
            vec!["b", "a"],
            "first occurrence wins and the rest keeps its order"
        );
    }

    #[test]
    fn dedup_labels_holds_the_head_that_the_label_sort_key_reads() {
        // `sort_eval::first_user_label` takes the first non-internal
        // entry as the Label sort key. Dropping the *first* `zebra` and
        // keeping the last would leave the same set with a different
        // sort key, so the head is asserted on its own.
        let deduped = dedup_labels(labels(&["persona:aya", "zebra", "alpha", "zebra"]));
        assert_eq!(
            label_strings(&deduped),
            vec!["persona:aya", "zebra", "alpha"]
        );
        assert_eq!(
            deduped[1].as_str(),
            "zebra",
            "the first user label is the one the grid sorts on"
        );
    }

    #[test]
    fn dedup_labels_leaves_a_repeat_free_list_untouched() {
        let deduped = dedup_labels(labels(&["inbox", "exporter:comfy"]));
        assert_eq!(label_strings(&deduped), vec!["inbox", "exporter:comfy"]);
        assert!(dedup_labels(Vec::new()).is_empty());
    }

    #[test]
    fn source_kind_for_dispatch_accepts_every_shipping_exporter_slug() {
        // Guard against `DISPATCH_PREFIX` drifting back to `dispatch:`
        // (the 2026-07-19 smoke Test Red: SourceKind grammar rejects
        // `:`, so `dispatch:comfy` blows up deep inside reify).
        for slug in ["comfy", "file", "http", "gemini", "vdsl", "alc-sd-bake"] {
            let source_kind = SourceKind::for_dispatch(slug)
                .unwrap_or_else(|e| panic!("for_dispatch({slug:?}) rejected: {e}"));
            assert!(
                source_kind
                    .as_str()
                    .starts_with(SourceKind::DISPATCH_PREFIX),
                "for_dispatch({slug:?}) should carry the DISPATCH_PREFIX marker"
            );
            assert_eq!(
                source_kind.as_str(),
                &format!("{}{}", SourceKind::DISPATCH_PREFIX, slug)
            );
        }
    }

    #[test]
    fn source_kind_for_dispatch_rejects_grammar_violations() {
        // Any exporter slug whose concatenation would break
        // `[a-z0-9_-]{1,64}` must surface here, not one layer down.
        assert!(SourceKind::for_dispatch("").is_err(), "empty slug");
        assert!(
            SourceKind::for_dispatch("bad:colon").is_err(),
            "`:` is not part of the SourceKind grammar"
        );
        assert!(
            SourceKind::for_dispatch("UPPER").is_err(),
            "uppercase is rejected"
        );
        assert!(
            SourceKind::for_dispatch("has space").is_err(),
            "whitespace is rejected"
        );
    }

    #[test]
    fn modality_accepts_well_known_and_open_slugs() {
        for slug in Modality::well_known() {
            assert!(Modality::new(slug).is_ok());
        }
        assert!(
            Modality::new("image").is_ok(),
            "open slug: adding a consumer is a data change"
        );
        assert!(
            Modality::new("dialogue!").is_err(),
            "non [a-z0-9_-] characters are rejected"
        );
        assert!(Modality::new("").is_err());
    }

    #[test]
    fn cover_template_round_trips_and_rejects_unknown() {
        for tpl in [
            CoverTemplate::FirstLine,
            CoverTemplate::Dialogue,
            CoverTemplate::WorkProduct,
            CoverTemplate::Tape,
        ] {
            assert_eq!(CoverTemplate::parse(tpl.as_str()).unwrap(), tpl);
        }
        assert!(CoverTemplate::parse("neon").is_err());
    }

    #[test]
    fn visibility_restricted_hides_from_unlisted_subjects() {
        let vault = Visibility::Restricted {
            sharing: vec!["alice".into()],
        };
        assert!(
            vault.visible_to(&Viewer::Owner),
            "owner always sees everything"
        );
        assert!(vault.visible_to(&Viewer::Subject("alice".into())));
        assert!(!vault.visible_to(&Viewer::Subject("bob".into())));
        assert!(Visibility::Open.visible_to(&Viewer::Subject("bob".into())));
    }

    #[test]
    fn text_vo_rejects_blank() {
        assert!(Label::new("  ").is_err());
        assert!(
            Label::new("in-review").is_ok(),
            "labels are free-form (status hints, tags, and so on)"
        );
    }

    #[test]
    fn external_id_vos_accept_importer_shapes_and_reject_junk() {
        // Every real-world importer key currently in flight must
        // survive the grammar (Claude Code session UUID, JSONL stem,
        // persona-journal composite, tape file stem).
        for value in [
            "018f8e57-1234-7abc-9def-0123456789ab",
            "persona-journal/aya/state",
            "tape-2026-07-25_003",
            "cc.session.42",
            "a",              // minimum length
            &"a".repeat(256), // maximum length
        ] {
            assert!(
                ExternalSessionKey::new(value).is_ok(),
                "importer-shape {value:?} must round-trip"
            );
            assert!(BundleId::new(value).is_ok(),);
        }
        // Rejected: empty, over-long, whitespace, or characters
        // outside `[\w./:-]`.
        for bad in ["", "has space", "unicode—dash", "tab\tsep", "with\"quote"] {
            assert!(
                ExternalSessionKey::new(bad).is_err(),
                "junk {bad:?} must be rejected"
            );
            assert!(BundleId::new(bad).is_err());
        }
        assert!(ExternalSessionKey::new("a".repeat(257)).is_err());
        assert!(BundleId::new("a".repeat(257)).is_err());
    }

    #[test]
    fn parsing_normalises_parameters_and_case() {
        // The whole point of parsing at one place: this spelling used
        // to be a PNG to `content_region` (the only caller that
        // normalised) and an unknown format to everyone else.
        let m = MimeType::parse("IMAGE/PNG; charset=binary");
        assert_eq!(m, MimeType::Image(ImageFormat::Png));
        // Normalised on the way in, so the stored token is canonical.
        assert_eq!(m.as_str(), "image/png");
        assert_eq!(
            MimeType::parse("  video/webm  "),
            MimeType::parse("video/webm")
        );
    }

    #[test]
    fn an_unnamed_subtype_keeps_its_family() {
        // `image/x-icon` is in no list this codebase keeps, and the
        // string form still treated it as an image (`starts_with`).
        // Dropping it into `Other` would silently stop it tiling — a
        // regression the parse must not introduce.
        let icon = MimeType::parse("image/x-icon");
        assert!(icon.thumbnailable());
        assert_eq!(icon.media(), MediaKind::Image);
        // And it round-trips: the token is what an HTTP Content-Type
        // sends back, so it cannot be rewritten to a named format.
        assert_eq!(icon.as_str(), "image/x-icon");

        let ogg = MimeType::parse("video/ogg");
        assert!(ogg.thumbnailable());
        assert_eq!(ogg.media(), MediaKind::Video);
        // An unnamed video is not assumed unplayable: only the three
        // measured containers take the rendition path.
        assert!(!ogg.needs_video_preview());
    }

    #[test]
    fn only_text_reads_as_body_text() {
        // The check `index_rebuild` did not have. A PNG answering
        // `true` here is the 2026-08-05 defect: 5,000 images read as
        // lossy UTF-8 into the full-text index.
        assert!(!MimeType::parse("image/png").body_text());
        assert!(!MimeType::parse("video/mp4").body_text());
        assert!(!MimeType::parse("audio/mpeg").body_text());
        // Formats that *contain* text are not text either — extracting
        // those is a decoder's job, not a lossy read of the container.
        assert!(!MimeType::parse("application/pdf").body_text());

        assert!(MimeType::parse("text/plain").body_text());
        // Any `text/*`, including subtypes no list names.
        assert!(MimeType::parse("text/markdown").body_text());
    }

    #[test]
    fn the_preview_and_frame_grab_sets_cannot_drift_apart() {
        // These were two copies of the same three literals, in
        // `render::needs_video_preview` and `thumb_ffmpeg::route_for`.
        // Adding a format to one and not the other is silent: the file
        // plays as a crossed-out player, or tiles as an empty card.
        for raw in ["video/webm", "video/x-matroska", "video/x-msvideo"] {
            let MimeType::Video(f) = MimeType::parse(raw) else {
                panic!("{raw} must parse as a video");
            };
            assert!(f.webview_cannot_play(), "{raw} needs a rendition");
            assert!(f.needs_external_frame_grab(), "{raw} needs ffmpeg");
        }
        for raw in ["video/mp4", "video/quicktime"] {
            let MimeType::Video(f) = MimeType::parse(raw) else {
                panic!("{raw} must parse as a video");
            };
            assert!(!f.webview_cannot_play());
            assert!(!f.needs_external_frame_grab());
        }
    }

    #[test]
    fn every_known_mime_parses_to_a_named_variant() {
        // The tripwire the two `KNOWN_*_MIMES` lists carried, moved to
        // the type: a format named in the list but missing a parse arm
        // would land in `Other` and lose whatever the named variant
        // decides (PNG's content walk, WebM's rendition route).
        use crate::domain::material::{KNOWN_IMAGE_MIMES, KNOWN_VIDEO_MIMES};

        for raw in KNOWN_IMAGE_MIMES {
            match MimeType::parse(raw) {
                MimeType::Image(ImageFormat::Other(_)) => {
                    panic!("{raw} is in KNOWN_IMAGE_MIMES but has no parse arm")
                }
                MimeType::Image(_) => {}
                other => panic!("{raw} parsed as {other:?}, not an image"),
            }
        }
        for raw in KNOWN_VIDEO_MIMES {
            match MimeType::parse(raw) {
                MimeType::Video(VideoFormat::Other(_)) => {
                    panic!("{raw} is in KNOWN_VIDEO_MIMES but has no parse arm")
                }
                MimeType::Video(_) => {}
                other => panic!("{raw} parsed as {other:?}, not a video"),
            }
        }
    }

    #[test]
    fn a_value_in_no_family_is_kept_verbatim() {
        let pdf = MimeType::parse("application/pdf");
        assert_eq!(pdf, MimeType::Other("application/pdf".into()));
        assert_eq!(pdf.as_str(), "application/pdf");
        assert_eq!(pdf.media(), MediaKind::None);
        assert!(!pdf.thumbnailable());
    }
}
