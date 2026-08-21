//! Repository ports — the persistence traits declared here and implemented
//! in `asterism-infra` (dependency inversion: trait declarations belong to
//! the consuming crate).
//!
//! **The raw layer's ports, and only those.** The forge's are in
//! [`domain::forge::repository`](crate::domain::forge::repository), so
//! that adding one here does not mean opening the file that holds the
//! forge's. The raw layer needs nothing of a pursuit.
//!
//! The rule the whole tree is measured against is one verb — *uses* —
//! and it is stated once, in [`domain`](crate::domain). Doc links
//! pointing at forge paths are prose about the boundary rather than a
//! crossing of it; a `use` is the thing that counts.
//!
//! Every trait is `Send + Sync` because Tauri v2 uses a multi-threaded
//! tokio runtime. Hot-path list / search methods return the `AssetCard`
//! projection instead of full `Asset` entities.

use async_trait::async_trait;

use chrono::{DateTime, Utc};

use crate::domain::app_setting::{AppSetting, SettingKey};
use crate::domain::asset::{Asset, AssetCard, AssetQuery};
use crate::domain::asset_comment::AssetComment;
use crate::domain::chapter_mark::ChapterMark;
use crate::domain::dir::Dir;
use crate::domain::dispatch::DispatchJob;
use crate::domain::duplicate_conflict::{ConflictResolution, DuplicateAxis, DuplicateConflict};
use crate::domain::edge::{ConstellationEdge, EdgeKind, IncidentEdge};
use crate::domain::group::{Group, GroupLink, GroupSummary};
use crate::domain::instance::InstanceIdentity;
use crate::domain::job::JobKind;
use crate::domain::material_layer::{LayerRole, MaterialLayer};
use crate::domain::material_mark::MaterialMark;
use crate::domain::measurement::Measurement;
use crate::domain::merge_plan::MergePlan;
use crate::domain::modality::{ModalityDef, ModalityView};
use crate::domain::persona::Persona;
use crate::domain::persona_profile::PersonaProfile;
use crate::domain::persona_theme::PersonaTheme;
use crate::domain::series::{SeriesKey, Strategy};
use crate::domain::session::{Session, SessionMetadataPatch};
use crate::domain::snapshot::Snapshot;
use crate::domain::source_locator::SourceLocator;
use crate::domain::tag::{Tag, TagCount, TagMergeOutcome};
use crate::domain::thread::{Message, Thread, ThreadAnchor};
use crate::domain::value::{
    AssetCommentId, AssetId, ChapterMarkId, DirId, DispatchId, DuplicateConflictId,
    ExternalSessionKey, GroupId, MaterialLayerId, MaterialMarkId, MessageId, MimeType, Modality,
    PackId, Page, PersonaId, Progress, SessionId, SnapshotId, SourceKind, StrategyId, TagId,
    ThreadId,
};
use crate::error::DomainError;

/// Persistence port for [`Persona`].
#[async_trait]
pub trait PersonaRepository: Send + Sync {
    /// Fetches one persona by surrogate id.
    async fn find(&self, id: &PersonaId) -> Result<Option<Persona>, DomainError>;

    /// Fetches one persona by external pack id (unique when present).
    async fn find_by_pack_id(&self, pack_id: &PackId) -> Result<Option<Persona>, DomainError>;

    /// Returns every persona. Personas are expected to number in the tens
    /// or low hundreds, so pagination is unnecessary here.
    async fn list(&self) -> Result<Vec<Persona>, DomainError>;

    /// Upserts (replaces the row that matches the id).
    async fn save(&self, persona: &Persona) -> Result<(), DomainError>;

    /// Stamps the persona as trashed at `at` and returns the
    /// **effective** stamp — `at` on the first call, the original stamp
    /// on any repeat. `NotFound` when the id is unknown.
    ///
    /// Returning the stamp rather than `()` is what keeps the persona
    /// and its assets on the same key: the caller
    /// (`PersonaService::trash`) hands this value to
    /// [`AssetRepository::trash_by_persona`], so a re-run stamps
    /// whatever assets are still live with the *original* timestamp
    /// instead of minting a second one that `restore_by_persona` would
    /// never match.
    async fn trash(&self, id: &PersonaId, at: DateTime<Utc>) -> Result<DateTime<Utc>, DomainError>;

    /// Clears the trash stamp. Idempotent; `NotFound` when unknown.
    async fn restore(&self, id: &PersonaId) -> Result<(), DomainError>;

    /// Reads the persona's current trash stamp (`None` when live or
    /// absent). The restore path needs it to identify exactly which
    /// assets went down with this persona.
    async fn trashed_at(&self, id: &PersonaId) -> Result<Option<DateTime<Utc>>, DomainError>;

    /// Physically deletes an **already-trashed** persona; the FK cascade
    /// takes its assets, groups, snapshots and dispatch history.
    /// `Conflict` when the persona is still live — as everywhere else,
    /// purge is reachable only through the trash. A missing id is a
    /// no-op.
    async fn purge(&self, id: &PersonaId) -> Result<(), DomainError>;

    /// Lists trashed personas whose stamp is older than `cutoff`, oldest
    /// first, capped at `limit`. Sibling of the asset / group scans.
    async fn scan_purgeable(
        &self,
        cutoff: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<PersonaId>, DomainError>;
}

/// Persistence port for [`PersonaTheme`]. The theme is a 1:1 side
/// aggregate of `Persona`, deliberately kept in a separate port so the
/// UI chrome data (wallpaper reference and future decoration fields)
/// does not leak into the write path of the identity entity.
///
/// Adapters should treat "no row" and "row with `wallpaper_asset_id =
/// NULL`" as different states: the first is "user never set a theme"
/// and returns `None`; the second is "user cleared the wallpaper" and
/// returns `Some(theme_with_none_wallpaper)`.
#[async_trait]
pub trait PersonaThemeRepository: Send + Sync {
    /// Fetches the theme for a persona. `None` when no row exists.
    async fn get(&self, persona_id: &PersonaId) -> Result<Option<PersonaTheme>, DomainError>;

    /// Upserts the theme (idempotent — the persona_id is the primary
    /// key). Callers stamp `updated_at`; adapters do not clock.
    async fn upsert(&self, theme: &PersonaTheme) -> Result<(), DomainError>;

    /// Deletes the theme row. Idempotent — missing row is a no-op.
    async fn delete(&self, persona_id: &PersonaId) -> Result<(), DomainError>;
}

/// Persistence port for [`PersonaProfile`]. Kept apart from
/// [`PersonaThemeRepository`] so identity metadata (avatar, bio,
/// role) and UI chrome (wallpaper) can evolve on independent
/// commit paths. Adapters treat "no row" as `None`; a row where
/// every field is `NULL` is still `Some(profile)` — the user has
/// explicitly cleared their notes rather than never having set
/// them.
#[async_trait]
pub trait PersonaProfileRepository: Send + Sync {
    /// Fetches the profile for a persona. `None` when no row exists.
    async fn get(&self, persona_id: &PersonaId) -> Result<Option<PersonaProfile>, DomainError>;

    /// Upserts the profile (idempotent — the persona_id is the
    /// primary key). Callers stamp `updated_at`.
    async fn upsert(&self, profile: &PersonaProfile) -> Result<(), DomainError>;

    /// Deletes the profile row. Idempotent — missing row is a no-op.
    async fn delete(&self, persona_id: &PersonaId) -> Result<(), DomainError>;
}

/// Persistence port for the [`Session`] entity — the Dialog-modality
/// 1st-class aggregate that replaces the old `asset.session_id`
/// projection.
///
/// The port carries just the primitives P1a's importer + P1b's HTTP
/// CRUD surface need: find by surrogate id, find by
/// `(persona_id, external_key)` (the importer's re-entry key), create,
/// metadata update, list-by-persona for the SessionsView, and
/// `delete_if_empty` (rejects when any asset still references the
/// Session — orphan delete is forbidden, mirroring the Modality
/// delete guard).
///
/// # Adapter contract
///
/// - `create` is a **find-or-create**: it hands back the row that holds
///   `(persona_id, external_key)` if one already does, and mints
///   otherwise. It does not fail on a second arrival, and the adapter
///   must make the lookup and the insert atomic *itself* — no index
///   asserts this. `external_key` is a Prop, not an identity, so
///   nothing in the schema refuses a repeat.
/// - `update` writes the caller-supplied `metadata` verbatim and
///   bumps `updated_at_ms`; the derived aggregates
///   (`started_at_ms` / `ended_at_ms` / `message_count`) are left
///   untouched (the `SessionRebuild` job in P1b owns them).
/// - `delete_if_empty` returns `Conflict` when any `asset` row
///   still carries the Session id in its `session_id` column.
///   **Trashed assets count as still attached**: the `session_id` FK is
///   `ON DELETE SET NULL`, so removing the Session under a trashed
///   asset would silently drop that asset's Session link and restore
///   would come back detached.
#[async_trait]
pub trait SessionRepository: Send + Sync {
    /// Fetches one Session by surrogate id (`None` when absent).
    async fn find_by_id(&self, id: &SessionId) -> Result<Option<Session>, DomainError>;

    /// Fetches one Session by its importer-visible external key,
    /// scoped to the owning persona. Returns `None` when the pair
    /// is not registered — the importer treats that as "mint a new
    /// Session via `create`".
    async fn find_by_external_key(
        &self,
        persona_id: &PersonaId,
        external_key: &ExternalSessionKey,
    ) -> Result<Option<Session>, DomainError>;

    /// Find-or-create keyed on `(persona_id, external_key)`.
    ///
    /// **Returns the row that holds the key.** When one already exists
    /// the stored row is returned as it stands and `session` is
    /// discarded — nothing is written, and in particular the caller's
    /// seed window and metadata do not overwrite what is there. When
    /// none exists the row is minted and `session` is returned.
    ///
    /// A second arrival is not an error. `external_key` is a Prop: an
    /// external record legitimately arrives more than once, and ids from
    /// different platforms collide, so no uniqueness can be asserted on
    /// it (V62 took the UNIQUE off).
    ///
    /// **The adapter owns the atomicity.** Since V62 no index refuses a
    /// repeat, so an implementation that looks up and then inserts in
    /// two separate round trips would let two concurrent callers both
    /// insert. The SQLite adapter does both inside one writer-isle
    /// closure, which serialises them.
    async fn create(&self, session: &Session) -> Result<Session, DomainError>;

    /// Partially updates the user-editable metadata (`title` / `note`
    /// / `cover_hint`) of an existing Session and bumps `updated_at_ms`
    /// to `now`. `None` fields on `patch` leave the existing value
    /// intact (SQL `COALESCE`); `Some(v)` overwrites. Fails with
    /// `Conflict` when the id is not registered (no row updated).
    /// Derived aggregates are left untouched.
    ///
    /// Clearing `title` back to `NULL` is expressed through
    /// [`rename`](Self::rename) rather than this port — the patch
    /// shape has no "explicit null" leg to keep the wire form flat.
    async fn update_metadata(
        &self,
        id: &SessionId,
        patch: &SessionMetadataPatch,
        now: DateTime<Utc>,
    ) -> Result<Session, DomainError>;

    /// Sets the Session's title to `new_title` explicitly, including
    /// `None` to clear it (this is the sole "back to untitled" write
    /// path — [`update_metadata`](Self::update_metadata) is patch-only
    /// and cannot express NULL). Bumps `updated_at_ms` to `now`. Fails
    /// with `Conflict` when the id is not registered. `note` /
    /// `cover_hint` are left untouched.
    async fn rename(
        &self,
        id: &SessionId,
        new_title: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<Session, DomainError>;

    /// Deletes the Session iff no `asset` row still references its
    /// id in the `session_id` column. Fails with `Conflict` when the
    /// guard trips (asset(s) still attached) or when the id is not
    /// registered.
    async fn delete_if_empty(&self, id: &SessionId) -> Result<(), DomainError>;

    /// Lists every Session belonging to `persona_id`, ordered by
    /// `started_at_ms DESC` (freshest first, matching the SessionsView
    /// tile order). Persona-scoped — the SessionsView never renders
    /// cross-persona sessions.
    async fn list_by_persona(&self, persona_id: &PersonaId) -> Result<Vec<Session>, DomainError>;
}

/// Persistence port for the instance identity record (`instance`
/// table) — the referent behind
/// [`Author::Owner`](crate::domain::attribution::Author::Owner).
///
/// The row is minted by the migration that creates the table, so a
/// migrated database always has exactly one; the port reads it and,
/// once, binds the owner subject.
#[async_trait]
pub trait InstanceRepository: Send + Sync {
    /// Reads the identity record.
    ///
    /// `None` means the row is absent, which a migrated database cannot
    /// produce — the read surfaces the anomaly rather than inventing an
    /// identity, because a minted id that disagrees with the one on disk
    /// would silently split the instance in two.
    async fn get(&self) -> Result<Option<InstanceIdentity>, DomainError>;

    /// Binds the owner subject.
    ///
    /// Called once, when authentication first establishes who the owner
    /// is. Rows written before the binding keep whatever they recorded:
    /// resolving them is a name-matching question, not something this
    /// write may decide.
    async fn bind_owner(&self, subject: &str) -> Result<(), DomainError>;
}

/// Persistence port for stored setting overrides (`app_setting` table).
///
/// Only overrides are persisted: a key the user has never changed has no
/// row, and the application layer resolves it from the closed registry
/// instead. That keeps "reset to default" a `DELETE` rather than a write
/// of a magic value, and it lets a default change in code reach every
/// profile that had not overridden the key.
#[async_trait]
pub trait AppSettingRepository: Send + Sync {
    /// Lists every stored override. A row the running build cannot
    /// interpret — an unknown key written by a newer build, or an
    /// unrepresentable timestamp — is skipped rather than failing the
    /// whole read, so one bad row cannot blank the settings screen.
    async fn list(&self) -> Result<Vec<AppSetting>, DomainError>;

    /// Fetches one stored override; `None` means "not overridden".
    /// A row that cannot be interpreted also reads as `None`, matching
    /// [`Self::list`] — the two paths must not disagree about whether a
    /// key is overridden.
    async fn find(&self, key: SettingKey) -> Result<Option<AppSetting>, DomainError>;

    /// Inserts or replaces the override for `setting.key`.
    async fn upsert(&self, setting: &AppSetting) -> Result<(), DomainError>;

    /// Removes the override, returning the key to its registry default.
    /// Deleting an absent key is a no-op, not an error — "make it the
    /// default" is idempotent.
    async fn delete(&self, key: SettingKey) -> Result<(), DomainError>;
}

/// Persistence port for the [`ModalityDef`] master (`modality` table).
///
/// The master is an **open** set: rows are added / edited / hidden /
/// removed at runtime. Deletion is guarded to `asset_count ==
/// 0` at the application layer — this port exposes the primitives that
/// guard needs (`asset_count`) plus the CRUD writes. `asset.modality`
/// carries no FK to this table (the importer escape hatch keeps
/// unregistered slugs valid), so `find` returning `None` is a normal
/// "unregistered slug" signal, not an error.
#[async_trait]
pub trait ModalityRepository: Send + Sync {
    /// Lists every master row with its live asset count, ordered by
    /// `sort_order` then `slug` (hidden rows included — the caller
    /// decides whether to render them).
    async fn list(&self) -> Result<Vec<ModalityView>, DomainError>;

    /// Fetches one master row by slug (`None` when the slug is not
    /// registered — a valid state for importer-supplied slugs).
    async fn find(&self, slug: &Modality) -> Result<Option<ModalityDef>, DomainError>;

    /// Inserts a new master row. Fails with
    /// [`DomainError::Conflict`](crate::error::DomainError::Conflict)
    /// when the slug already exists (primary-key violation → `409`).
    async fn create(&self, def: &ModalityDef) -> Result<(), DomainError>;

    /// Overwrites an existing master row (full-row write of the
    /// caller-resolved [`ModalityDef`]). Fails with `Conflict` when the
    /// slug does not exist.
    async fn update(&self, def: &ModalityDef) -> Result<(), DomainError>;

    /// Deletes a master row. The application layer must have already
    /// verified `asset_count == 0`; this method fails with `Conflict`
    /// when the slug does not exist (no row removed).
    async fn delete(&self, slug: &Modality) -> Result<(), DomainError>;

    /// Counts `asset` rows carrying `slug` (the delete guard input).
    ///
    /// **Counts trashed assets too.** This is a referential-integrity
    /// guard, not a display count: dropping the master row while a
    /// trashed asset still carries the slug would leave that asset
    /// pointing at a missing modality once it is restored. The
    /// user-facing per-modality count on
    /// [`list`](Self::list) excludes trashed rows instead.
    async fn asset_count(&self, slug: &Modality) -> Result<u64, DomainError>;
}

/// One row of a dimension-measuring pass.
///
/// Sibling of [`UnhashedMaterial`], one table over: dimensions live on
/// the asset, so this walk is per-asset and reads the asset's own
/// locator rather than a material's.
///
/// **That locator is the point, not a convenience.** It is the path the
/// importer declared and therefore the bytes the importer measured at
/// ingest, so a re-measure reading it reaches the same evidence the
/// ingest path did. Measuring some other artefact of the same asset —
/// a derived material, a thumbnail — would put a second meaning in the
/// column, told apart from the first by nothing.
#[derive(Debug, Clone)]
pub struct DimsCandidate {
    /// The asset whose columns are being filled.
    pub asset_id: AssetId,
    /// Where its original bytes are. `SourceLocator`, not a string, so
    /// this pass asks the same "is there a local file" question
    /// `hash_material` does and gets it answered by the type rather
    /// than by a prefix test.
    pub locator: SourceLocator,
}

/// Which rows a measuring pass is about.
///
/// The **selection** half of the pair this vocabulary splits (the other
/// is [`DimsWritePolicy`]). Both used to be implicit — one predicate and
/// one overwrite rule, baked into the repository — which meant every
/// caller inherited the conservative reading the startup walk needs, and
/// a caller that wanted anything else got silence instead of an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimsScope {
    /// `dims_probed_at IS NULL` — rows nothing has looked at yet.
    ///
    /// The startup walk's scope, and the only one that terminates for
    /// good: a row is offered once, whatever came back. See the V71
    /// migration for the three states that separates.
    Unlooked,
    /// `width_px IS NULL` — rows that carry no measurement, whether or
    /// not something has already tried.
    ///
    /// The scope for "the situation changed": a volume that was not
    /// mounted is mounted now, a parser learned a container it used to
    /// reject. Re-offers rows the startup walk has retired, which is the
    /// point — the stamp records that nothing has to look again *on its
    /// own*, not that nobody may.
    Unmeasured,
    /// Every row.
    ///
    /// The scope for "the measurement changed" — a fixed transposition,
    /// a parser that now answers differently for files it always read.
    /// Only meaningful with [`DimsWritePolicy::Overwrite`]; paired with
    /// `FillOnly` it reads every artefact in the library and changes
    /// nothing.
    All,
}

impl DimsScope {
    /// The wire spelling.
    ///
    /// Paired with [`parse`](Self::parse) so the slug a pass
    /// chain-enqueues and the slug the next page accepts cannot drift
    /// apart — a mismatch there would let a force pass revert to the
    /// default scope halfway through its own walk.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unlooked => "unlooked",
            Self::Unmeasured => "unmeasured",
            Self::All => "all",
        }
    }

    /// Reads the wire spelling, refusing anything else.
    ///
    /// An unknown slug is an error rather than a fall back to the
    /// default: the difference between the scopes is the difference
    /// between "fill the gaps" and "replace every answer", and a
    /// mistyped one that quietly became `Unlooked` would report success
    /// for a pass that did nothing the caller asked for.
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "unlooked" => Ok(Self::Unlooked),
            "unmeasured" => Ok(Self::Unmeasured),
            "all" => Ok(Self::All),
            other => Err(DomainError::Validation(format!(
                "unknown dims scope: {other:?} (expected unlooked, unmeasured or all)"
            ))),
        }
    }
}

/// What a measuring pass may do to a row that already has an answer.
///
/// The **write** half of the pair (see [`DimsScope`]). Explicit because
/// the two callers want opposite things and neither is a sensible
/// default for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimsWritePolicy {
    /// Fill the columns only if they are empty.
    ///
    /// For passes nobody asked for. An ingest measurement that landed
    /// between the scan and the write came off the artefact at import
    /// time and is the better evidence, so a background sweep must not
    /// step on it.
    FillOnly,
    /// Replace whatever is there.
    ///
    /// For passes somebody asked for — a person who replaced the file, a
    /// caller that knows the measurement itself has changed. Their
    /// request is newer information than the stored value, which is
    /// exactly what `FillOnly` cannot express.
    Overwrite,
}

/// What reading an artefact's bytes produced.
///
/// Three values rather than `Option`, because "no dimensions" hides two
/// facts that have to be recorded differently:
///
/// - [`NothingToMeasure`](Self::NothingToMeasure) is permanent. A text
///   note has no pixels; a container no probe reads will not grow them
///   by being asked again. Recording it is what lets a walk finish.
/// - [`Unreadable`](Self::Unreadable) is a statement about **now**. An
///   unmounted volume, a locked file, a path that has moved. Recording
///   it as an answer would retire the row permanently on the strength of
///   a temporary condition — a library on an external disk, measured
///   once while the disk was out, would never be measurable again.
///
/// `hash_material` draws the same line and for the same reason: it marks
/// a locator with no local bytes and stays silent about a read that
/// failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimsProbe {
    /// The bytes stated a coded pixel pair.
    Measured(u32, u32),
    /// The bytes were read and state no dimensions.
    NothingToMeasure,
    /// The bytes could not be read this time.
    Unreadable,
}

/// One material whose fingerprints are already written — the unit the
/// duplicate re-scan walks.
///
/// Sibling of [`UnhashedMaterial`], and deliberately **without a
/// locator**: this pass re-derives conflicts from digests the row
/// already holds and never opens a file. That is the whole difference in
/// cost between it and the hashing walk, and the reason it is a separate
/// job rather than a scope on that one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintedMaterial {
    /// Owning asset.
    pub asset_id: AssetId,
    /// Position within that asset (`0` = primary original).
    pub ord: u32,
    /// The three axes as the row holds them, in the shape
    /// [`detect_duplicate`](crate::application_support::duplicate_detection::detect_duplicate)
    /// takes — so the re-scan hands it exactly what the hashing pass
    /// handed it, and the two cannot disagree about what was measured.
    pub fingerprint: MaterialFingerprint,
}

/// One material still waiting for its fingerprints — the unit the
/// backfill job walks.
///
/// Carries the locator because hashing needs the path, and the
/// `(asset_id, ord)` key because that is what the result is written
/// back against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnhashedMaterial {
    /// Owning asset.
    pub asset_id: AssetId,
    /// Position within that asset (`0` = primary original).
    pub ord: u32,
    /// Where the bytes are.
    ///
    /// The same type [`Material::locator`](crate::domain::material::Material::locator)
    /// carries, for the same reason the `mime` field below is the same
    /// type: `hash_material` is reached from this walk and from the
    /// per-asset pass, and if the two handed it different readings of
    /// one locator they would disagree about the same artefact.
    pub locator: SourceLocator,
    /// What the row believes the format is — the material's `mime`,
    /// parsed at the same boundary the entity's is, `None` when the
    /// extension named nothing.
    ///
    /// Here because the content axis needs it and this walk has no
    /// entity to read it off. The per-asset pass holds the whole asset
    /// and takes the same field from it, which is what makes the two
    /// passes agree; deriving it here from the locator instead would be
    /// a second implementation of `guess_mime`, and the day the two
    /// disagreed the same file would fingerprint differently depending
    /// on which pass reached it first.
    pub mime: Option<MimeType>,
}

/// One material no chapter reading has reached yet — the unit the
/// `ChapterScan` backfill walks.
///
/// The same three fields [`UnhashedMaterial`] carries and for the same
/// reasons: the locator is what gets opened, the `(asset_id, ord)` key
/// is what the resulting band is filed under, and the `mime` travels
/// with the row because the walk has no entity to read it off.
///
/// A separate type rather than a reuse of that one, because the two
/// select different populations under predicates that will not stay
/// parallel — and a shared struct is the shape in which a change to one
/// walk silently becomes a change to the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterScanCandidate {
    /// Owning asset.
    pub asset_id: AssetId,
    /// Position within that asset (`0` = primary original).
    pub ord: u32,
    /// Where the bytes are.
    pub locator: SourceLocator,
    /// What the row believes the format is. Carried even though the
    /// scan already filtered on it, so the handler decides eligibility
    /// through
    /// [`MimeType::carries_chapters`](crate::domain::value::MimeType::carries_chapters)
    /// on both routes rather than trusting one caller's filter.
    pub mime: Option<MimeType>,
}

/// The values one fingerprint pass produces for one material.
///
/// A struct rather than loose arguments because the invariant is that
/// they travel together. They come from a single read of the file and
/// are written by a single statement, so there is no moment at which
/// one is known and the other is not — and positional parameters of the
/// same type are the shape that lets a caller swap them and a compiler
/// agree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialFingerprint {
    /// File axis: the status, the `sha256:<hex>` digest when there is
    /// one, and the reason when the status carries one
    /// ([`Measurement`](crate::domain::measurement::Measurement)).
    pub file: Measurement,
    /// Content axis: the status, and the `cr1-sha256:<hex>` digest when
    /// there is one (`crate::domain::content_region`).
    pub content: Measurement,
    /// Meta axis: the status, and the `m1-sha256:<hex>` digest when
    /// there is one (`crate::domain::material_meta`).
    pub meta: Measurement,
    /// The canonical metadata object the meta digest was taken over —
    /// `Some` exactly when [`meta`](Self::meta) is a digest.
    ///
    /// Carried beside the digest rather than derived later because it
    /// **is** the thing that was hashed: the digest is the index a
    /// lookup uses, and this is what a person reads and a field
    /// comparison walks ("made the same way apart from *this*"). Two
    /// values, one measurement, written by one statement, so no reader
    /// can see a digest whose body says something else.
    pub meta_kv: Option<String>,
    /// The container's metadata bytes, base64 under
    /// [`RAW_PREFIX`](crate::domain::material_meta_raw::RAW_PREFIX), or a
    /// marker — the value
    /// [`MetaRaw::stored_value`](crate::domain::material_meta_raw::MetaRaw::stored_value)
    /// produces. `None` is the column staying NULL: nothing here keeps
    /// bytes for this format.
    ///
    /// Travels with the other three because it comes from the same read
    /// and is written by the same statement. It is not a fourth axis —
    /// nothing groups on it and nothing compares it — it is what
    /// [`meta_kv`](Self::meta_kv) was derived from, kept so the
    /// derivation can be replaced without reading every file in a
    /// library again (`material_meta_raw`).
    ///
    /// **A pass that reads a row back rather than measuring it may
    /// legitimately leave this `None`.** The re-scan behind duplicate
    /// detection is the one that does: it rebuilds this struct from
    /// stored columns to re-derive conflicts, and selecting a payload
    /// that can reach a megabyte a row for a walk that only compares
    /// digests would be paid on every page. Such a caller never writes
    /// the struct back, which is what keeps that omission from clearing
    /// the column.
    pub meta_raw: Option<String>,
    /// The words the container wrote into the artefact, recovered for
    /// search (`crate::domain::embedded_text`) — the canonical object,
    /// or `None` when this pass did not look.
    ///
    /// Not a fourth axis and deliberately not a digest. Nothing groups
    /// on it; it is the *document* side of the same chunks the meta
    /// digest is taken over, read generously where that one has to be
    /// frozen — `zTXt` and `iTXt` included, Latin-1 recovered rather
    /// than replaced.
    ///
    /// Here rather than in a pass of its own because the bytes are
    /// already in the buffer. The alternative is opening every picture
    /// again from the job that composes documents, which is the read
    /// this whole struct exists to avoid doing twice.
    ///
    /// `Some("{}")` is a real answer — "read, and these bytes carry no
    /// words" — and is what keeps a backfill from reading such a file
    /// on every pass. `None` reaches the column as SQL `NULL`, which
    /// means nobody has looked.
    pub meta_text: Option<String>,
}

/// A set of live assets that share one content fingerprint.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateGroup {
    /// Which fingerprint they agreed on.
    ///
    /// Set by the adapter that chose the column to group by, not by the
    /// service reading the result: "which axis is this" is a fact about
    /// the query that ran, and a caller restating it would be a second
    /// place to keep in step. Both values are produced now — the
    /// caller asks for one axis per call and gets it back on every
    /// group, so a report that was rendered under one axis cannot be
    /// read as the other.
    pub axis: DuplicateAxis,
    /// The fingerprint they agree on, on `axis`: `sha256:<hex>` on the
    /// file axis, `cr1-sha256:<hex>` on the content one.
    ///
    /// The algorithm tag is part of the value, so the two vocabularies
    /// cannot be confused for one another and
    /// [`axis_of`](crate::domain::content_hash::axis_of) reads the axis
    /// back out of a bare key.
    pub content_hash: String,
    /// The members, oldest first — the order a "keep the first one"
    /// reading of the group expects.
    pub members: Vec<AssetCard>,
}

/// What one call to [`AssetRepository::fold_into`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldOutcome {
    /// The headstone was stood up and the structure moved.
    Folded(FoldReport),
    /// Nothing was written. The pair failed the re-read the fold does
    /// on its way in, and the variant says which half of it.
    Skipped(FoldRefusal),
}

/// Counts from a fold that went through. Every number is a row count,
/// so a caller can tell "folded an isolated row" from "moved a card
/// that was filed in nine places" without a second read — which is
/// what the job's log line is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FoldReport {
    /// Edges that now name the keeper on the side that named the
    /// headstone.
    pub edges_repointed: u64,
    /// Edges that could not move and were removed instead: the pair's
    /// own edges (which would have become a keeper→keeper self-loop)
    /// and those whose `(from, to, kind)` the keeper already had. What
    /// each of them claimed is written to the headstone's `_trace`
    /// before it goes.
    pub edges_dropped: u64,
    /// Group memberships the keeper gained. Excludes the buckets it was
    /// already in — those keep the keeper's own position.
    pub buckets_moved: u64,
    /// Rows that were filed inside the headstone and are now filed
    /// inside the keeper.
    pub children_repointed: u64,
    /// Tag links the keeper gained (again excluding tags it already
    /// carried).
    pub tags_moved: u64,
    /// Comments that hung off the headstone and now hang off the
    /// keeper. They keep their own timestamps, so the keeper's thread
    /// reads as one chronology rather than two appended blocks.
    pub comments_moved: u64,
    /// Threads anchored on the headstone's card that are now anchored
    /// on the keeper's.
    pub threads_reanchored: u64,
    /// Columns of the keeper the merge rewrote (see
    /// [`fold_into`](AssetRepository::fold_into) for which columns can
    /// be among them, and by what rule).
    pub columns_merged: u64,
    /// Values the headstone held where the keeper's own stood: the
    /// columns no rule combines, plus its position in each Group the
    /// keeper was already filed in. Each one is written to the keeper's
    /// `_trace` before the fold ends — a keeper-wins rule still has to
    /// say what it did not take.
    pub values_discarded: u64,
}

/// Why a fold wrote nothing.
///
/// Every variant is a legitimate outcome rather than an error: the fold
/// is enqueued by a decision taken earlier, and the world can change
/// between the decision and the job (the re-read exists exactly for
/// that gap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldRefusal {
    /// The row to fold is already a headstone. A second fold of the
    /// same pair lands here, and so does the loser of two workers
    /// racing to fold two rows into each other.
    AlreadyFolded,
    /// The row to fold is gone.
    Missing,
    /// The keeper is gone.
    KeeperMissing,
    /// The keeper is itself a headstone. Folding into it would build a
    /// chain every reader would have to walk.
    KeeperFolded,
    /// The keeper is in the trash, where retention physically deletes
    /// it — the structure would be moved onto a row on its way out.
    KeeperTrashed,
    /// Both ids are the same row.
    SameAsset,
}

/// What one call to [`AssetRepository::merge_into`] did — or, on a dry
/// run, would have done.
///
/// A struct rather than an enum, unlike [`FoldOutcome`], because a merge
/// is not one decision: several rows are folded and each one is
/// separately capable of being refused, so "it happened" and "it was
/// refused" are counts here rather than alternatives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    /// Rows this call folded, in fold order.
    ///
    /// The ids are carried rather than counted because the application
    /// verb hands them back to the person who ruled the set — the panel
    /// wants to say which rows moved, not just how many — and only the
    /// verb that did the fold can tell which of the plan's discards
    /// landed here as opposed to [`already_folded`](Self::already_folded)
    /// or [`refusals`](Self::refusals). Reconstructing that outside
    /// would mean a second read of `folded_into` on every candidate,
    /// racing the transaction that produced these numbers — the same
    /// shape the port doc says the whole verb is built to avoid.
    pub folded: Vec<AssetId>,
    /// Rows that were **already folded into this keeper** when the call
    /// ran, in the order they were reached, which the verb treats as
    /// the plan already being true rather than as a refusal — see the
    /// port doc. Carried as ids for the reason
    /// [`folded`](Self::folded) is.
    pub already_folded: Vec<AssetId>,
    /// The counts of every fold that went through, added together.
    ///
    /// Every field is a row count, so the sum is the same kind of number
    /// the parts are, with one thing to know:
    /// [`columns_merged`](FoldReport::columns_merged) and
    /// [`values_discarded`](FoldReport::values_discarded) count
    /// **writes**, not distinct columns. Three rows that each
    /// contributed a label add three, though `labels` was one column
    /// throughout. Reporting distinct columns instead would need the
    /// merge to remember which columns earlier folds touched, and the
    /// number a caller wants from a fold report is how much moved.
    pub totals: FoldReport,
    /// The rows that were refused and why, in the order they were
    /// reached. Non-empty means **nothing was written** (the port doc's
    /// all-or-nothing rule), so this is the list to put in front of the
    /// person whose ruling did not run.
    pub refusals: Vec<(AssetId, FoldRefusal)>,
    /// Whether the transaction was kept.
    ///
    /// `false` on a dry run and on a refused merge — the two are the
    /// same event at the storage layer, and a caller that only looked at
    /// [`folded`](Self::folded) would read a prediction as a result.
    pub committed: bool,
}

impl MergeOutcome {
    /// Empty outcome — no rows folded, no refusals, no counts, not
    /// committed. The initial value for the accumulating loop inside
    /// [`AssetRepository::merge_into`] and the same shape a dry run of
    /// an empty plan would report if the plan existed (which
    /// [`MergePlan::declare`](crate::domain::merge_plan::MergePlan::declare)
    /// refuses to build).
    pub fn empty() -> Self {
        Self {
            folded: Vec::new(),
            already_folded: Vec::new(),
            totals: FoldReport::default(),
            refusals: Vec::new(),
            committed: false,
        }
    }
}

impl FoldRefusal {
    /// Slug for the job's log line.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AlreadyFolded => "already folded",
            Self::Missing => "no such asset",
            Self::KeeperMissing => "no such keeper",
            Self::KeeperFolded => "the keeper is itself folded",
            Self::KeeperTrashed => "the keeper is in the trash",
            Self::SameAsset => "an asset cannot be folded into itself",
        }
    }
}

/// Which rows count as holding a Source value, for
/// [`AssetRepository::find_by_source`].
///
/// A parameter rather than a fixed rule because the two callers ask
/// different questions — one about the library, one about storage — and
/// neither answer is the other's default. The trash and the fold axis
/// both fall out of that difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLookupScope {
    /// Rows a person could open — what the ingest path asks. The gate on
    /// minting is "is this record here", so both axes are read as that
    /// question.
    ///
    /// **Trash.** A record in the trash is not here: the trash is
    /// invisible from the active side and in the way of nothing, as a
    /// trash is everywhere else. Re-importing a file whose old row was
    /// thrown away mints a new row, which is what the person importing
    /// it asked for. If they later restore the old one, two rows carry
    /// one Source value — legal under `N : 1`, and the match axes
    /// propose the merge if the bytes agree.
    ///
    /// **Fold.** A headstone *is* here, but as a redirection rather than
    /// as a record: it is in no listing and in no duplicate group. So
    /// this scope does not answer with one — it follows `folded_into`
    /// and answers with the keeper. A fold is a person ruling that two
    /// rows are one thing, and after that ruling the path names the row
    /// that survived it; handing back the headstone would hand the
    /// caller an id nothing can show them, while filtering it out would
    /// call the path unregistered and mint the duplicate again.
    ///
    /// Chains are followed to the end (a keeper folded later leaves the
    /// headstones that pointed at it two hops out), bounded, and kept
    /// inside the persona — a fold written across two libraries by hand
    /// is not followed, because answering one library with another's row
    /// would hand it a locator, a title and labels it may not see. A
    /// chain that cycles, leaves the persona, or runs past the bound is
    /// reported through the adapter's log rather than passed off as an
    /// unregistered path.
    ///
    /// **Where a chain ends decides one candidate, not the lookup.** A
    /// chain ending in the trash, in nothing, or out of bounds means
    /// *that* row holds no live locator; the question then moves to the
    /// next row holding the same value, oldest first, and the answer is
    /// the first one that resolves. `None` — no row a person could open
    /// holds this locator, so minting is right — is what comes back when
    /// every one of them dead-ends.
    ///
    /// Anything less than that mints without end. A fold writes no
    /// `trashed_at`, so a headstone stays live and stays the oldest row
    /// carrying its locator: a lookup that stopped at the first dead end
    /// would never see the row it caused to be minted, and every
    /// re-scan of that folder would mint another one.
    Live,
    /// Every row — trash and headstones included, exactly as stored.
    /// "Who is holding this locator?" asked as a question about storage
    /// rather than about the library. For diagnostics and for the panels
    /// that show what a path resolved to, not for deciding whether to
    /// mint.
    ///
    /// Deliberately **not** resolved through the fold: the row that
    /// holds the locator is the headstone, and resolving would answer a
    /// question this caller did not ask (the keeper carries its own
    /// locator, not this one).
    Any,
}

/// Persistence port for [`Asset`], including the read projection.
#[async_trait]
pub trait AssetRepository: Send + Sync {
    /// Fetches the full entity by id (used on the write and detail paths).
    async fn find(&self, id: &AssetId) -> Result<Option<Asset>, DomainError>;

    /// Ids of assets whose `derived_from` claim is recorded but not
    /// resolved (`extra._trace.resolved == false`).
    ///
    /// The re-resolve sweep's worklist: a claim against a dispatch
    /// that had produced nothing yet becomes answerable once the
    /// export lands, and this is how those rows are found again.
    /// Bounded by `limit` — the sweep is retried on every reify, so a
    /// page left behind is picked up by the next one.
    async fn unresolved_provenance_ids(&self, limit: u32) -> Result<Vec<AssetId>, DomainError>;

    /// Upserts.
    ///
    /// Writes every column on insert, but leaves `palette` alone when
    /// the row already exists — that column belongs to
    /// [`set_palette`](Self::set_palette), which keeps it in step with
    /// the colour facet's index.
    async fn save(&self, asset: &Asset) -> Result<(), DomainError>;

    /// Stamps the asset as trashed at `at` (reversible). Idempotent —
    /// re-trashing an already-trashed row leaves the original stamp so
    /// the retention clock does not restart. A missing id is a no-op.
    ///
    /// The row stays in the table on purpose: every `ON DELETE CASCADE`
    /// child (tags, edges, group filing + order, comments, body,
    /// thumbnails, snapshot membership) survives, which is what makes
    /// [`restore`](Self::restore) a single stamp clear.
    async fn trash(&self, id: &AssetId, at: DateTime<Utc>) -> Result<(), DomainError>;

    /// Clears the trash stamp, returning the asset to the live set.
    /// Idempotent; a missing id is a no-op.
    async fn restore(&self, id: &AssetId) -> Result<(), DomainError>;

    /// Physically deletes an **already-trashed** row and lets the FK
    /// cascade take its children. Returns `Conflict` when the row is
    /// still live — purge is reachable only through the trash, so a
    /// runaway bulk caller always leaves a recoverable intermediate
    /// state. A missing id is a no-op (idempotent).
    async fn purge(&self, id: &AssetId) -> Result<(), DomainError>;

    /// Lists trashed assets whose stamp is older than `cutoff`, oldest
    /// first, capped at `limit`. Drives the retention sweep; the cutoff
    /// is computed by the caller from an injected retention period, so
    /// no policy constant lives in the adapter.
    async fn scan_purgeable(
        &self,
        cutoff: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<AssetId>, DomainError>;

    /// Lists trashed assets regardless of how long they have been
    /// there, oldest first, capped at `limit`. Drives "empty the
    /// trash", where the user is the clock.
    ///
    /// Separate from [`scan_purgeable`](Self::scan_purgeable) rather
    /// than a `cutoff = now` call of it: passing the current instant
    /// as a retention cutoff reads as a policy decision at every call
    /// site, and it silently drops rows trashed within the same
    /// millisecond — exactly the row the user just threw away before
    /// hitting Empty Trash.
    async fn list_trashed_ids(&self, limit: u32) -> Result<Vec<AssetId>, DomainError>;

    /// Fetches an asset of `persona_id` carrying the Source value
    /// `(source_kind, locator)`.
    ///
    /// **This is the ingest path's first question** — asked before an
    /// `AssetId` is minted, so that a record arriving again is handed
    /// the row that was already there instead of producing a second one.
    /// It answers with a candidate, never with a
    /// refusal: `Asset : Source` is `N : 1`, so several rows may carry
    /// one value and the caller decides what that means.
    ///
    /// Because several rows may match, the answer is the **earliest**
    /// one — the row that was already there, which is the only reading
    /// that makes a re-arrival idempotent. Under
    /// [`Live`](SourceLookupScope::Live) that is the earliest one the
    /// scope can answer with: a row whose fold leads nowhere holds no
    /// live locator, so the next row along is asked the same question.
    ///
    /// Scoped to the persona because "have I seen this before" is a
    /// question one library asks about its own sources. Two personas
    /// holding one path are two first imports, not a duplicate. The
    /// scoping holds through the fold as well — a chain leaving the
    /// persona is not followed — so no arrangement of `folded_into` can
    /// answer one library with another library's row.
    ///
    /// `scope` decides what counts as holding the value on both the
    /// trash and the fold axis; it is a real choice per caller rather
    /// than a default, and [`SourceLookupScope`] says what each side is
    /// for. Folded rows are **not** filtered out in either scope — the
    /// fold leaves the locator with the headstone rather than copying it
    /// onto the keeper, so a re-arrival at that path has to reach the
    /// headstone, and that is what keeps an answered duplicate answered.
    /// What differs is the answer: [`Live`](SourceLookupScope::Live)
    /// follows `folded_into` and returns the keeper (a headstone is in
    /// no listing, so an ingest handed one would hand its caller an
    /// invisible id), while [`Any`](SourceLookupScope::Any) returns the
    /// row as stored.
    ///
    /// Takes the value, not a spelling: the implementation renders
    /// [`SourceLocator::to_storage`] for the query, so two callers
    /// holding the same locator ask the same question whatever string
    /// each of them started from.
    async fn find_by_source(
        &self,
        persona_id: &PersonaId,
        source_kind: &SourceKind,
        locator: &SourceLocator,
        scope: SourceLookupScope,
    ) -> Result<Option<Asset>, DomainError>;

    /// Returns every asset id under `persona_id`, trashed or not.
    ///
    /// Exists so the persona purge can drop the right search documents
    /// *before* the FK cascade removes the rows — after that there is no
    /// way to learn which documents to drop. Deliberately id-only and
    /// unpaginated: the card / index projections clamp at their page
    /// ceiling (silently losing the tail on a large persona) and would
    /// materialise labels, group ids and a count nobody reads.
    async fn ids_by_persona(&self, persona_id: &PersonaId) -> Result<Vec<AssetId>, DomainError>;

    /// Trashes every **live** asset of one persona with a shared stamp,
    /// returning the ids it actually stamped (so the caller can drop
    /// their search documents).
    ///
    /// The shared stamp is what makes the paired
    /// [`restore_by_persona`](Self::restore_by_persona) exact: assets the
    /// user had already trashed individually carry a different stamp and
    /// are left alone by both calls, so restoring a persona does not
    /// resurrect things the user threw away on purpose.
    async fn trash_by_persona(
        &self,
        persona_id: &PersonaId,
        at: DateTime<Utc>,
    ) -> Result<Vec<AssetId>, DomainError>;

    /// Restores exactly the assets that carry `stamp` under `persona_id`
    /// — the ones a [`trash_by_persona`](Self::trash_by_persona) call
    /// took down. Returns the restored ids. Assets trashed separately
    /// (different stamp) stay trashed.
    async fn restore_by_persona(
        &self,
        persona_id: &PersonaId,
        stamp: DateTime<Utc>,
    ) -> Result<Vec<AssetId>, DomainError>;

    /// Hot-path listing. Adapters may build the [`AssetCard`] projection
    /// directly from a row scan (no need to materialise the full entity).
    /// The adapter must enforce the visibility filter implied by
    /// `query.viewer`.
    async fn list(&self, query: &AssetQuery) -> Result<Page<AssetCard>, DomainError>;

    /// Index-only listing for large grids (10⁵+ rows). Returns
    /// [`AssetIndex`] projections — same filter / sort surface as
    /// [`list`], but the per-row payload drops cover text /
    /// source locator / file size to keep the IPC transport small
    /// enough for eager full-set fetches. Callers hydrate the
    /// viewport slice with [`cards_by_ids`](Self::cards_by_ids).
    /// Visibility filter is applied exactly as in [`list`].
    async fn list_index(
        &self,
        query: &AssetQuery,
    ) -> Result<Page<crate::domain::asset::AssetIndex>, DomainError>;

    /// Narrows a candidate id set down to the ids that satisfy `query`'s
    /// filter surface — the SQL half of the search pipeline.
    ///
    /// Retrieval ([`AssetRetriever`], Tantivy today) cannot be expressed
    /// as a `WHERE` predicate, so a filtered retrieval runs the two halves
    /// in order — shortlist first, then this filter over it
    /// (the same shape the Query
    /// Group evaluator uses). This method is the SQL half: hand it the
    /// candidate ids and it returns the subset that also passes
    /// modality / tag / group / occurred-range / label / container
    /// filters plus the visibility rule implied by `query.viewer`.
    ///
    /// `query.limit` / `query.offset` are **ignored** — paging happens on
    /// the caller's side after the surviving ids are put back into
    /// relevance order. Return order is unspecified.
    async fn filter_ids(
        &self,
        ids: &[AssetId],
        query: &AssetQuery,
    ) -> Result<Vec<AssetId>, DomainError>;

    /// Draws up to `k` ids at random from the set `query`'s filter
    /// describes — the read behind the sidebar's "🎲 Random".
    ///
    /// Same filter surface as [`list`](Self::list) (the shared
    /// `QueryParts` predicate, visibility rule included); only the order
    /// differs, and here there is none to speak of. **The result is not
    /// reproducible**: two calls with the same argument may share no ids
    /// at all, and the sequence is meaningless. That is the point of the
    /// verb, not a weakness of the implementation — a caller that needs a
    /// stable answer wants `list`.
    ///
    /// `query.limit` / `query.offset` / `query.sort` are **ignored**: a
    /// sample has no pages and no axis. Fewer than `k` ids come back only
    /// when the whole set is smaller.
    async fn sample(&self, query: &AssetQuery, k: u32) -> Result<Vec<AssetId>, DomainError>;

    /// Fetches cards by id (used to resolve constellation-burst targets).
    /// Rows the viewer cannot see are dropped from the result set.
    async fn cards_by_ids(
        &self,
        ids: &[AssetId],
        viewer: &crate::domain::value::Viewer,
    ) -> Result<Vec<AssetCard>, DomainError>;

    /// Maps the headstones among `ids` to the rows they were folded
    /// into — the resolving half of "paths that name a row keep it"
    /// ([`Asset::folded_into`](crate::domain::asset::Asset::folded_into)).
    ///
    /// A caller holding an id set it wrote down earlier — a Snapshot's
    /// frozen membership, an export's inputs — reaches a headstone
    /// legitimately, and the answer it wants is not "drop that one".
    /// Dropping loses a member, and the set's identity with it. The
    /// fold said those two rows are one thing, so the row the id names
    /// *is* the keeper, and this is where that is said. Redirecting on
    /// the way in is what [`crate::application::fold_redirect`] does with
    /// this answer.
    ///
    /// **Entries only for ids that resolve.** A live row, an id nothing
    /// holds, and a chain that dead-ends (a cycle, a keeper in another
    /// persona, a chain past the walk's ceiling, a chain ending in the
    /// trash) are all absent from the map, so a caller that finds no
    /// entry keeps the id it had. That is the honest answer for a dead
    /// end: the row in hand is the only one this call can name.
    ///
    /// Follows the whole chain rather than one link — a row folded into
    /// a row that was itself folded resolves to the far end, the same
    /// walk [`find_by_source`](Self::find_by_source) makes for a
    /// locator.
    ///
    /// Deliberately **not** built into
    /// [`cards_by_ids`](Self::cards_by_ids). That call hydrates whatever
    /// it is handed for a reason its own doc records (the trash view's
    /// hydration depends on it, and the grid hands it ids a filtered
    /// read already vetted). This one answers a question *about* the
    /// ids, and the caller that holds an id set of its own decides what
    /// to do with the answer.
    async fn resolve_folds(
        &self,
        ids: &[AssetId],
    ) -> Result<std::collections::HashMap<AssetId, AssetId>, DomainError>;

    /// Index-projection twin of [`cards_by_ids`](Self::cards_by_ids).
    ///
    /// Exists for the sorted index read: naming an axis means the order
    /// is decided over the whole filtered set before the page is cut, so
    /// the rows have to be fetched by the ids that survived rather than
    /// by a `LIMIT`. Same set semantics as its sibling — free to
    /// reorder, and rows the viewer cannot see drop out, so the caller
    /// re-projects onto the order it computed.
    async fn index_by_ids(
        &self,
        ids: &[AssetId],
        viewer: &crate::domain::value::Viewer,
    ) -> Result<Vec<crate::domain::asset::AssetIndex>, DomainError>;

    /// Read-side listing of every [`Session`] in scope. Sourced from
    /// the `session` table (Dialog-modality 1st-class entity);
    /// `message_count` /
    /// `started_at_ms` / `ended_at_ms` are derived per-session from
    /// the `asset` aggregate join so a stale snapshot column can
    /// never mislead a caller.
    ///
    /// Only `query.persona_id` / `query.offset` / `query.limit` are
    /// honoured — Session is Dialog-only and metadata-driven, so the
    /// old asset-level filters (modality / tag_ids / group_ids /
    /// label / occurred_at) do not apply. Ordering is by
    /// `started_at_ms` descending (freshest run first, matching the
    /// SessionsView tile order).
    async fn list_sessions(&self, query: &AssetQuery) -> Result<Page<Session>, DomainError>;

    /// Narrow write — persists the dominant-colour palette for one
    /// asset without round-tripping the whole entity. Called from the
    /// `thumb_gen` handler once `color-thief` has finished. `None`
    /// clears the column.
    ///
    /// The palette's quantised form (the colour facet's index) is
    /// rewritten in the same transaction, so an implementation can
    /// never leave a palette and its buckets disagreeing — a stale
    /// bucket would put an asset under a swatch its palette no longer
    /// justifies.
    ///
    /// This verb **owns** the palette: [`save`](Self::save) does not
    /// update the column on an existing row. Otherwise a
    /// read-modify-write elsewhere (find → edit metadata → save) would
    /// carry a pre-extraction palette back over an extraction that
    /// landed in between, and the buckets — which `save` knows nothing
    /// about — would outlive the palette that produced them.
    async fn set_palette(
        &self,
        asset_id: &AssetId,
        palette: Option<Vec<String>>,
    ) -> Result<(), DomainError>;

    /// Sidebar count aggregation — one `(persona_id, count)` row
    /// per persona that owns at least one asset. Ordered by count
    /// descending then persona uuid ascending (stable). No viewer
    /// filter is applied; the count reflects the underlying `asset`
    /// table verbatim (`Restricted` assets included).
    ///
    /// `trash` selects which side is counted, and must match whatever
    /// the grid is showing: a live count beside a trash grid describes
    /// the other half of the app, and clicking the chip then filters the
    /// trash by a number that was never about it.
    async fn counts_by_persona(
        &self,
        trash: crate::domain::asset::TrashFilter,
    ) -> Result<Vec<(PersonaId, u64)>, DomainError>;

    /// Sidebar count aggregation — one `(modality, count)` row per
    /// modality slug present in the corpus, optionally scoped to
    /// one persona. Ordered by count descending then modality slug
    /// ascending. Same "no viewer filter" and `trash` notes as
    /// [`counts_by_persona`](Self::counts_by_persona).
    async fn counts_by_modality(
        &self,
        persona_id: Option<&PersonaId>,
        trash: crate::domain::asset::TrashFilter,
    ) -> Result<Vec<(String, u64)>, DomainError>;

    /// Sidebar FORMAT facet aggregation (asset-model v4) — one
    /// `(format, count)` row per mime top-level type (`image` /
    /// `video` / `audio` / `text`, …) present on **top-level** assets'
    /// primary materials, optionally scoped to one persona. Rows with
    /// an unknown mime carry no format and are not counted. Ordered by
    /// count descending then format ascending. Same "no viewer filter"
    /// and `trash` notes as [`counts_by_persona`](Self::counts_by_persona).
    async fn counts_by_format(
        &self,
        persona_id: Option<&PersonaId>,
        trash: crate::domain::asset::TrashFilter,
    ) -> Result<Vec<(String, u64)>, DomainError>;

    /// Sidebar COLOR facet aggregation — one `(bucket, count)` row per
    /// swatch carried by at least one **top-level** asset's palette,
    /// optionally scoped to one persona. An asset counts once per
    /// bucket regardless of how many of its five palette entries fall
    /// into it, and an asset whose palette was never extracted counts
    /// nowhere: the facet reports what is known rather than filling the
    /// gap with a guess.
    ///
    /// Ordered by [`ColorBucket::ALL`](crate::domain::color::ColorBucket::ALL)
    /// — swatch order, not count order, because a colour row that moves
    /// as counts change is hard to aim at. Same "no viewer filter" and
    /// `trash` notes as [`counts_by_persona`](Self::counts_by_persona).
    async fn counts_by_color(
        &self,
        persona_id: Option<&PersonaId>,
        trash: crate::domain::asset::TrashFilter,
    ) -> Result<Vec<(crate::domain::color::ColorBucket, u64)>, DomainError>;

    /// Narrow write — records one material's fingerprints, **both axes
    /// in one statement**, without round-tripping the whole entity.
    /// Called from the `material_hash` job once the file has been read.
    ///
    /// This verb owns the two columns: [`save`](Self::save) never
    /// writes them, so a metadata round-trip cannot erase a fingerprint
    /// computed in between. A material that has since disappeared is
    /// not an error — the write simply matches no row.
    ///
    /// # One statement, not two
    ///
    /// Writing the axes with two updates would leave a window in which
    /// the row holds one and not the other, and something reads rows
    /// during that window: the walk's own predicate
    /// ([`needs_fingerprint`](crate::domain::content_hash::needs_fingerprint))
    /// would see a half-filled row as work, the duplicate report would
    /// group on an axis for rows whose other axis had not landed, and a
    /// process killed between the two would leave the half permanently
    /// — the second write is not retried by anything. The implementation
    /// is one `UPDATE` with two `SET` clauses, and the argument being a
    /// single value is the shape that keeps it from drifting back into
    /// two calls.
    async fn set_material_fingerprint(
        &self,
        asset_id: &AssetId,
        ord: u32,
        fingerprint: &MaterialFingerprint,
    ) -> Result<(), DomainError>;

    /// Narrow write — records that one material's bytes could not be
    /// read: every axis still `pending` (or already `failed`, which
    /// refreshes the error) flips to
    /// [`Failed`](crate::domain::measurement::MeasurementStatus::Failed) with
    /// `reason` (the I/O error) beside it, and every other axis is
    /// left exactly as it was.
    ///
    /// The conditional per-axis write matters: a partially answered row
    /// (a build that predates the newest column) keeps the digests it
    /// has, and only the axes the failed read was going to fill record
    /// the failure. Writing a whole
    /// [`MaterialFingerprint`] here would overwrite measurements with a
    /// statement about a read that never happened.
    ///
    /// **Deliberately not "every axis the walk calls unanswered".** A
    /// `computed` axis holding a superseded-generation digest is
    /// unanswered for the walk and still does not flip: no version bump
    /// has shipped, so the state cannot exist today, and the bump that
    /// creates it owes a managed migration moment of its own
    /// ([`needs_content_walk`](crate::domain::content_hash::needs_content_walk)
    /// records why). KNOWN LIMITATION carried with that bump: a
    /// stale-`computed` axis on a permanently unreadable original would
    /// sit in the progress count rather than the unreadable one, so
    /// whoever bumps a generation revisits this CASE alongside the
    /// re-walk it already owes.
    ///
    /// What this buys is the split issue #17 asked for: a row whose
    /// original is gone stops sitting in the "still fingerprinting"
    /// count forever ([`unhashed_material_count`](Self::unhashed_material_count)
    /// excludes `failed`) and surfaces in
    /// [`unreadable_material_count`](Self::unreadable_material_count)
    /// instead — while the walk keeps retrying it, because `failed` is
    /// deliberately not a final answer and the disk may come back.
    async fn mark_material_unreadable(
        &self,
        asset_id: &AssetId,
        ord: u32,
        reason: &str,
    ) -> Result<(), DomainError>;

    /// Narrow write — records one field under
    /// `extra.`[`_trace`](crate::domain::provenance::TRACE_KEY),
    /// touching no other column.
    ///
    /// Narrow for the same reason its neighbour above is, and then some.
    /// The callers are background jobs: they run on a worker while
    /// somebody may be editing the same asset in the grid, so a
    /// read-modify-save of the whole entity would write back every
    /// column as it stood when the job started — a rating or a tag set
    /// applied in between would vanish, and the loss would be silent
    /// because the job succeeded.
    ///
    /// `field` names the key inside the trace, and each one belongs to
    /// the module that owns the concept rather than being spelled at
    /// the call site:
    /// [`declared_hash`](crate::domain::content_hash::DECLARED_HASH_NOTE_KEY)
    /// for what became of a caller's declared digest, and
    /// [`disclosure`](crate::domain::disclosure::DISCLOSURE_NOTE_KEY)
    /// for what became of an artefact's AI disclosure. The trace is a
    /// shared bag with several independent writers, so a field is
    /// replaced and its neighbours are left alone.
    ///
    /// Returns whether the note landed. `false` means the `extra`
    /// column holds something this cannot merge into without destroying
    /// it (unparseable JSON, or a `_trace` that is not an object); the
    /// column is then left exactly as it was, and the caller says so
    /// out loud rather than overwriting somebody's bag to record a
    /// bookkeeping note. A row that has since been deleted is not an
    /// error either — the write matches no row and reports `false`.
    async fn note_trace_field(
        &self,
        asset_id: &AssetId,
        field: &str,
        note: serde_json::Value,
    ) -> Result<bool, DomainError>;

    /// Materials whose bytes have not been fingerprinted yet, oldest
    /// asset first, at most `limit` of them — the backfill job's page.
    ///
    /// "Not fingerprinted yet" is
    /// [`needs_fingerprint`](crate::domain::content_hash::needs_fingerprint),
    /// which this and
    /// [`unhashed_material_count`](Self::unhashed_material_count) have
    /// to evaluate identically — see that method's note on what a
    /// disagreement does to the product.
    ///
    /// Ordering by `(asset_id, ord)` (UUID v7 = time-ordered, then
    /// position) makes the walk resumable from a cursor rather than
    /// re-scanning from the start, and means the oldest imports are
    /// answered first. The cursor is the same composite key — an
    /// `asset_id`-only cursor would skip the remaining `ord > 0`
    /// materials of an asset a page boundary happened to cut through.
    async fn scan_unhashed_materials(
        &self,
        after: Option<(&AssetId, u32)>,
        limit: u32,
    ) -> Result<Vec<UnhashedMaterial>, DomainError>;

    /// Materials whose embedded text nobody has looked for yet
    /// (`meta_text IS NULL`), oldest asset first, at most `limit` of
    /// them — the recovery walk's page.
    ///
    /// Same row shape and same composite cursor as
    /// [`scan_unhashed_materials`](Self::scan_unhashed_materials),
    /// because it is the same table walked for the same kind of reason;
    /// what differs is the question. That one asks "does this row owe a
    /// digest", which is answered by a versioned vocabulary and can
    /// therefore be re-asked when the vocabulary moves. This one asks
    /// "has anything looked for words in these bytes", which the column
    /// answers by existing at all — so the predicate is `IS NULL` and a
    /// row leaves the set whatever the walk found in it, `{}` included.
    ///
    /// The format is **not** filtered here. A caller that reads bytes
    /// decides what it can read
    /// ([`embedded_text::walks_format`](crate::domain::embedded_text::walks_format)),
    /// and pushing that list into SQL would put a second copy of it in a
    /// string, to be re-typed the day the recovery learns a container.
    async fn scan_unrecovered_text(
        &self,
        after: Option<(&AssetId, u32)>,
        limit: u32,
    ) -> Result<Vec<UnhashedMaterial>, DomainError>;

    /// Writes one material's recovered text and nothing else.
    ///
    /// Narrow on purpose. The three digest columns beside it are one
    /// measurement written by one statement
    /// ([`set_material_fingerprint`](Self::set_material_fingerprint)),
    /// and the recovery walk has no business restating any of them: it
    /// did not compute them, the row already carries them, and writing
    /// them back would make a text pass into a re-fingerprint of the
    /// library under another name.
    ///
    /// `None` writes SQL `NULL` — "nobody has looked" — which is what a
    /// walk records when it could not read the bytes at all, so the row
    /// stays in the set for a later pass.
    async fn set_material_embedded_text(
        &self,
        asset_id: &AssetId,
        ord: u32,
        meta_text: Option<&str>,
    ) -> Result<(), DomainError>;

    /// Materials that **already carry** their fingerprints, oldest asset
    /// first, at most `limit` of them — the inverse of
    /// [`scan_unhashed_materials`](Self::scan_unhashed_materials), and
    /// the page the duplicate re-scan walks.
    ///
    /// # Why a second walk over rows the first one is done with
    ///
    /// A conflict is *derived* from a fingerprint, and the derivation
    /// runs once — at the moment the digest is written
    /// (`detect_after_hash`). Its own doc names the consequence: "once
    /// both are written nothing comes back to redo the lookup for this
    /// row", and the consolation it offers ("raised again the next time
    /// either side of the pair is fingerprinted") never arrives, because
    /// the hashing walk selects on the fingerprint being *absent*. A
    /// pair whose moment passed — the feature landed after the rows did,
    /// the lookup errored and was swallowed, the second side arrived
    /// while the first was mid-write — is invisible for good.
    ///
    /// Measured, not supposed: a Dogfood profile with 289 fingerprinted
    /// materials and two byte-identical groups (five rows, one persona,
    /// no `Separate` on any of them) carried **zero** conflict rows.
    ///
    /// This walk carries no bookkeeping column, and does not need one:
    /// `duplicate_conflict` is `UNIQUE (pair_lo, pair_hi, axis)` and the
    /// insert is `ON CONFLICT DO NOTHING`, so re-deriving is free of
    /// effect. A pair a person already answered keeps its `resolution`
    /// and is not asked again.
    ///
    /// Rows whose axes hold *markers* rather than digests
    /// (`unhashable:no-bytes` and the unsupported-region family) are in
    /// the page: the marker is what "answered" means for the walk, and
    /// which markers are excluded from *matching* is the detection's
    /// rule to apply, not this scan's to pre-empt.
    async fn scan_fingerprinted_materials(
        &self,
        after: Option<(&AssetId, u32)>,
        limit: u32,
    ) -> Result<Vec<FingerprintedMaterial>, DomainError>;

    /// Assets a measuring pass should visit under `scope`, oldest first,
    /// at most `limit` of them.
    ///
    /// Ordered by `id` — UUID v7, so time-ordered — which makes a pass
    /// resumable from a cursor and answers the oldest imports first.
    /// One row per asset rather than per material, because the columns
    /// being filled are the asset's.
    ///
    /// The scope is a parameter rather than a fixed predicate because
    /// three different questions are asked of this table and only one of
    /// them is the startup walk's; see [`DimsScope`].
    async fn scan_dims_candidates(
        &self,
        scope: DimsScope,
        after: Option<&AssetId>,
        limit: u32,
    ) -> Result<Vec<DimsCandidate>, DomainError>;

    /// Materials that can declare chapters and have no imported
    /// structure layer yet, oldest asset first, at most `limit` of them
    /// — the `ChapterScan` backfill's page.
    ///
    /// Ordered and cursored by the composite `(asset_id, ord)` key, the
    /// same shape and for the same reason as
    /// [`scan_unhashed_materials`](Self::scan_unhashed_materials): an
    /// `asset_id`-only cursor would skip the remaining materials of an
    /// asset a page boundary cut through.
    ///
    /// # Two predicates, and the second is why this is on this port
    ///
    /// "Can declare chapters" is the material's `mime`; "has no imported
    /// structure layer" is the absence of a `material_layer` row. The
    /// second is what makes the walk terminate — a completed reading
    /// always leaves that row, so a material is offered once — and it is
    /// stated here, in SQL, rather than by filtering a full walk in the
    /// handler: the handler's version of the filter would still have
    /// paid for a full table walk on every start, and a walk that
    /// re-offers the whole library is a walk that re-opens it.
    ///
    /// Kept beside its two sibling walks rather than on the layer port
    /// because the row it yields is a material's, and because a caller
    /// looking for "how does a backfill find its work" should find all
    /// three answers in one place.
    async fn scan_chapter_scan_candidates(
        &self,
        after: Option<(&AssetId, u32)>,
        limit: u32,
    ) -> Result<Vec<ChapterScanCandidate>, DomainError>;

    /// Records the outcome of one dimension probe.
    ///
    /// **The pair and the stamp are one write.** Setting the pair
    /// without the stamp leaves the row in the startup walk; setting the
    /// stamp without the pair throws away the measurement. They are the
    /// same fact — "this row was probed, and here is what came back" —
    /// so they are not separable calls.
    ///
    /// What each outcome does:
    ///
    /// | outcome | columns | stamp |
    /// |---|---|---|
    /// | [`Measured`](DimsProbe::Measured) | written, subject to `policy` | set |
    /// | [`NothingToMeasure`](DimsProbe::NothingToMeasure) | left alone | set |
    /// | [`Unreadable`](DimsProbe::Unreadable) | **nothing is written at all** | |
    ///
    /// The last row is the one worth reading twice: a failed read leaves
    /// no trace, so the asset stays in every scope it was in. That is
    /// deliberate — see [`DimsProbe`] for why recording it would be a
    /// permanent answer to a temporary question.
    async fn record_dims_probe(
        &self,
        asset_id: &AssetId,
        outcome: DimsProbe,
        policy: DimsWritePolicy,
        probed_at: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// Assets under `persona_id` whose **primary** material (`ord = 0`)
    /// already carries `digest` **on `axis`** — the lookup duplicate
    /// detection runs after a fingerprint lands.
    ///
    /// # The axis is a parameter, not a column named here
    ///
    /// Each axis has its own column (`content_hash` for `Artefact`,
    /// `content_region_hash` for `Content`) and its own vocabulary of
    /// digests and markers, and the adapter maps one to the other in
    /// the same place the duplicate report already does. Taking the
    /// axis rather than a bare digest is what stops the two being
    /// crossed: a `cr1-sha256:` value read against the artefact column
    /// would answer "nobody holds these bytes" about a question nobody
    /// asked.
    ///
    /// The by-key sibling of
    /// [`find_by_source`](Self::find_by_source): that one asks "who
    /// holds this path", this one asks "who holds these bytes". Ordered
    /// oldest first (`occurred_at`, then id), the same order
    /// [`list_duplicate_groups`](Self::list_duplicate_groups) puts its
    /// members in, so the caller reads the incumbent off the front
    /// instead of re-deriving an order. Full entities rather than ids:
    /// every column the decision needs — `fold_policy`, `trashed_at`,
    /// `folded_into`, `occurred_at`, `source` — is on the row already,
    /// and a duplicate set is a handful of rows.
    ///
    /// `ord = 0` is the key half nothing else can supply. A material at
    /// `ord > 0` is a *secondary* original of an asset (the RAW beside
    /// the JPEG, once that wave lands); it holding the same bytes as
    /// somebody's primary is not two of the same asset, and folding on
    /// it would discard the row that owns the other resource.
    ///
    /// # What must be passed
    ///
    /// A real digest. Values that are stored in the column but do not
    /// stand for bytes — the `unhashable:` marker every fragment and
    /// remote locator shares, the empty-file digest — are refused with
    /// [`DomainError::Validation`](crate::error::DomainError::Validation)
    /// rather than answered, and the rule is
    /// [`is_duplicate_key`](crate::domain::content_hash::is_duplicate_key),
    /// not a second list here.
    ///
    /// Refusing beats returning an empty set, which is the tempting
    /// cheap option: empty is the answer that also means "nobody else
    /// holds these bytes", so a caller that forgot to pre-filter would
    /// be told its conversation record is unique and would carry on
    /// forever without a signal. It also beats answering honestly —
    /// every row sharing the marker *is* a match for it, and that set
    /// is the entire conversation corpus reported as one duplicate.
    /// The caller's filter is the same predicate this refusal uses.
    ///
    /// # What comes back
    ///
    /// **Headstones do not** (`folded_into IS NULL`). This enumerates,
    /// and the fold rule for enumerating reads is that a folded row is
    /// gone (`Asset::folded_into`). It is also the answer that keeps
    /// the third copy working: bytes arriving again find the keeper —
    /// which still carries the hash — and raise a conflict against a
    /// row that exists, instead of against a dead one they could then
    /// be folded into.
    ///
    /// **Trashed rows do** — deliberately, and unlike the duplicate
    /// report. That report asks the user to act, and a row already on
    /// its way out is not worth acting on; this is a lookup, and a
    /// trashed row holds the bytes just as firmly as a live one holds
    /// its locator (the reasoning `find_by_source` already applies to
    /// the path axis). Dropping it would make a re-import of something
    /// the user threw away look unique, which is the one case where
    /// silence is worse than a question. `trashed_at` is on the
    /// returned entity, so "it is here, in the trash" stays
    /// distinguishable from "it is here" without a second read.
    ///
    /// Cross-persona matches are **not** returned: the key is
    /// `(persona_id, digest)` and folding across personas is a manual
    /// act only.
    ///
    /// The result is **unbounded**, and each row is hydrated with its
    /// materials. That is honest for the sets this is written for — a
    /// re-import of one file finds one or two holders — but a library
    /// that regenerated the same bytes a thousand times would get a
    /// thousand entities on every fingerprint. No cap is imposed here
    /// because the right cap depends on what the caller does with the
    /// tail: detection needs only the incumbent at the front, while a
    /// resolution surface wants the whole set. The consumer that first
    /// needs one should add it as a parameter rather than have this
    /// method guess.
    ///
    /// A match is a fact about bytes, never a verdict — see
    /// [`EdgeKind::IdenticalTo`](crate::domain::edge::EdgeKind::IdenticalTo).
    /// Deciding what to do with the returned set (raise, fold,
    /// separate) belongs to the caller.
    async fn find_by_content_hash(
        &self,
        persona_id: &PersonaId,
        axis: DuplicateAxis,
        digest: &str,
    ) -> Result<Vec<Asset>, DomainError>;

    /// Folds `headstone` into `keeper`: marks the losing row
    /// (`folded_into`), moves the structure that hung off it, and
    /// combines the columns the two rows both carry — all in **one
    /// transaction**.
    ///
    /// # What moves
    ///
    /// - `edge` — re-pointed to the keeper. The pair's own edges would
    ///   become a keeper→keeper self-loop and are removed; so are those
    ///   whose `(from, to, kind)` the keeper already carries, since the
    ///   table admits one row per triple. Both losses are recorded on
    ///   the headstone's `_trace` before they go (a dropped edge
    ///   is a claim somebody made, and it must not vanish silently).
    /// - `asset_bucket` — the keeper joins the Groups the headstone was
    ///   filed in. Where it was already a member its own position
    ///   stands, and the position it displaced goes to
    ///   `_trace.absorbed` — that row is deleted, so unlike a losing
    ///   column the value would otherwise be gone (hand arrangement
    ///   falls under the same rule as `container_id`).
    /// - `asset.container_id` — rows filed inside the headstone are
    ///   re-filed inside the keeper.
    /// - `asset_tag` — the keeper gains the headstone's tags.
    /// - `asset_comment` — the headstone's comments are re-pointed at
    ///   the keeper. Their own `created_at` is untouched, so the
    ///   keeper's thread reads as a single chronology with the two
    ///   sets interleaved, which is what a comment thread *is*. No
    ///   de-duplication: two identical bodies are two things two people
    ///   said, and collapsing them would delete one of them.
    /// - `thread.anchor_id` — threads anchored on the headstone's card
    ///   are re-anchored on the keeper's. The anchor carries no
    ///   uniqueness, so a keeper that already had a thread simply ends
    ///   up with both.
    ///
    /// # Which columns of the keeper move
    ///
    /// A fold is not "the keeper was right about everything". The two
    /// rows hold the same bytes, and people put different values on
    /// them. Every column falls in exactly one of three groups
    /// (the first six were fixed when the fold was designed; the rest
    /// were settled here on the same axis), and the adapter asserts the
    /// three lists partition the table — a column added later belongs
    /// to none of them and says so instead of drifting into a rule
    /// nobody chose.
    ///
    /// **Combined**, because combining two values needs no verdict on
    /// which claim was right:
    ///
    /// | column | rule |
    /// |---|---|
    /// | `labels`, `keywords`, `vis_sharing` | union, keeper's order first |
    /// | `rating` | max, counting only values `> 0` (`0` is "unrated", not a low score) |
    /// | `register_note` | non-empty halves joined by a blank line; identical text is not doubled |
    /// | `vis_restricted` | `OR` — the more restricted side wins |
    ///
    /// **Kept, and the discarded value recorded.** Every remaining
    /// single-valued column: `source_kind`, `source_locator`,
    /// `file_size_bytes`, `platform`, `modality`, `occurred_at`,
    /// `bundle_id`, `cover`, `duration_ms`, `palette`, `container_id`,
    /// `title`, `role`, `author_kind`, `author_subject`,
    /// `operator_ai`, `on_duplicate`, `fold_policy`. Where the two rows
    /// disagree the keeper's value stands and the headstone's is
    /// written to the keeper's `_trace.absorbed` — including where the
    /// keeper's value is `NULL`. Nothing is transplanted into a hole:
    /// an absent author is *unrecorded*
    /// ([`attribution`](crate::domain::attribution)), and filling it
    /// from the other row would mint an assertion about the keeper that
    /// nobody made; an absent `cover` / `palette` belongs to the job
    /// that owns the column, not to this verb.
    ///
    /// **Untouched, and not compared**: `id`, `persona_id`,
    /// `created_at`, `trashed_at`, `folded_into` (identity and
    /// lifecycle — a fold is not a way to move a row between personas
    /// or out of the trash), `updated_at` (stamped, not merged), and
    /// `extra`. `extra` is the bag the note itself lives in, and
    /// merging two opaque bags can only overwrite an importer's keys;
    /// the headstone's own bag stays verbatim on the headstone row,
    /// which the note names.
    ///
    /// The keeper's `updated_at` is stamped on **every** fold that goes
    /// through, including one that combines nothing: the keeper gained
    /// tags, Groups, comments and edges, and `updated_from_ms` (V46) is
    /// the only way a differential sync hears about any of it. That is
    /// the same reason a re-filed child is stamped.
    ///
    /// # What stays
    ///
    /// - **The locator, on the headstone.** `UNIQUE(source_kind,
    ///   source_locator)` admits one holder and the row that was
    ///   imported from there is the one that holds it — which is what
    ///   makes a re-import of that path, and a `<locator>#<keyword>`
    ///   fragment reference, resolve *through* the headstone.
    /// - `snapshot_asset` — a snapshot is content-addressed by its
    ///   member set, so re-pointing a member would silently change what
    ///   a frozen selection was.
    /// - `material`, `asset_body`, `asset_color`, `thumb_cache` — the
    ///   headstone's own bytes and everything derived from them. The
    ///   keeper has its own.
    /// - `message.refs_json` — a card reference inside a message body
    ///   is prose, not filing. It resolves through the headstone by id
    ///   like every other minted reference.
    ///
    /// # The re-read
    ///
    /// The guard is **in the marking statement's own predicate**, not in
    /// a preceding `SELECT`, for the reason [`purge`](Self::purge)
    /// records: two processes share the database file, so a
    /// check-then-write pair can be overtaken between its halves. Here
    /// that would mean two workers folding a pair into each other from
    /// both ends. With the predicate inlined the second one matches zero
    /// rows and the whole transaction is a no-op — [`FoldOutcome::Skipped`].
    ///
    /// The other statements have no predicate of their own to carry the
    /// guard; they are covered by the transaction instead. They run only
    /// after the marking statement matched, and nothing outside can
    /// observe an intermediate state, so a fold either happened whole or
    /// not at all.
    ///
    /// Refusals are not errors: the decision that enqueued a fold was
    /// taken before the job ran, and the row could have been trashed,
    /// folded elsewhere, or deleted in between. Re-running a fold that
    /// already happened is likewise a refusal
    /// ([`FoldRefusal::AlreadyFolded`]) rather than a second fold — the
    /// verb is safe to replay, which a job engine without retries needs
    /// it to be.
    async fn fold_into(
        &self,
        headstone: &AssetId,
        keeper: &AssetId,
    ) -> Result<FoldOutcome, DomainError>;

    /// Carries out a person's ruling that a set of rows is one thing:
    /// every row [`MergePlan::discard`] names is folded into
    /// [`MergePlan::keeper`], in **one transaction**.
    ///
    /// Each fold is exactly [`fold_into`](Self::fold_into) — same
    /// re-read, same structure moves, same column rules, same
    /// `_trace` notes. This verb adds three things and nothing else: N
    /// rows instead of one, one transaction around all of them, and
    /// `dry_run`. It is deliberately **not** bound by the exclusions
    /// that stop an automatic fold (lineage, dispatch output): a person
    /// looking at the rows can see what those rules were protecting.
    ///
    /// # Why one transaction
    ///
    /// A partly-executed merge — "three rows were ruled one thing and
    /// two of them were folded" — leaves a state the person cannot
    /// inspect: the rows that were folded are gone from every listing,
    /// so what they are left looking at is a set smaller than the one
    /// they ruled over, with no way to tell whether the missing rows
    /// went where they meant them to. `dry_run` also depends on it,
    /// since a prediction is only a prediction of the whole.
    ///
    /// # What a refusal does
    ///
    /// **One refusal abandons the whole merge**, and nothing is
    /// written. A manual merge is a decision about *this set*, and
    /// executing some other set is the worst way for it to fail —
    /// worse than not running, because not running is visible and a
    /// half-merge is not. The caller re-reads and rules again; the
    /// refusals are handed back with the row each one is about, and
    /// `dry_run` is there to find them before committing to anything.
    ///
    /// **One exception**, and it is a narrow one: a row already folded
    /// **into this same keeper** is counted in
    /// [`MergeOutcome::already_folded`] and does not stop the merge.
    /// That is not "a refusal we decided to tolerate" — it is the plan
    /// already being true for that row, and the state after the call is
    /// exactly the state the plan declares either way. The same
    /// reasoning as [`fold_into`](Self::fold_into) being safe to replay:
    /// a double click, a retried request, or a set assembled from a
    /// panel that a fold job has since overtaken all produce it, and
    /// refusing them would make the person re-rule a set that is
    /// already partly settled while giving them nothing they did not
    /// know. A row folded into **anybody else** is a different ruling by
    /// somebody else and stops everything, like every other refusal.
    ///
    /// # Order
    ///
    /// The folds happen in the order [`MergePlan::discard`] lists, and
    /// that order is **the caller's**.
    ///
    /// It cannot be made not to matter: `register_note` is combined by
    /// joining the non-empty halves keeper-first, so A→B→C and C→B→A
    /// give the same paragraphs in a different sequence, and the entries
    /// under the keeper's `_trace.absorbed` come out in fold order too.
    /// Making the result order-independent would mean a second way of
    /// combining columns that does not exist anywhere else — the one
    /// thing this verb is built not to have.
    ///
    /// So the order is a decision, and the only party in a position to
    /// make it is the one that chose the set: a panel showing five rows
    /// shows them in some arrangement, and the notes should read in it.
    /// Sorting here — by id, which for UUID v7 means by arrival — would
    /// replace an order somebody chose with one nobody did, and would
    /// make the panel's arrangement unrepresentable. A caller with no
    /// order to express therefore has to pick one deliberately;
    /// [`MergePlan`] refuses repeats, so what it holds is a genuine
    /// sequence rather than a set that happened to be iterated.
    ///
    /// # `dry_run`
    ///
    /// The whole merge runs and the transaction is then dropped, which
    /// is how [`TagRepository::merge`] previews itself too. It goes one
    /// step further than that one: the counts come from **the statements
    /// that would have been kept**, where the tag merge answers its
    /// preview with a `COUNT` of its own. A prediction computed by a
    /// second route is a second implementation, and there are seven
    /// numbers here rather than two — the moment any of them disagrees,
    /// the preview stops describing the thing it previews, and it is a
    /// preview of an operation with no undo. Refusals are found the same
    /// way, by actually reaching them.
    ///
    /// [`MergePlan::discard`]: crate::domain::merge_plan::MergePlan::discard
    /// [`MergePlan::keeper`]: crate::domain::merge_plan::MergePlan::keeper
    /// [`MergePlan`]: crate::domain::merge_plan::MergePlan
    /// [`TagRepository::merge`]: TagRepository::merge
    async fn merge_into(
        &self,
        plan: &MergePlan,
        dry_run: bool,
    ) -> Result<MergeOutcome, DomainError>;

    /// Groups of **live** assets whose primary material carries the
    /// same content hash — the duplicate report.
    ///
    /// Only groups of two or more are returned; a hash held by one
    /// asset is not a finding. Trashed assets are excluded: they are
    /// already on their way out, and listing them would ask the user
    /// to resolve a duplicate twice.
    ///
    /// `limit` bounds the number of *groups*, newest-first by the
    /// group's most recent member, so a corpus with thousands of
    /// duplicates still answers in one page.
    ///
    /// # One axis per call
    ///
    /// `axis` picks which fingerprint column is grouped on, and the
    /// call answers about that one only. The alternative — group on the
    /// content axis and hand back the file-identical sets as an
    /// internal breakdown — reads as the cheaper shape, because a
    /// content group does contain the file group whenever both rows
    /// carry a content digest. **They do not always carry one.** A
    /// format with no walker holds the
    /// [`Unsupported`](crate::domain::measurement::MeasurementStatus::Unsupported)
    /// status, and a material whose original could not be read when the
    /// column was filled in holds
    /// [`NotWalked`](crate::domain::measurement::MeasurementStatus::NotWalked);
    /// the content axis cannot see either, while the file axis groups
    /// both perfectly well. Deriving one axis from the other would
    /// therefore drop findings this port already reports today —
    /// silently, and only for the rows least likely to be in a test
    /// fixture.
    ///
    /// One axis per call also fixes what a reader is looking at: a
    /// group is the answer to one question, so the same two assets
    /// never appear twice in one report.
    async fn list_duplicate_groups(
        &self,
        persona_id: Option<&PersonaId>,
        axis: DuplicateAxis,
        limit: u32,
    ) -> Result<Vec<DuplicateGroup>, DomainError>;

    /// How many materials are still **open work** for the fingerprint
    /// pass — the "not finished looking" number that stops an empty
    /// duplicate report from reading as "no duplicates".
    ///
    /// **Deliberately not the scan's rule.** The walk
    /// ([`scan_unhashed_materials`](Self::scan_unhashed_materials))
    /// selects by
    /// [`needs_fingerprint`](crate::domain::content_hash::needs_fingerprint),
    /// which includes `failed` rows so the retry happens; this count
    /// uses [`awaits_fingerprint`](crate::domain::content_hash::awaits_fingerprint),
    /// which excludes them so the number can reach zero. The rows in
    /// the difference are exactly
    /// [`unreadable_material_count`](Self::unreadable_material_count)'s,
    /// and the differential tests pin the three-way split. Before issue
    /// #17 the two rules were one, and a library with one permanently
    /// missing original wore a "still fingerprinting" notice forever.
    ///
    /// A material that can never be hashed counts in neither number: it
    /// holds `no-bytes` on every axis, which is *answered*, not
    /// pending — otherwise this could never reach zero on a library of
    /// conversation records.
    async fn unhashed_material_count(&self) -> Result<u64, DomainError>;

    /// How many materials are stuck on an **unreadable original**: the
    /// walk still owes them a pass and every unanswered axis says
    /// `failed` — the original was not where the library says it is
    /// when the job tried to read it.
    ///
    /// The other half of
    /// [`unhashed_material_count`](Self::unhashed_material_count)'s
    /// split (issue #17): these rows are not work in progress — the
    /// number does not move on its own, the files have to come back —
    /// so they are surfaced separately instead of holding the progress
    /// notice open forever. The I/O error sits in each row's reason
    /// column, so the count is a pointer to rows that can explain
    /// themselves.
    ///
    /// Not persona-scoped, like its two siblings: it describes how far
    /// a *disk* has been read.
    async fn unreadable_material_count(&self) -> Result<u64, DomainError>;

    /// How many materials the content axis has **no reading of** — the
    /// rows still carrying
    /// [`NotWalked`](crate::domain::measurement::MeasurementStatus::NotWalked).
    ///
    /// Without this number a content-axis report is a lie by omission:
    /// a row in that state is in no content-axis group and looks from
    /// the outside exactly like a row that has no duplicate.
    ///
    /// # What a non-zero answer means now
    ///
    /// The state is written by the migration that adds the column and
    /// cleared by the next step of the same chain, which reads the files
    /// (`v56_walk_deferred_content_regions`). Both run before the
    /// application accepts a request, so a running build never sees a
    /// library that is merely *waiting* to be walked.
    ///
    /// What survives is the rows that pass could not read: an original
    /// that had been moved or deleted, a disk that was not plugged in
    /// when the upgrade ran. So this counts **originals that are not
    /// where the library says they are** — a real finding about the
    /// user's files rather than a progress bar, and one that does not
    /// move on its own.
    ///
    /// [`unhashed_material_count`](Self::unhashed_material_count) cannot
    /// stand in for it. That one counts open work, and `not-walked` is
    /// an answer — it is what keeps those rows out of the fingerprint
    /// walk in the first place.
    ///
    /// Not persona-scoped, for the same reason its sibling is not: it
    /// describes how far a *disk* has been read, and a per-persona
    /// slice of that would answer a question nobody asks while making
    /// the two numbers disagree about their own scope.
    async fn unwalked_material_count(&self) -> Result<u64, DomainError>;

    /// Puts one raised duplicate question on the queue, returning
    /// whether it was new.
    ///
    /// Keyed by the **unordered pair and the axis**
    /// ([`DuplicateConflict::pair_key`]), so re-detecting a pair adds
    /// nothing. Re-detection is ordinary rather than exceptional: a
    /// re-run of the fingerprint job, a re-import of a path whose row
    /// was folded, and the backfill walk reaching the second row of a
    /// pair the first already reported all produce it. `false` means the
    /// question is already waiting, and the caller is expected to treat
    /// that as success — the queue's job is to hold each question once,
    /// not to count how often the bytes agreed.
    ///
    /// The stored row is verbatim, including the direction and the
    /// reason an automatic fold was declined
    /// ([`FoldExclusion`](crate::domain::duplicate_conflict::FoldExclusion)):
    /// a second detection from the other end does not rewrite which row
    /// the first one called the newcomer, nor what the first one found
    /// to be in the way. The pair is what the question is about, and the
    /// first raising is the one that describes how it was found.
    ///
    /// **Not resolved by this verb.** A question is answered by the
    /// resolution surface, which is a separate wave; this one only
    /// raises.
    async fn record_duplicate_conflict(
        &self,
        conflict: &DuplicateConflict,
    ) -> Result<bool, DomainError>;

    /// Duplicate questions still worth asking, newest first, at most
    /// `limit` of them — what a resolution panel lists.
    ///
    /// "Still worth asking" is two conditions, and only the first is on
    /// this row:
    ///
    /// - **Unanswered** (`resolved_at IS NULL`). An answered question is
    ///   closed rather than deleted, so the record that a conflict was
    ///   raised and ruled on survives — the same reasoning that keeps
    ///   the `identical_to` edge after a `keep`.
    /// - **Both sides still there.** A pair one of whose rows has been
    ///   folded is answered structurally (a headstone is not a thing to
    ///   compare), and a pair one of whose rows is in the trash is not
    ///   worth interrupting anyone over — the row is already on its way
    ///   out, which is the reading
    ///   [`list_duplicate_groups`](Self::list_duplicate_groups) applies
    ///   to the report.
    ///
    /// The second condition is evaluated **against the assets on every
    /// read** rather than written into the queue row, because it can
    /// change back: restoring a row from the trash makes its question
    /// live again, and a stamped-in "resolved: vanished" would have to
    /// be un-stamped by whatever restores it. A fold is one-way, so its
    /// row is filtered forever — which costs one dead row per fold and
    /// keeps one rule instead of two.
    ///
    /// `persona_id`: `None` counts every persona, `Some(id)` restricts
    /// to one — the sidebar's active-persona filter, as everywhere else.
    async fn list_open_duplicate_conflicts(
        &self,
        persona_id: Option<&PersonaId>,
        limit: u32,
    ) -> Result<Vec<DuplicateConflict>, DomainError>;

    /// One queue row by id, answered **whatever its state** — open,
    /// answered, or with a side that has since gone.
    ///
    /// The unfiltered read is the point. A caller naming an id has one
    /// of them in hand from a listing that may be minutes old, and the
    /// three ways that id can have stopped being answerable want three
    /// different things said back
    /// ([`resolve_duplicate_conflict`](crate::application::AssetService::resolve_duplicate_conflict)).
    /// Reusing the listing's filter here would collapse all of them
    /// into "no such conflict", which is the one answer that is not
    /// true.
    async fn find_duplicate_conflict(
        &self,
        id: &DuplicateConflictId,
    ) -> Result<Option<DuplicateConflict>, DomainError>;

    /// Writes the answer onto a row that is **still open**, returning
    /// whether it was this call that wrote it.
    ///
    /// `resolved_at` and `resolution` are set together (V51 checks that
    /// they agree) and nothing is deleted: the record that a question
    /// was raised and ruled on outlives the queue, for the reason the
    /// `identical_to` edge outlives a `kept` ruling.
    ///
    /// **Conditional on `resolved_at IS NULL`, and that is the whole
    /// concurrency story.** The caller reads the row first to decide
    /// what it is answering, and between that read and this write a
    /// second panel can answer the same question; `false` is that race
    /// losing, not an error. Guarding it here rather than in the caller
    /// is the same choice
    /// [`fold_into`](Self::fold_into) makes for its own re-read — the
    /// alternative writes the second answer over the first and enqueues
    /// a fold for a pair somebody already ruled apart.
    ///
    /// Nothing else is written. A `kept` ruling in particular does
    /// **not** set `fold_policy` on either row: that column is a
    /// statement about a row ("this is not a copy of anything") which
    /// suppresses every pair the row takes part in, while the answer
    /// given here is about one pair. The closed row is what keeps this
    /// pair from being asked again — `record_duplicate_conflict`
    /// inserts on `(pair_lo, pair_hi, axis)`, a key with no
    /// `resolved_at` in it, so a re-detection of the same pair adds
    /// nothing and the listing skips the answered row.
    async fn close_duplicate_conflict(
        &self,
        id: &DuplicateConflictId,
        resolution: ConflictResolution,
        resolved_at: DateTime<Utc>,
    ) -> Result<bool, DomainError>;
}

/// Persistence port for [`Tag`] and its many-to-many link with assets.
#[async_trait]
pub trait TagRepository: Send + Sync {
    /// Get-or-create by name (makes the auto-tag job idempotent).
    async fn find_or_create(&self, name: &str) -> Result<Tag, DomainError>;

    /// Returns every tag (used to render the channel sidebar).
    async fn list(&self) -> Result<Vec<Tag>, DomainError>;

    /// Returns the tags attached to an asset (used on the detail view).
    async fn tags_of(&self, asset_id: &AssetId) -> Result<Vec<Tag>, DomainError>;

    /// Returns every tag paired with the number of distinct assets
    /// currently linked to it, ordered by count descending (name
    /// ascending on ties). Tags with zero assets in the query scope
    /// are omitted so the sidebar does not list dead channels.
    ///
    /// `persona_id`:
    /// - `None` — count across every persona.
    /// - `Some(id)` — restrict to assets owned by that persona
    ///   (mirrors the sidebar's active persona filter).
    ///
    /// Feeds the sidebar Tags section (Are.na-style channel list) and
    /// the count badges on tag chips.
    async fn tag_counts(
        &self,
        persona_id: Option<&PersonaId>,
    ) -> Result<Vec<TagCount>, DomainError>;

    /// Renames a tag in place and returns the updated row.
    ///
    /// `name` is expected to be normalised by the caller (the
    /// application layer owns that rule, shared with the attach path).
    ///
    /// - `NotFound` when `id` is unknown.
    /// - `Conflict` when another tag already carries `name`. The
    ///   adapter must **not** silently fold the two together: merging
    ///   channels is [`merge`](Self::merge)'s job, and a rename that
    ///   quietly destroyed a tag would be an unannounced data loss.
    ///   Renaming a tag to its current name is a no-op success.
    ///
    /// The existence and uniqueness checks belong in the same
    /// transaction as the write, or a concurrent rename could slip
    /// between them.
    async fn rename(&self, id: &TagId, name: &str) -> Result<Tag, DomainError>;

    /// Deletes a tag and every `asset_tag` row that referenced it, in
    /// one transaction. Returns the number of links removed (link
    /// rows, so trashed assets count — see
    /// [`TagMergeOutcome`](crate::domain::tag::TagMergeOutcome)).
    /// `NotFound` when `id` is unknown.
    async fn delete(&self, id: &TagId) -> Result<u64, DomainError>;

    /// Folds `source` into `target`: every asset carrying the source
    /// ends up carrying the target (assets that already carried it
    /// keep exactly one link), then the source row is deleted. One
    /// transaction, `NotFound` when either id is unknown.
    ///
    /// With `dry_run` the transaction is rolled back and the returned
    /// counts describe what *would* have happened
    /// (`source_removed = false`). Merge has no undo, so this is the
    /// only way to size the blast radius before committing to it.
    ///
    /// Rejecting `source == target` is the caller's job (it is a
    /// malformed request, not a storage conflict).
    async fn merge(
        &self,
        source: &TagId,
        target: &TagId,
        dry_run: bool,
    ) -> Result<TagMergeOutcome, DomainError>;

    /// Distinct personas owning at least one asset linked to `tag`.
    ///
    /// Feeds the Query-Group invalidation hook on the writes that
    /// change membership wholesale ([`delete`](Self::delete) /
    /// [`merge`](Self::merge)), which — unlike attach / detach — do
    /// not start from an asset whose persona is already in hand.
    /// Trashed assets are included: a Query Group's rule can select
    /// them, and the cost of an extra refresh is a debounce.
    async fn personas_with_tag(&self, tag: &TagId) -> Result<Vec<PersonaId>, DomainError>;

    /// Idempotent m:n link between an asset and a tag.
    async fn link(&self, asset_id: &AssetId, tag_id: &TagId) -> Result<(), DomainError>;

    /// Removes the m:n link (no-op if absent).
    async fn unlink(&self, asset_id: &AssetId, tag_id: &TagId) -> Result<(), DomainError>;
}

/// Persistence port for [`Group`] and its many-to-many link with
/// assets. Unlike [`TagRepository`], `create` is caller-driven
/// (there is no auto-materialisation) — a group only comes into
/// being when the user says so.
#[async_trait]
pub trait GroupRepository: Send + Sync {
    /// Fetches one group by id (`None` when absent). Used by the
    /// command layer to gate hand edits on `kind`.
    ///
    /// Returns trashed groups too — this is the read path `restore` and
    /// the `purge` guard go through, so filtering here would make a
    /// trashed group unreachable by its own id. [`list`](Self::list)
    /// does the excluding.
    async fn find(&self, id: &GroupId) -> Result<Option<Group>, DomainError>;

    /// Creates a group. Fails with `DuplicateGroup` when
    /// `(persona_id, name)` already exists.
    async fn create(
        &self,
        persona_id: PersonaId,
        name: String,
        description: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<Group, DomainError>;

    /// Stamps the group as trashed at `at` (reversible). Idempotent,
    /// and preserves the original stamp so the retention clock does not
    /// restart. `NotFound` when the id is unknown.
    ///
    /// The `asset_bucket` rows are left alone, which is the point: the
    /// membership *and* its hand-arranged `position` survive, so
    /// [`restore`](Self::restore) is a single stamp clear. Member assets
    /// are never touched — a Group is a filing, not a container.
    async fn trash(&self, id: &GroupId, at: DateTime<Utc>) -> Result<(), DomainError>;

    /// Clears the trash stamp. Idempotent; `NotFound` when the id is
    /// unknown.
    async fn restore(&self, id: &GroupId) -> Result<(), DomainError>;

    /// Physically deletes an **already-trashed** group and every
    /// `asset_bucket` row that referenced it (`ON DELETE CASCADE`).
    /// Returns `Conflict` when the group is still live — purge is
    /// reachable only through the trash. A missing id is a no-op.
    async fn purge(&self, id: &GroupId) -> Result<(), DomainError>;

    /// Lists trashed groups whose stamp is older than `cutoff`, oldest
    /// first, capped at `limit`. Sibling of
    /// [`AssetRepository::scan_purgeable`]; the caller computes the
    /// cutoff from an injected retention period.
    async fn scan_purgeable(
        &self,
        cutoff: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<GroupId>, DomainError>;

    /// Returns every **live** group with its live-asset count, ordered
    /// by count descending then name ascending. Persona-scoped when
    /// `persona_id` is `Some`, global otherwise.
    ///
    /// Trashed groups are excluded, and trashed assets do not count
    /// toward the members of the groups that survive.
    async fn list(&self, persona_id: Option<&PersonaId>) -> Result<Vec<GroupSummary>, DomainError>;

    /// Idempotent m:n link between an asset and a group.
    async fn add(
        &self,
        asset_id: &AssetId,
        group_id: &GroupId,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// Bulk append of `ordered` members in one transaction, positions
    /// continuing after the group's current tail. Idempotent per pair
    /// (already-linked assets are skipped). The promote path's
    /// replacement for the per-item `add` loop, which does not hold at
    /// 100k members.
    async fn add_bulk(
        &self,
        group_id: &GroupId,
        ordered: &[AssetId],
        now: DateTime<Utc>,
    ) -> Result<u64, DomainError>;

    /// Stamps the birth record of a promoted Group: the
    /// snapshot its members were materialised from. Write-once by
    /// convention — the promote handler is the only caller.
    async fn set_origin_snapshot(
        &self,
        group_id: &GroupId,
        snapshot_id: &SnapshotId,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// Removes the m:n link (no-op if absent).
    async fn remove(&self, asset_id: &AssetId, group_id: &GroupId) -> Result<(), DomainError>;

    /// Bulk unlink in one transaction; returns the number of rows
    /// actually removed (missing pairs are skipped silently). The
    /// batch-membership counterpart of `add_bulk`.
    async fn remove_bulk(
        &self,
        group_id: &GroupId,
        asset_ids: &[AssetId],
    ) -> Result<u64, DomainError>;

    /// Merges `from` into `into` atomically: every `from` member not
    /// already in `into` is appended after `into`'s current tail
    /// (source position order preserved), then the `from` group row
    /// is deleted (FK cascade drops its remaining links). Returns
    /// the number of members moved. `Conflict` when either group is
    /// missing; callers gate `from != into` and kind/persona rules.
    async fn merge(
        &self,
        from: &GroupId,
        into: &GroupId,
        now: DateTime<Utc>,
    ) -> Result<u64, DomainError>;

    /// Returns the **live** groups the asset currently sits in — used by
    /// the detail overlay to render the "already in these groups" chips.
    /// Trashed groups are excluded: the chips double as toggles, and a
    /// toggle for a group the sidebar does not show has nothing to
    /// toggle against.
    async fn groups_of(&self, asset_id: &AssetId) -> Result<Vec<Group>, DomainError>;

    /// Returns every asset filed in the group, in hand-arranged order.
    /// The filing rows, not the live view: a trashed or folded member
    /// keeps its `asset_bucket` row by design, and the one caller —
    /// fanning a trash-time remark out to the members (#65) — wants
    /// the batch the sentence was said over, not the subset that
    /// happens to be visible.
    async fn member_asset_ids(&self, group_id: &GroupId) -> Result<Vec<AssetId>, DomainError>;

    /// Overwrites the ordering of a group's members. `ordered` is the
    /// full sequence of asset ids in the new front-to-back order; any
    /// asset id not currently in the group is silently ignored (the UI
    /// sends what it has, and drift between the client snapshot and
    /// server state should not fail the write).
    async fn reorder(&self, group_id: &GroupId, ordered: &[AssetId]) -> Result<(), DomainError>;

    /// Renames a group. Fails with `Conflict` when the new name
    /// collides with another group of the same persona, or when the
    /// group does not exist.
    async fn rename(
        &self,
        id: &GroupId,
        name: String,
        now: DateTime<Utc>,
    ) -> Result<Group, DomainError>;

    /// Files the group under a dir (`None` = back to the root level).
    /// Organisation axis only — membership and nesting are untouched.
    /// The target dir must belong to the same persona.
    async fn set_dir(
        &self,
        id: &GroupId,
        dir_id: Option<&DirId>,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// Connects `child` into `parent` (idempotent). Fails with
    /// `Validation` when the two groups belong to different personas
    /// and with `Conflict` when the link would close a cycle —
    /// including the degenerate `parent == child` case.
    async fn link(
        &self,
        parent: &GroupId,
        child: &GroupId,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// Removes the connection (no-op if absent).
    async fn unlink(&self, parent: &GroupId, child: &GroupId) -> Result<(), DomainError>;

    /// Returns every group-in-group connection, persona-scoped when
    /// `persona_id` is `Some`. The UI builds the nesting graph (child
    /// bands, descendant expansion for the filter) from this flat
    /// list, so the SQL filter layer never needs recursive queries.
    async fn links(&self, persona_id: Option<&PersonaId>) -> Result<Vec<GroupLink>, DomainError>;

    /// Overwrites the ordering of a parent's child groups. Same
    /// drift-tolerant contract as [`reorder`](Self::reorder): ids not
    /// currently linked are silently ignored.
    async fn reorder_children(
        &self,
        parent: &GroupId,
        ordered: &[GroupId],
    ) -> Result<(), DomainError>;
}

/// Persistence port for [`Dir`], the sidebar organisation tree.
/// Strictly a navigation structure: dirs contain dirs and groups,
/// never assets, and never participate in asset filtering.
#[async_trait]
pub trait DirRepository: Send + Sync {
    /// Creates a dir under `parent_id` (`None` = root). Fails with
    /// `Conflict` when a sibling of the same name exists.
    async fn create(
        &self,
        persona_id: PersonaId,
        parent_id: Option<DirId>,
        name: String,
        now: DateTime<Utc>,
    ) -> Result<Dir, DomainError>;

    /// Renames a dir. Fails with `Conflict` on a sibling name
    /// collision or a missing dir.
    async fn rename(
        &self,
        id: &DirId,
        name: String,
        now: DateTime<Utc>,
    ) -> Result<Dir, DomainError>;

    /// Re-parents a dir (`None` = to the root). Fails with `Conflict`
    /// when the move would put the dir inside its own subtree (cycle)
    /// or collide with a sibling name, and with `Validation` when the
    /// target parent belongs to a different persona.
    async fn move_to(
        &self,
        id: &DirId,
        new_parent: Option<&DirId>,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// Deletes an **empty** dir. Fails with `Conflict` when the dir
    /// still contains child dirs or groups — emptying it first is the
    /// caller's (user's) deliberate act, so no cascade happens here.
    async fn delete(&self, id: &DirId) -> Result<(), DomainError>;

    /// Returns every dir (persona-scoped when `persona_id` is
    /// `Some`), ordered by `(position, name)`. The client assembles
    /// the tree from the flat `parent_id` list.
    async fn list(&self, persona_id: Option<&PersonaId>) -> Result<Vec<Dir>, DomainError>;
}

/// Persistence port for [`ConstellationEdge`].
///
/// The abstraction is kept narrow enough to sit behind either a dedicated
/// SQLite table or an external graph engine.
#[async_trait]
pub trait EdgeRepository: Send + Sync {
    /// Returns outgoing-only edges (weight-descending, top N).
    ///
    /// Kept for detail views and other callers that specifically want
    /// the `from = asset_id` semantic of the write path. The hover
    /// burst uses [`Self::edges_incident`] instead so that
    /// unidirectionally-written edges surface from both sides.
    async fn edges_of(
        &self,
        asset_id: &AssetId,
        kind: Option<EdgeKind>,
        limit: u32,
    ) -> Result<Vec<ConstellationEdge>, DomainError>;

    /// Returns every edge the asset participates in — as an
    /// [`IncidentEdge`] with a direction hint — regardless of which
    /// side of the row it sits on. Weight-descending, top N.
    ///
    /// Motivation: the `edge_rebuild` job only writes edges from the
    /// newer asset (a v1 decision: edges are one-directional, with
    /// `from` = the newer asset), so a naive
    /// `edges_of(older_asset)` misses the link entirely. This method
    /// is the read-side fix — the write path stays untouched and its
    /// `replace_synth_edges_of` atomic-scope semantics are preserved.
    async fn edges_incident(
        &self,
        asset_id: &AssetId,
        kind: Option<EdgeKind>,
        limit: u32,
    ) -> Result<Vec<IncidentEdge>, DomainError>;

    /// Atomically replaces the **synth** edges originating from
    /// `asset_id` — the unit of work for the `edge_rebuild` job.
    ///
    /// Scoped to [`EdgeKind::is_synth`] on purpose. The job recomputes
    /// time / keyword / co-presence links from scratch every time an
    /// input changes, so it must be free to throw the old set away; but
    /// the same asset can also carry *asserted* links
    /// ([`EdgeKind::DerivedFrom`] written at reify or at a correlated
    /// re-ingest) that nothing can recompute. An unscoped delete takes
    /// both, and the assertion has no second copy to restore from.
    ///
    /// Implementations must ignore any non-synth edge in `edges` rather
    /// than smuggling provenance in through the rebuild path — use
    /// [`Self::add_edges`] for those.
    async fn replace_synth_edges_of(
        &self,
        asset_id: &AssetId,
        edges: Vec<ConstellationEdge>,
    ) -> Result<(), DomainError>;

    /// Inserts asserted edges, leaving every existing row alone.
    ///
    /// The counterpart to [`Self::replace_synth_edges_of`]: provenance
    /// is stated once (dispatch reify, correlated re-ingest) and
    /// accumulates — a later assertion about the same asset must not
    /// erase an earlier one. Re-stating an identical
    /// `(from, to, kind)` is a no-op, so a retried ingest does not
    /// produce a second edge.
    async fn add_edges(&self, edges: Vec<ConstellationEdge>) -> Result<(), DomainError>;
}

/// Persistence port for the pre-generated thumbnail cache
/// (`thumb_cache` table).
///
/// One asset can have multiple sizes cached (256 px, 512 px, …); the
/// (`asset_id`, `size_px`) pair is the primary key. The adapter owns the
/// bytes — callers pass raw encoded image data (typically JPEG or WebP)
/// and receive it back unchanged.
#[async_trait]
pub trait ThumbRepository: Send + Sync {
    /// Inserts or replaces a thumbnail for one asset at one size.
    async fn upsert(
        &self,
        asset_id: &AssetId,
        size_px: u32,
        data: Vec<u8>,
    ) -> Result<(), DomainError>;

    /// Fetches a cached thumbnail (`None` when the pair is not cached).
    async fn get(&self, asset_id: &AssetId, size_px: u32) -> Result<Option<Vec<u8>>, DomainError>;

    /// Fetches many thumbnails at one size, answering **in the order
    /// asked** — slot `i` is the thumbnail for `asset_ids[i]`, `None`
    /// when that pair is not cached.
    ///
    /// A method of its own rather than a loop over [`get`](Self::get)
    /// because the round trip is the cost being removed. The grid asks
    /// for a screenful at a time; a screenful of single fetches is a
    /// screenful of IPC hops and a screenful of statement preparations,
    /// and repeating that per scroll is what put p95 at 10.4 s once the
    /// blob cache stopped absorbing the repeats [measured 2026-08-05,
    /// bench-scroll-v3: 8,263 single fetches over 1,000 jumps].
    ///
    /// Duplicates in `asset_ids` are answered per slot, so a caller
    /// does not have to de-duplicate before asking.
    async fn get_many(
        &self,
        asset_ids: &[AssetId],
        size_px: u32,
    ) -> Result<Vec<Option<Vec<u8>>>, DomainError>;
}

/// Port for reading the full text of an asset's **original source**
/// (the file `SourceRef.locator` points at). The DB only stores the
/// 200-char cover snippet; the session Reader view needs the whole
/// message body, which stays in the source of truth on disk.
///
/// Locator shapes the adapter must understand, and they arrive as
/// variants rather than as spellings it has to take apart:
/// - [`File`](crate::domain::source_locator::SourceLocator::File) — a
///   plain text file; the whole (capped) content.
/// - [`Record`](crate::domain::source_locator::SourceLocator::Record) —
///   one record inside a container file (for example one JSONL line of
///   a Claude Code session, addressed by its message uuid). The adapter
///   extracts just that record's text.
/// - `Remote` / `Logical` — nothing on this disk to open; `None`.
///
/// Read-only by invariant: Asterism never writes back to a locator.
#[async_trait]
pub trait SourceTextReader: Send + Sync {
    /// Resolves each locator to its full text (`None` when the file
    /// is missing, unreadable, or the fragment cannot be found —
    /// callers fall back to the stored cover). Implementations
    /// should batch by container file so a 200-message session costs
    /// one file pass, not 200.
    async fn read_batch(
        &self,
        locators: &[TextLocator],
    ) -> Result<Vec<Option<String>>, DomainError>;
}

/// A locator established to point at text.
///
/// The reason this is a type rather than a `String` is a defect it
/// makes impossible. `read_batch` took a bare locator, and the adapter
/// behind it read whatever was there as lossy UTF-8 under a size cap —
/// no format was in the argument, so none could be checked. Every
/// asset's `IndexRebuild` therefore put its original through that
/// reader, and a 5,000-file PNG corpus went into the body cache and the
/// full-text index whole [measured 2026-08-05].
///
/// A gate in the handler would have fixed that instance. This fixes the
/// shape: the constructor is the only way to obtain the argument, so a
/// caller holding a picture cannot reach the reader at all, and a
/// second call site added later cannot forget the check that no longer
/// exists as a separate step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLocator(SourceLocator);

impl TextLocator {
    /// Accepts the locator only when the format says these bytes are
    /// text.
    ///
    /// `None` for the mime answers `None` here too. An unrecorded
    /// format is "we have not read the bytes", and reading unknown
    /// bytes as text on the chance that they are is precisely the
    /// assumption that failed — the conservative direction costs a
    /// search hit on a file with no extension, and the other costs the
    /// index.
    pub fn new(locator: SourceLocator, mime: Option<&MimeType>) -> Option<Self> {
        mime.filter(|m| m.body_text()).map(|_| Self(locator))
    }

    /// A path whose text-ness is not a fact read off a row.
    ///
    /// A sidecar (`<original>.json`) is written by an exporter to a
    /// path this codebase composes, and no `material` row describes it
    /// — there is no mime to consult, and consulting the original's
    /// would answer for the wrong file. Separate from [`new`](Self::new)
    /// so that skipping the check is a named decision at the call site
    /// rather than an `unwrap` on a check that was never going to fail.
    pub fn of_known_text(locator: SourceLocator) -> Self {
        Self(locator)
    }

    /// The locator, for the adapter that resolves it — as the value,
    /// not as a spelling. The adapter matches on the variant to decide
    /// whether it is opening a file or scanning a container, which is
    /// the question it used to answer by splitting the string itself.
    pub fn locator(&self) -> &SourceLocator {
        &self.0
    }
}

/// Persistence port for the full-text search **body cache** — the
/// resolved plain-text form of each asset's source body, held in
/// SQLite as the durable truth. The Tantivy index (written through
/// [`AssetIndexer`]) is a derived rank-time projection rebuilt from
/// this table when it drifts.
///
/// Kept as its own port so the adapter can bulk-scan for the
/// `IndexRebuild` backfill without touching the hot `Asset` write
/// path.
#[async_trait]
pub trait AssetBodyRepository: Send + Sync {
    /// Upserts the body text for one asset. `indexed_at_ms` is
    /// written by the adapter (unix epoch ms).
    async fn upsert(&self, asset_id: &AssetId, body_text: &str) -> Result<(), DomainError>;
    /// Fetches the body text of one asset (`None` when the row is
    /// missing — e.g. source unreadable at ingest time).
    async fn get(&self, asset_id: &AssetId) -> Result<Option<String>, DomainError>;
    /// Drops the cached body for one asset, answering whether there was
    /// one. Idempotent — a missing row is a no-op that returns `false`.
    ///
    /// The verb this port was missing. A body is composed from what an
    /// asset says about itself
    /// ([`derive_text`](crate::domain::derived_text::derive_text)), and
    /// that population can *shrink* to nothing: a comment thread
    /// emptied, a cover cleared, a title deleted. The indexer is told
    /// to forget such a row, and until this verb existed the cache was
    /// not — leaving a body behind that no longer describes anything,
    /// which the next Tantivy rebuild would read back as truth.
    ///
    /// Deleting rather than writing an empty string, because the two
    /// mean different things to the backfill scan: no row is "nothing
    /// to say", and a row holding `""` would be a composed answer of
    /// zero length.
    ///
    /// The answer is what lets a caller tell "this asset just stopped
    /// having anything to say" from "this asset never had anything to
    /// say" — the common case on a walk over a fresh library, where
    /// retracting a document that was never written costs an index
    /// write and a flush per row for no change.
    async fn delete(&self, asset_id: &AssetId) -> Result<bool, DomainError>;
    /// Clears the composition stamp on one asset's cached body, leaving
    /// the text in place. A missing row is a no-op.
    ///
    /// What a writer calls when it changed the text an asset derives
    /// from and could **not** get the re-index onto the queue. Without
    /// it the row is invisible to recovery: its body was composed by the
    /// current reading, so the backfill — which selects bodies composed
    /// by an older one — passes over it, and the stale document survives
    /// until somebody happens to edit that asset again.
    async fn unstamp(&self, asset_id: &AssetId) -> Result<(), DomainError>;
    /// Bulk page scan for backfill jobs. Returns `(asset_id, body_text)`
    /// pairs in id order, starting from the given cursor (exclusive).
    /// Empty vec signals end of scan. `limit` clamps to a sane page.
    async fn scan_after(
        &self,
        cursor: Option<&AssetId>,
        limit: u32,
    ) -> Result<Vec<(AssetId, String)>, DomainError>;
    /// Row count (backfill progress denominator).
    async fn count(&self) -> Result<u64, DomainError>;
}

/// Ceiling on [`RetrievalQuery::k`], honoured by every retriever.
///
/// One number, stated once, because the two-number version was a lie:
/// callers used to ask for 200 000 candidates while the Tantivy
/// adapter silently clamped the request to 500, so the code read as
/// "we look at everything" and behaved as "we look at the top 500"
/// [measured: 2026-08-06, `SEARCH_CANDIDATE_CAP` vs `MAX_QUERY_LIMIT`].
/// Whatever the value is, the request and the answer now agree on it.
///
/// This is a candidate-window size, not a completeness knob: raising
/// it does not make Retrieval exhaustive, it only makes the shortlist
/// longer and the rank heap bigger. Exhaustive narrowing belongs to
/// the Query side. Right-sizing it is deferred until the candidate
/// count is on screen.
pub const RETRIEVAL_K_CEILING: u32 = 500;

/// What the caller is looking for, in the Retrieval domain's own terms.
///
/// Each variant is a different way of pointing at assets, not a
/// different backend: the same [`AssetRetriever`] answers all of them
/// with whatever machinery it has (BM25 today, embeddings / VLM /
/// agent-driven expansion later).
#[derive(Debug, Clone)]
pub enum RetrievalIntent {
    /// Free text — a phrase, a sentence, a half-remembered word.
    /// The current Tantivy path serves this one.
    Text(String),
    /// "More like this one." Entry point for embedding / VLM routes.
    Similar(AssetId),
}

/// One Retrieval request.
///
/// `k` is how many candidates to look at, **not** a claim about how
/// many assets match: Retrieval answers "the closest N", so there is
/// no total to be truncated from. Narrowing to an exact set is the
/// Query side's job (`ListAssetsQuery`).
#[derive(Debug, Clone)]
pub struct RetrievalQuery {
    /// What to look for.
    pub intent: RetrievalIntent,
    /// Restrict to one persona when `Some`; every persona when `None`.
    pub scope: Option<PersonaId>,
    /// How many candidates to look at. Unrelated to the size of the
    /// matching set. Retrievers clamp this to [`RETRIEVAL_K_CEILING`].
    pub k: u32,
}

/// Why one asset came back as a candidate.
///
/// Kept implementation-neutral so a future retriever can explain
/// itself without the shape changing: a full-text hit has a snippet,
/// a tag-expansion route has the tags it went through, an
/// agent-driven route has its reasoning. A caller that only knows how
/// to render `Snippet` still compiles against every other route.
#[derive(Debug, Clone)]
pub enum Evidence {
    /// Window of the body around the matched terms (highlighted with
    /// `<b>` / `</b>` by the Tantivy renderer).
    Snippet(String),
    /// Reached through these tags (RichTag / agent expansion routes).
    Tags(Vec<TagId>),
    /// Picked for this stated reason (agent-driven routes).
    Rationale(String),
    /// The retriever offered no explanation.
    None,
}

/// One candidate from [`AssetRetriever::retrieve`] — an asset, its
/// rank score, and why it is here.
///
/// Deliberately **not** called a "hit": the result of a retrieval is a
/// ranked shortlist, and treating it as a set is what made the search
/// path disagree with the Query path.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The asset.
    pub asset_id: AssetId,
    /// Persona the asset belongs to (denormalised at index time so a
    /// persona-scope filter can be pushed into the query without a
    /// SQL round-trip per candidate).
    pub persona_id: PersonaId,
    /// Rank score. Comparable within one response, meaningless across
    /// responses or across retriever implementations.
    pub score: f32,
    /// Why this one is a candidate.
    pub evidence: Evidence,
}

/// The answer to one [`RetrievalQuery`] — a ranked shortlist.
#[derive(Debug, Clone)]
pub struct Retrieved {
    /// Candidates in rank order (best first).
    pub candidates: Vec<Candidate>,
    /// The shortlist filled up to `k`, so closer-than-nothing matches
    /// exist beyond it. Callers must not present the response as
    /// exhaustive when this is set — and must not present it as
    /// exhaustive when it is clear either, since Retrieval never
    /// promises completeness. This is the signal for "and more".
    pub truncated: bool,
}

/// Retrieval port — "find me something like this", answered as a
/// ranked shortlist.
///
/// # What this port does not promise
///
/// - **Not a set.** The answer is the top `k` by whatever notion of
///   closeness the implementation has. Asking it "which assets match
///   condition C" is a category error; that question belongs to
///   `AssetRepository` + `ListAssetsQuery`, which answers exactly and
///   can be counted, sorted and paged.
/// - **Not deterministic.** The same query may come back in a
///   different order, or with different members, on a later call.
///   Diversity and freshness are legitimate behaviours here. So
///   results must not be used as a cache key, and must not define the
///   membership of anything persistent (which is why a Query Group's
///   full-text condition is a Query-side predicate, not a retrieval).
///
/// Implementations sit outside the SQLite tx (Tantivy on-disk index
/// today); durability is looser than [`AssetBodyRepository`] and index
/// drift is recovered by re-running the `IndexRebuild` job.
#[async_trait]
pub trait AssetRetriever: Send + Sync {
    /// Returns the ranked shortlist for one query.
    async fn retrieve(&self, query: &RetrievalQuery) -> Result<Retrieved, DomainError>;
}

/// One asset as handed to the retrieval index.
///
/// Every field beyond the identity pair is optional and additive: a
/// route that has no text (an image with no caption yet) still
/// produces a document, and later waves widen this struct rather than
/// the port (embeddings, derived tags, captions).
#[derive(Debug, Clone)]
pub struct IndexDoc {
    /// The asset being indexed.
    pub asset_id: AssetId,
    /// Owning persona, denormalised so scope can be pushed into the
    /// query (see [`Candidate::persona_id`]).
    pub persona_id: PersonaId,
    /// Resolved plain-text body, when the asset has one.
    pub text: Option<String>,
}

/// Ingest side of retrieval — keeps the index in step with the assets.
///
/// Split from [`AssetRetriever`] because the callers are disjoint:
/// trash / purge / rebuild paths write and never read, the read path
/// reads and never writes. Keeping them apart means a service's field
/// list states which half it actually touches.
///
/// `flush` is required after any series of `upsert` / `remove` before
/// the changes are retrievable; batching a whole `IndexRebuild` page
/// before one flush is the intended pattern.
#[async_trait]
pub trait AssetIndexer: Send + Sync {
    /// Adds or replaces the document for one asset. Idempotent by
    /// `asset_id`. Not retrievable until `flush` runs.
    async fn upsert(&self, doc: &IndexDoc) -> Result<(), DomainError>;

    /// Removes the document for one asset. No-op when absent.
    async fn remove(&self, asset_id: &AssetId) -> Result<(), DomainError>;

    /// Flushes pending writes so subsequent retrievals see them.
    /// Flushes are ~10-100 ms in v1 [estimated, tantivy 0.25]; batch first.
    async fn flush(&self) -> Result<(), DomainError>;
}

/// Port for enqueueing background jobs. The adapter wraps `apalis`
/// `SqliteStorage`.
///
/// Job lifecycle (pending / running / retry) is owned by apalis — the
/// application layer only enqueues; execution, retry, and persistence
/// belong to the engine.
#[async_trait]
pub trait JobQueue: Send + Sync {
    /// Enqueues one job and returns the engine-assigned task id.
    async fn enqueue(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
    ) -> Result<String, DomainError>;

    /// Enqueues one job with an explicit priority hint. Higher
    /// priority = popped first. `0` is the neutral default and is
    /// equivalent to [`enqueue`](Self::enqueue). Adapters that do
    /// not honour priority fall back to normal FIFO order.
    async fn enqueue_with_priority(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
        priority: i32,
    ) -> Result<String, DomainError> {
        // Default: adapters not overriding this get FIFO order —
        // priority is a hint, not a contract, so callers using it
        // still get a job enqueued even against a legacy backend.
        let _ = priority;
        self.enqueue(kind, payload).await
    }

    /// Whether a **queued** (not yet picked up) batch job of `kind` —
    /// one whose payload carries `"batch": true` — is already sitting
    /// in the queue.
    ///
    /// Exists for the startup backfill dedupe: the self-chaining walks
    /// (`material_hash` batch pages) survive a restart in the durable
    /// queue, and blindly enqueueing a fresh `cursor: null` walk on
    /// every boot runs it in parallel with the resumed one — the same
    /// files read twice. Deliberately counts *queued* rows only, not
    /// running ones: a running row orphaned by a crash is never fetched
    /// again (no orphan-reenqueue is configured), and treating it as
    /// "in flight" would suppress the backfill on every boot thereafter.
    ///
    /// Default is `false` ("nothing queued") so in-memory test doubles
    /// — which do not persist a queue across anything — stay correct
    /// without overriding.
    async fn has_pending_batch(&self, kind: JobKind) -> Result<bool, DomainError> {
        let _ = kind;
        Ok(false)
    }
}

/// Port for pushing job progress to the UI. In Tauri, the adapter emits
/// `job:progress:{id}` events; in the standalone server it logs.
#[async_trait]
pub trait ProgressEmitter: Send + Sync {
    /// Pushes a single progress payload. Emitter failures should not tear
    /// the job down — callers may swallow the error at their discretion.
    async fn emit(&self, job_id: &str, progress: Progress) -> Result<(), DomainError>;

    /// Broadcasts a global UI event (not scoped to a `job:progress:{id}`
    /// channel). Used by long-running jobs — the SessionRebuild
    /// handler emits `sessions:progress` here so the UI can show a
    /// build indicator without knowing the apalis task id upfront
    /// (auto-enqueued rebuilds have no caller-visible id).
    ///
    /// Default is a no-op so backends that do not have a global event
    /// bus (`LogEmitter`) can ignore it.
    async fn broadcast(
        &self,
        _event: &str,
        _payload: serde_json::Value,
    ) -> Result<(), DomainError> {
        Ok(())
    }
}

/// Persistence port for [`Snapshot`] — the immutable content-addressed
/// freeze of an ordered asset set.
///
/// The port is deliberately narrow — no list, no rename, no delete:
/// snapshots are *created (or reused by content hash)* and *read*. They
/// are never listed, renamed, or deleted through this port — deletion is
/// the province of the later GC job, so no `delete` appears here.
#[async_trait]
pub trait SnapshotRepository: Send + Sync {
    /// Materialises `snapshot`, or reuses the existing row that shares
    /// its `(persona_id, content_hash)` (content dedupe). On a fresh insert
    /// the ordered members are bulk-written in the *same transaction*; on
    /// reuse nothing is written and the pre-existing row (its id +
    /// `created_at`) is returned. The returned entity's members equal the
    /// input (the content hash guarantees identical ordered membership).
    async fn create_or_reuse(&self, snapshot: &Snapshot) -> Result<Snapshot, DomainError>;

    /// Fetches one snapshot (with its ordered members) by surrogate id.
    /// The "open it from its referencing source" read.
    async fn find(&self, id: &SnapshotId) -> Result<Option<Snapshot>, DomainError>;

    /// Lists snapshots whose frozen members include the given asset — the
    /// P5 reverse lookup used on the detail panel so a user can jump from
    /// a card to every freeze that ever included it. Most-recent first,
    /// capped at `limit`.
    async fn list_containing_asset(
        &self,
        asset_id: &AssetId,
        limit: u32,
    ) -> Result<Vec<Snapshot>, DomainError>;
}

/// Persistence port for [`DispatchJob`].
#[async_trait]
pub trait DispatchRepository: Send + Sync {
    /// Fetches one dispatch job by id.
    async fn find(&self, id: &DispatchId) -> Result<Option<DispatchJob>, DomainError>;

    /// Upserts. Callers stamp `updated_at`; adapters do not clock.
    async fn save(&self, job: &DispatchJob) -> Result<(), DomainError>;

    /// Lists dispatch jobs, most-recent first, filtered by persona
    /// (`None` = all), snapshot (`None` = all), and state slug
    /// (`None` = all). Adapters combine the non-`None` predicates with
    /// AND.
    async fn list(
        &self,
        persona_id: Option<&PersonaId>,
        snapshot_id: Option<&SnapshotId>,
        state_slug: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DispatchJob>, DomainError>;
}

// `PursuitRepository` and `ProjectRepository` were declared here, among
// the raw layer's own. They are the forge's storage contract and moved
// to `domain::forge::repository` unchanged; the header says what that
// leaves this file.

/// Persistence port for the Query Group evaluation core.
/// A Query Group stores a *rule*
/// (`query_json`) and its members are materialised into `asset_bucket`
/// rows by an evaluation pass. This port carries the three persistence
/// primitives that pass needs; the orchestration (parse → expand → filter
/// → intersect → sort → materialize) lives in
/// [`QueryGroupService`](crate::application::query_group_service::QueryGroupService),
/// which W2 (migration-time synchronous first eval) and W4 (refresh job)
/// both drive.
///
/// The three methods stay on their own port rather than bloating
/// [`AssetRepository`] / [`GroupRepository`] because they are one cohesive
/// concern (evaluate-and-freeze) and share no caller with the hot read
/// path.
#[async_trait]
pub trait QueryGroupRepository: Send + Sync {
    /// Expands a set of **raw** (un-expanded) group ids to their
    /// descendant closure through the `bucket_link` nesting graph — the
    /// same recursive-CTE reachability walk the link cycle check uses.
    /// The result is the input ids that exist plus every group reachable
    /// as a descendant, deduplicated. Query-group children are walked too
    /// (a topological run has already materialised the dependency).
    ///
    /// An empty input yields an empty vector (no query issued). The raw
    /// ids are kept raw at rest and expanded here at evaluate time so the
    /// membership stays faithful to later nesting edits.
    async fn expand_group_closure(&self, raw: &[GroupId]) -> Result<Vec<GroupId>, DomainError>;

    /// Evaluates the SQL filter with **no `LIMIT`** and returns every
    /// matching asset carrying exactly the columns the sort evaluator
    /// reads ([`SortableAsset`](crate::domain::sort_eval::SortableAsset)).
    /// `group_ids` and `labels` are resolved without an N+1 (one bulk
    /// `asset_bucket` join, mirroring `list_index`). Measured at
    /// 68 ms / 100k rows.
    ///
    /// The `query.limit` / `query.offset` fields are ignored — the caller
    /// evaluates the whole set. `search_text` is **not** applied here
    /// (Tantivy is joined separately by the service).
    async fn fetch_sortable_assets(
        &self,
        query: &crate::domain::asset::AssetQuery,
    ) -> Result<Vec<crate::domain::sort_eval::SortableAsset>, DomainError>;

    /// Bulk-replaces a bucket's membership in one transaction: `DELETE`
    /// every `asset_bucket` row for `bucket_id`, then insert `ordered`
    /// at `position = 0, 1, 2, …`. Returns the number of rows written.
    ///
    /// This is the write half of "materialize". The single tx is
    /// mandatory — the existing promote path's app-loop 1-row insert
    /// (`selection_service.rs`) does not hold at 100k members.
    async fn replace_membership(
        &self,
        bucket_id: &GroupId,
        ordered: &[AssetId],
        now: DateTime<Utc>,
    ) -> Result<u64, DomainError>;

    /// Lists every query-kind bucket (`kind = 'query'`) with its stored
    /// rule, in a deterministic order. Feeds the startup refresh (W2b),
    /// the W4 refresh job, and the cycle-guard graph build.
    async fn list_query_groups(&self) -> Result<Vec<QueryGroupRow>, DomainError>;

    /// The group's current membership in `position` order — the member
    /// list a dispatch freezes. Works for both kinds (manual
    /// hand order / query materialised order).
    async fn member_ids(&self, bucket_id: &GroupId) -> Result<Vec<AssetId>, DomainError>;

    /// Mints a `kind='query'` Group carrying `query_json` ("Save as
    /// Group"). Name uniqueness maps to `Conflict` exactly like
    /// the manual-group create.
    async fn create_query_group(
        &self,
        persona_id: PersonaId,
        name: String,
        query_json: String,
        now: DateTime<Utc>,
    ) -> Result<Group, DomainError>;

    /// Rewrites the stored rule of an existing query group. Errs when
    /// the id is unknown or the bucket is not `kind='query'` (the
    /// caller has already validated the blob + the cycle guard).
    async fn set_query_json(
        &self,
        bucket_id: &GroupId,
        query_json: &str,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// Stamps the outcome of one refresh run on the bucket (W4-b
    /// failure signal): `status` is `"ok"` / `"failed"`, `error`
    /// carries the failure text for the UI tooltip (`None` on
    /// success). Deliberately does **not** bump `updated_at` — a
    /// refresh stamp is telemetry, not a user edit. Unknown ids are a
    /// no-op (the group may have been deleted mid-refresh).
    ///
    /// KNOWN LIMITATION: the stamp is a separate write from the
    /// membership replace, so two concurrent evaluates of the same
    /// bucket (pre-dispatch refresh racing the refresh job) can leave
    /// a status that belongs to the loser's evaluation. Telemetry
    /// only — the next refresh self-heals; fold both writes into one
    /// isle closure if the chip ever needs hard consistency.
    async fn mark_refresh_result(
        &self,
        bucket_id: &GroupId,
        status: &str,
        error: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<(), DomainError>;
}

/// One query group as listed by
/// [`QueryGroupRepository::list_query_groups`].
#[derive(Debug, Clone)]
pub struct QueryGroupRow {
    /// The bucket that owns the rule and receives the materialisation.
    pub bucket_id: GroupId,
    /// Owning persona — scopes the evaluation.
    pub persona_id: PersonaId,
    /// The stored `query_json` v1 blob.
    pub query_json: String,
}

/// Persistence port for [`Thread`] and its [`Message`] children —
/// the app-level container that unifies UI-authored notes and
/// HTTP-authored (Claude Code / agent) writes on the same rows.
///
/// Message writes are append-only (there is no `edit_message`); the
/// only mutation on an existing row is `delete_message`. The
/// adapter is responsible for maintaining the projection columns
/// (`last_message_at`, `message_count`, `updated_at`) on `threads`
/// as `messages` change — a SQLite trigger is the intended
/// implementation.
///
/// Idempotency: `append_message` accepts an optional
/// `idempotency_key`. When the pair `(thread_id, idempotency_key)`
/// already exists the adapter returns the pre-existing row instead
/// of inserting a duplicate; callers get an `Ok` either way. This
/// makes remote-agent retries safe without any deletion dance.
#[async_trait]
pub trait ThreadRepository: Send + Sync {
    /// Fetches one Thread (with its projection columns filled in)
    /// by surrogate id.
    async fn find(&self, id: &ThreadId) -> Result<Option<Thread>, DomainError>;

    /// Lists Threads that match `anchor`, most-recently-active first
    /// (`last_message_at DESC`, `created_at DESC` on ties).
    /// Archived Threads are excluded unless `include_archived` is
    /// set.
    async fn list_by_anchor(
        &self,
        anchor: &ThreadAnchor,
        include_archived: bool,
    ) -> Result<Vec<Thread>, DomainError>;

    /// Upserts a Thread row. Used for creation and for
    /// title / archive-flag mutations. Adapters preserve the
    /// projection columns (`last_message_at`, `message_count`)
    /// unchanged — those are trigger-owned.
    async fn save(&self, thread: &Thread) -> Result<(), DomainError>;

    /// Deletes a Thread and every Message attached to it
    /// (`ON DELETE CASCADE`). Idempotent — a missing id is a no-op.
    async fn delete(&self, id: &ThreadId) -> Result<(), DomainError>;

    /// Lists the Thread's Messages in chronological (`created_at`
    /// ascending) order. `since` is exclusive — pass the greatest
    /// timestamp the caller has seen to poll for new writes.
    async fn list_messages(
        &self,
        thread_id: &ThreadId,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<Message>, DomainError>;

    /// Appends a Message. When `idempotency_key` is `Some` and the
    /// pair `(thread_id, idempotency_key)` already exists, the
    /// adapter returns the pre-existing row instead of creating a
    /// duplicate.
    async fn append_message(
        &self,
        message: &Message,
        idempotency_key: Option<&str>,
    ) -> Result<Message, DomainError>;

    /// Deletes one Message. Idempotent — a missing id is a no-op.
    async fn delete_message(&self, id: &MessageId) -> Result<(), DomainError>;
}

/// Persistence port for [`AssetComment`] — the per-Asset thread of
/// User / Persona notes.
#[async_trait]
pub trait AssetCommentRepository: Send + Sync {
    /// Persists a new comment (or replaces one that already exists on
    /// the same id, so `edit` reuses the same path). Adapters stamp
    /// `updated_at`-style side effects (none for the MVP row).
    async fn save(&self, comment: &AssetComment) -> Result<(), DomainError>;

    /// Fetches every comment attached to `asset_id`, oldest first
    /// (matches the natural conversation reading order).
    async fn list_by_asset(&self, asset_id: &AssetId) -> Result<Vec<AssetComment>, DomainError>;

    /// Fetches one comment by its own id (`None` when it is not
    /// there).
    ///
    /// The thread walk above cannot answer this: it is keyed by the
    /// asset, and the one command that carries a comment id alone is
    /// `delete`. What that command needs before the row goes is the
    /// asset it was attached to — the comment is a section of that
    /// asset's derived text, so deleting one makes the asset's search
    /// document stale, and the id of the asset to re-index is only
    /// readable while the row still exists.
    async fn find(&self, id: &AssetCommentId) -> Result<Option<AssetComment>, DomainError>;

    /// Deletes a single comment. Idempotent — a missing id is a
    /// no-op.
    async fn delete(&self, id: &AssetCommentId) -> Result<(), DomainError>;
}

/// Persistence port for [`MaterialMark`] — the marks placed into an
/// Asset's material.
///
/// Same three verbs as [`AssetCommentRepository`], and deliberately no
/// more: no range fetch, no cap. The index behind `list_by_asset` is
/// `(asset_id, start_ms)`, so a `[t0, t1)` fetch is a signature change
/// against the schema as it stands, to be made when a library with
/// thousands of marks on one asset exists to make it against.
#[async_trait]
pub trait MaterialMarkRepository: Send + Sync {
    /// Persists a mark, replacing the row on the same id (so an edit
    /// reuses this path).
    async fn save(&self, mark: &MaterialMark) -> Result<(), DomainError>;

    /// Fetches every mark in `asset_id`'s material in **timeline
    /// order** (`start_ms` ascending, ties broken by id) — the order
    /// they are read in, which is not the order they were placed in.
    ///
    /// The order is stated in terms of the temporal anchor because that
    /// is the only anchor there is. A second coordinate space arrives
    /// with a listing order of its own, and the wording here is what
    /// has to be settled then (see the V61 doc comment in
    /// `migrations.rs` for the SQL side of the same question).
    async fn list_by_asset(&self, asset_id: &AssetId) -> Result<Vec<MaterialMark>, DomainError>;

    /// Deletes one mark. Idempotent — a missing id is a no-op.
    async fn delete(&self, id: &MaterialMarkId) -> Result<(), DomainError>;
}

/// Persistence port for [`MaterialLayer`] — the bands of marks over an
/// Asset's material.
///
/// Four verbs, and the fourth is the one worth explaining.
/// [`set_default`](MaterialLayerRepository::set_default) is not
/// `save` called twice: at most one layer per
/// `(asset_id, material_ord, role)` may carry the flag, so moving it
/// means clearing the old holder and setting the new one, and a caller
/// doing that through two `save`s gets a window where the pair is
/// briefly both — which the schema's partial unique index aborts on —
/// or briefly neither, which the lazy-creation path in the service
/// reads as "this asset has no band" and answers by making a second
/// one. One statement, one transaction, adapter's problem.
///
/// No `find_default` verb. The default is one row of
/// [`list_by_asset`](MaterialLayerRepository::list_by_asset)'s answer,
/// and an asset carries a handful of layers, not thousands; a dedicated
/// lookup would be a second statement that can disagree with the
/// listing about which band is current.
#[async_trait]
pub trait MaterialLayerRepository: Send + Sync {
    /// Persists a layer, replacing the row on the same id.
    ///
    /// Does **not** move the default flag between rows: writing
    /// `is_default = true` here on a second layer of the same
    /// `(asset_id, material_ord, role)` is refused by the unique index.
    /// [`Self::set_default`] is the verb that moves it.
    async fn save(&self, layer: &MaterialLayer) -> Result<(), DomainError>;

    /// Fetches one layer by id. `None` when no row carries it.
    async fn find(&self, id: &MaterialLayerId) -> Result<Option<MaterialLayer>, DomainError>;

    /// Fetches every band over `asset_id`'s materials, ordered by
    /// `(material_ord, role, ord, id)` — the display order, with `id`
    /// as the tie-break so the answer is total.
    ///
    /// Every material of the asset in one answer, rather than a
    /// `(asset, ord)` fetch: an asset carries one original in practice
    /// and a handful at most, and the caller that renders a switch
    /// wants the whole set anyway.
    async fn list_by_asset(&self, asset_id: &AssetId) -> Result<Vec<MaterialLayer>, DomainError>;

    /// Makes `id` the default band for its own
    /// `(asset_id, material_ord, role)`, clearing whichever row held it
    /// before — in one transaction (see the trait doc).
    ///
    /// `NotFound` when no layer carries `id`. Idempotent: naming the
    /// row that already holds the flag leaves it holding the flag.
    async fn set_default(&self, id: &MaterialLayerId) -> Result<(), DomainError>;

    /// Deletes a layer and everything in it (`ON DELETE CASCADE`
    /// reaches both `chapter_mark` and `material_mark`). Idempotent —
    /// a missing id is a no-op.
    ///
    /// Deliberately blunt: the marks in a band are the band, so there
    /// is no state in which deleting one and keeping the other is the
    /// answer. Whether a *particular* band may be deleted at all —
    /// an imported one, the last default — is the service's rule, in
    /// the same place the rest of the origin guard lives.
    async fn delete(&self, id: &MaterialLayerId) -> Result<(), DomainError>;
}

/// Persistence port for [`ChapterMark`] — the sections a structure
/// layer declares.
///
/// The verbs are the mark port's three plus
/// [`replace_layer_content`](ChapterMarkRepository::replace_layer_content),
/// which is what re-reading a file does. That one exists as a port verb
/// rather than as "delete the layer's rows, then save each" because the
/// two are not the same operation: the second leaves a window in which
/// the file's chapter list is empty — visible to any concurrent read,
/// and permanent if the process dies mid-way, with nothing left to
/// re-derive it from until the next probe. Replacement is atomic or it
/// is a data-loss path with extra steps.
#[async_trait]
pub trait ChapterMarkRepository: Send + Sync {
    /// Persists one chapter, replacing the row on the same id (so an
    /// edit reuses this path).
    async fn save(&self, chapter: &ChapterMark) -> Result<(), DomainError>;

    /// Fetches a layer's chapters in reading order (`ord` ascending,
    /// then `start_ms`, then `id`).
    ///
    /// `ord` leads because it is the order the container declared, and
    /// a container may declare its sections out of timeline order;
    /// `start_ms` and `id` follow so that the answer is total even when
    /// a writer left every `ord` at zero.
    async fn list_by_layer(
        &self,
        layer_id: &MaterialLayerId,
    ) -> Result<Vec<ChapterMark>, DomainError>;

    /// Deletes one chapter. Idempotent — a missing id is a no-op.
    async fn delete(&self, id: &ChapterMarkId) -> Result<(), DomainError>;

    /// Replaces everything in `layer_id` with `chapters`, atomically
    /// (see the trait doc). An empty slice is a legal argument and
    /// means "this material declares no chapters" — the answer a
    /// re-probe of a file that had them and no longer does must be able
    /// to record.
    ///
    /// Refuses a chapter whose `layer_id` is not the one named, rather
    /// than silently rehoming it: the argument would otherwise be two
    /// disagreeing statements of where the rows go.
    async fn replace_layer_content(
        &self,
        layer_id: &MaterialLayerId,
        chapters: &[ChapterMark],
    ) -> Result<(), DomainError>;
}

/// The `(asset, material, role)` triple a layer lookup is scoped by.
///
/// Carried as a value rather than as three positional arguments because
/// the first two are both ordinals of the same shape and the third is
/// what tells "the chapter bands" from "the note bands" — a call site
/// that transposes them compiles either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerScope {
    /// Asset whose material the bands are over.
    pub asset_id: AssetId,
    /// Which of that asset's originals (`0` = the primary one).
    pub material_ord: u32,
    /// Which kind of band.
    pub role: LayerRole,
}

/// One `(material, rule)` pair nothing has answered yet — the unit the
/// series walk hands out.
///
/// Everything [`derive`](crate::domain::series::derive) needs and nothing
/// else, because that is the whole of what the derivation reads: no
/// locator, no bytes, no asset. A pair is a row join, so re-deriving a
/// library is a scan and a parse — the property the
/// [`series`](crate::domain::series) module doc sells the axis on —
/// rather than a pass over somebody's disk.
///
/// The rule travels **with** the pair rather than being looked up by id
/// against a separately-fetched list. One statement is one consistent
/// read: a rule registered or deleted between two reads would otherwise
/// leave the caller holding a `strategy_id` it cannot resolve, and the
/// only thing it could do with one is skip — which writes no row and puts
/// the pair straight back in the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnderivedSeries {
    /// Owning asset.
    pub asset_id: AssetId,
    /// Position within that asset (`0` = primary original). Part of the
    /// unit because `material_series` is keyed by it: an asset with two
    /// originals carries two containers and gets an answer per original.
    pub ord: u32,
    /// What the material's row says the format is — `material.mime`,
    /// parsed at the same boundary the entity's is.
    ///
    /// Carried rather than compared in SQL, and that is the mime gate
    /// decision: see [`SeriesRepository::scan_underived`].
    pub mime: Option<MimeType>,
    /// The container's metadata, taken apart — the same map
    /// [`Material::meta_fields`](crate::domain::material::Material::meta_fields)
    /// produces, from the same column.
    ///
    /// A column that will not parse arrives as an empty map rather than
    /// as an error or a skipped row. The walk has to answer every pair it
    /// offers (see [`scan_underived`](SeriesRepository::scan_underived)),
    /// and an empty map is answered honestly: no keyword any rule names is
    /// present, so every rule replies
    /// [`NotApplicable`](crate::domain::series::SeriesKey::NotApplicable).
    pub meta_kv: std::collections::BTreeMap<String, String>,
    /// The rule to apply.
    pub strategy: Strategy,
}

/// One `series_strategy` row: the rule, and what the row says about
/// itself.
///
/// The provenance travels beside the rule rather than on it because
/// [`Strategy`](crate::domain::series::Strategy) is what
/// [`derive`](crate::domain::series::derive) reads, and neither stamp
/// nor the `system` flag is an input to a key. Putting them on the
/// domain type would mean every fixture that exercises the derivation
/// had to invent a creation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredStrategy {
    /// The rule itself.
    pub strategy: Strategy,
    /// Whether a migration seeded this row.
    ///
    /// Provenance, not permission — a seeded rule may be edited and
    /// deleted like any other (V73's doc comment holds the argument).
    /// What it buys is the pair below: `system = 1 AND
    /// updated_at = created_at` is how a later corrective migration
    /// tells a pristine seed from one somebody took over.
    pub system: bool,
    /// When the rule was registered.
    pub created_at: DateTime<Utc>,
    /// When it was last written. Equal to
    /// [`created_at`](Self::created_at) until something edits it, which
    /// is the property the identification above rests on.
    pub updated_at: DateTime<Utc>,
}

/// Persistence port for the series axis — the [`Strategy`] rules
/// themselves (`series_strategy`, V73) and the answers they derive
/// (`material_series`).
///
/// Two populations behind one port because they are one axis and
/// neither is usable without the other: a key means nothing without the
/// rule it was derived under, and the rule's id is what the key is filed
/// by. Splitting them would put the foreign key across two ports.
///
/// **Still nothing reads a derived key back.** [`scan_underived`] reads
/// `material_series` only to ask which pairs are *missing* from it, and
/// that is the slice rather than an omission: what a reader wants is "the
/// materials sharing this key" — a grouping query whose shape is decided
/// by the surface asking for it, and the surfaces do not exist yet.
/// Guessing at it now would ship a signature to be replaced. Whatever
/// that statement turns out to be, it has to name
/// [`SERIES_RESERVED_VALUES`](crate::domain::series::SERIES_RESERVED_VALUES)
/// the way the duplicate report's adapter names its own reserved list.
///
/// The registration surface arrived without changing that. Its `GET`
/// lists the **rules**, which is what a caller about to write one needs,
/// and a rule listing groups nothing — so the reserved-value obligation
/// is still owed by a statement nobody has written, and the paragraph
/// above still describes the state of this port. Adding a group
/// projection to that listing would have been the guess: the shape it
/// wants (per key with counts? per material? paged how?) follows from a
/// reader, and the reader this axis is for — a person promoting one
/// series into a real Group, which is why the key sits on the material
/// rather than in `asset_bucket` ([`series`](crate::domain::series),
/// "A key on the material, and not a Group") — is a slice nobody has
/// reached.
///
/// [`update_strategy`] and [`delete_strategy`] arrived with the surface
/// that calls them (`/asterism/series-strategies`), and
/// [`clear_derived`] with them: an edit invalidates the keys derived
/// under the id it edits, and *when those are recomputed* — the one
/// question the design left open — is "delete that rule's rows and the
/// walk derives them again", the whole of it. Held back until there was
/// a caller for the reason that argument implies — a `delete` with
/// nobody calling it is a way to leave a library holding keys from a
/// rule that no longer exists.
///
/// [`scan_underived`]: SeriesRepository::scan_underived
/// [`update_strategy`]: SeriesRepository::update_strategy
/// [`delete_strategy`]: SeriesRepository::delete_strategy
/// [`clear_derived`]: SeriesRepository::clear_derived
#[async_trait]
pub trait SeriesRepository: Send + Sync {
    /// Every registered rule, system-seeded and user-written alike,
    /// oldest first, each with the provenance its row carries.
    ///
    /// No `system` filter and no paging. The flag is provenance rather
    /// than a visibility class (see the V73 doc comment), so a caller
    /// that wants one half can say so in Rust, and the population is
    /// bounded by how many rules a person writes.
    ///
    /// One listing rather than a bare-rule one for the derivation and a
    /// provenance-bearing one for the surface. Two statements over one
    /// table are two column lists that can come to disagree about which
    /// rules exist, and the derivation simply borrows
    /// [`RegisteredStrategy::strategy`] and ignores the rest.
    async fn list_strategies(&self) -> Result<Vec<RegisteredStrategy>, DomainError>;

    /// One rule by id, or `None` when nothing is registered under it.
    ///
    /// The read half of a partial update: a `PATCH` that names three
    /// fields has to resolve the other two against what is stored, and
    /// resolving them against a listing would be the same read with a
    /// scan in front of it.
    async fn find_strategy(
        &self,
        id: &StrategyId,
    ) -> Result<Option<RegisteredStrategy>, DomainError>;

    /// Registers a rule, stamping both timestamps with `at`.
    ///
    /// The id comes from the caller ([`Strategy::id`]) rather than being
    /// minted here, so the value a caller holds is the value keys are
    /// filed under from the first derivation onward.
    ///
    /// `at` is an argument for the reason every other write in this
    /// codebase takes one: a clock read inside an adapter is a clock a
    /// test cannot hold still.
    async fn create_strategy(
        &self,
        strategy: &Strategy,
        at: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// Overwrites a registered rule's five fields and stamps
    /// `updated_at` with `at`. `NotFound` when the id names nothing.
    ///
    /// **The stamp is not optional and not conditional.** V73's doc
    /// comment tells a pristine seed from a rule somebody took over by
    /// `system = 1 AND updated_at = created_at`, and a corrective
    /// migration is written against that test — so a write that left the
    /// stamp alone would make an edited rule readable as untouched, and
    /// the next numbered step would overwrite somebody's work. That
    /// paragraph was written before any code could edit a rule; this is
    /// the code, and this sentence is the promise being kept.
    ///
    /// `system` and `created_at` are not arguments. The first records
    /// that a migration wrote the row, which no later write makes truer
    /// or falser; the second is when it did.
    async fn update_strategy(
        &self,
        strategy: &Strategy,
        at: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// Removes a rule. `NotFound` when the id names nothing.
    ///
    /// The keys derived under it go too — `material_series.strategy_id`
    /// cascades (V73) — and that is the deletion, not a side effect of
    /// it: a key means nothing without the rule it was derived under.
    ///
    /// Absent rather than idempotent, unlike
    /// [`MaterialMarkRepository::delete`]. Deleting a rule discards
    /// every key it derived, so answering "done" to an id that named
    /// nothing would tell a caller its library had changed when it had
    /// not.
    ///
    /// [`MaterialMarkRepository::delete`]: MaterialMarkRepository::delete
    async fn delete_strategy(&self, id: &StrategyId) -> Result<(), DomainError>;

    /// Deletes every answer filed under one rule and reports how many
    /// there were — **invalidation, in full**.
    ///
    /// There is nothing else to it because the walk's population is
    /// "a `(material, rule)` pair with no row"
    /// ([`scan_underived`](Self::scan_underived)): a pair with a stale
    /// row is one no pass can ever re-offer, and a pair with no row is
    /// one the next pass answers. So an edit expresses itself by
    /// removing its own rows, and there is no dirty flag and no
    /// detector anywhere on this axis.
    ///
    /// **Scoped to the id, and that is load bearing.** Clearing the
    /// table would re-derive the whole library — the difference between
    /// a keystroke and a sweep on a table that is the library times the
    /// rules. V74's index is what makes the scoped statement a seek.
    ///
    /// The count is returned because the caller's next act is to enqueue
    /// the walk, and "how many keys did that edit throw away" is the one
    /// number that says what the edit cost.
    async fn clear_derived(&self, id: &StrategyId) -> Result<u64, DomainError>;

    /// Files what one rule concluded about one material, replacing any
    /// earlier answer for the same `(material, strategy)` pair.
    ///
    /// **Takes the [`SeriesKey`] whole.** Not an `Option<String>`: the
    /// two ways of having no key lead somewhere different — a rule that
    /// is not about this material is working as written, a rule that is
    /// and found nothing needs fixing — and a caller asked to flatten
    /// them is a caller who decides which, at every call site, from
    /// whatever it happens to know. The column pair is held to the same
    /// distinction by a `CHECK`, so a flattened caller would be turning
    /// a fact into an error a layer later.
    ///
    /// The material is named by `(asset_id, ord)` because that is its
    /// identity (`material`'s primary key); an asset with two originals
    /// gets an answer per original.
    async fn record(
        &self,
        asset_id: &AssetId,
        ord: u32,
        strategy_id: &StrategyId,
        key: &SeriesKey,
        at: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    /// `(material, rule)` pairs with metadata and no answer, oldest
    /// material first, at most `limit` of them — the derivation walk's
    /// page.
    ///
    /// The population is every material carrying a `meta_kv` object
    /// crossed with every registered rule, minus the pairs
    /// `material_series` already holds a row for. That one predicate is
    /// all three reasons to recompute — a new material, a new rule, a
    /// rule whose rows were deleted to invalidate it — which is why no
    /// detector and no dirty flag exists anywhere on this axis (see
    /// [`JobKind::SeriesDerive`]).
    ///
    /// # The caller must answer every pair it is handed
    ///
    /// **The walk shrinks only because each pair leaves it**, and a pair
    /// leaves by acquiring a row — including the two rows that are not
    /// keys ([`NothingToSelect`] and [`NotApplicable`]). A caller that
    /// filed only derived keys would be handed the same declining pairs
    /// on every page for the life of the library. The same rule
    /// `dims_probed_at IS NULL` states on the axis next door: a row is
    /// offered once, whatever the answer.
    ///
    /// # No mime filter here, and that is the decision
    ///
    /// A rule states one `applies_to`, so most pairs a cross join
    /// produces are pairs the rule declines, and comparing the two in SQL
    /// would drop them before the page — cheaper, and wrong twice.
    ///
    /// The first reason is that it would be a **second implementation of
    /// [`Strategy::claims`]**. `applies_to` and `material.mime` are both
    /// text a caller wrote, and the Rust comparison is over
    /// [`MimeType`] — parsed, so `IMAGE/PNG; charset=binary` and
    /// `image/png` are the one format they are. SQL `=` is not, so the
    /// walk would decline pairs the derivation claims, and a material
    /// would get a key or not depending on how its mime happened to be
    /// spelled at import.
    ///
    /// The second is that "this rule is not about this material" is an
    /// answer, and the axis keeps it: it is what tells a Strategy that is
    /// working as written from one that needs fixing
    /// ([`SeriesKey`]'s three states). Filtered out in SQL it would never
    /// be filed — and the pair would be re-offered on every pass, because
    /// what keeps a pair out of the next page is its row and nothing
    /// else.
    ///
    /// So the gate lives in [`derive`] alone, on the single door that
    /// function's doc argues for, and this walk offers.
    ///
    /// # Cursor
    ///
    /// `(asset_id, ord, strategy_id)`, the pair's identity and the order
    /// the page is returned in — a two-part cursor would skip the
    /// remaining rules of a material a page boundary cut through, the
    /// same way an `asset_id`-only cursor skips `ord > 0` in the
    /// fingerprint walk.
    ///
    /// [`JobKind::SeriesDerive`]: crate::domain::job::JobKind::SeriesDerive
    /// [`NothingToSelect`]: crate::domain::series::SeriesKey::NothingToSelect
    /// [`NotApplicable`]: crate::domain::series::SeriesKey::NotApplicable
    /// [`SeriesKey`]: crate::domain::series::SeriesKey
    /// [`Strategy::claims`]: crate::domain::series::Strategy::claims
    /// [`derive`]: crate::domain::series::derive
    async fn scan_underived(
        &self,
        after: Option<(&AssetId, u32, &StrategyId)>,
        limit: u32,
    ) -> Result<Vec<UnderivedSeries>, DomainError>;
}
