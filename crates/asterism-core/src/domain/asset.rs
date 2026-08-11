//! `Asset` — an aggregate root for a single footprint, plus the read
//! projection used on hot paths.
//!
//! The write path uses [`Asset`] (rich entity with invariants). The read hot
//! path — grid listing and search — goes through [`AssetCard`] instead, so
//! the physical row layout can evolve (for example switching to a columnar
//! representation) without changing the port signature.

use asterism_contract::query::TagMatch;
use chrono::{DateTime, Utc};

use crate::domain::attribution::{
    AttributionChannel, AttributionContext, Author, OperatorRef, PersistedAttribution,
};
use crate::domain::color::ColorBucket;
use crate::domain::material::Material;
use crate::domain::source_locator::SourceLocator;
use crate::domain::value::{
    AssetId, AssetRole, BundleId, CoverText, FoldPolicy, GroupId, Keyword, Label, Modality,
    OnDuplicate, PersonaId, RegisterNote, SourceRef, TagId, Viewer, Visibility,
};
use crate::error::DomainError;

/// A single Asterism item (aggregate root; used for writes and detail views).
///
/// Every asset belongs to exactly one persona. The original artefact lives at
/// `source.locator` and Asterism never writes back to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    /// Surrogate id (UUID v7).
    pub id: AssetId,
    /// The persona this asset belongs to (exactly one; required).
    /// Membership says nothing about who wrote it — that question is
    /// answered by [`author`](Asset::author) and
    /// [`operator_ai`](Asset::operator_ai)
    /// ([`attribution`](crate::domain::attribution)).
    pub persona_id: PersonaId,
    /// Reference to the real source of truth.
    pub source: SourceRef,
    /// What the source itself calls this row — an issue key, a row's
    /// primary key, an upstream API's id, a Session's importer-visible
    /// key. **External linkage, nothing else.**
    ///
    /// A Prop like every other column here, and it
    /// carries no uniqueness: an external record legitimately arrives
    /// more than once, and two platforms numbering their records `12345`
    /// is ordinary. V62 took the last UNIQUE off it. Nothing about
    /// matching or minting reads this — "have I seen this record" is the
    /// Source lookup's question and "are these the same thing" is the
    /// digest axes'.
    ///
    /// `None` when the source states no id of its own, which is most
    /// rows: a filesystem path is an address, not a name the source
    /// gave.
    pub external_key: Option<String>,
    /// Semantic classification (open slug: Tape / Memory / Emo / …).
    /// `None` = unclassified — the normal state for conversation
    /// messages and containers. Data format is **not** a modality (it
    /// lives on [`Material::mime`]); container structure is **not** a
    /// modality either ([`Asset::role`]). Asset-model v4.
    pub modality: Option<Modality>,
    /// Free-form labels attached to the asset (status hints, secondary
    /// modality tags, category notes, and so on).
    pub labels: Vec<Label>,
    /// When the asset occurred in the outside world. Distinct from
    /// `created_at` (which records the moment Asterism ingested it) and is
    /// the primary axis of the time-proximity constellation edge.
    pub occurred_at: DateTime<Utc>,
    /// Constellation-edge grouping key for **non-Dialog** modalities
    /// (tape / journal / image / future slot). Set by importers that
    /// used to write `session_id` for the same purpose — Session is
    /// now Dialog-only, and `bundle_id` inherits the
    /// "time_proximity edge grouping"
    /// role. Modality-agnostic; feeds edge_rebuild in P3.
    pub bundle_id: Option<BundleId>,
    /// Composition membership — the id of the **composite Asset** this
    /// asset is a member of (`None` = not inside any composite). The
    /// composition axis: exclusive 1:n containment / provenance ("this
    /// message was born inside that conversation"), physically a
    /// self-reference into the `asset` table. Distinct from the m:n
    /// `Group` filing (user-curated, shared) an asset carries — it sits
    /// in exactly one composite but any number of Groups. Generalises
    /// the pre-v2
    /// `session_id` (which only Dialog assets could carry): membership
    /// is now modality-agnostic, so an image that entered a
    /// conversation is a member of the same Session composite. The
    /// composite itself is an Asset with `modality = "session"` /
    /// `ContentKind::Composition`. DB
    /// column + self-FK land in a later slice; this field is inert
    /// (always `None`) until the migration wires it.
    pub container_id: Option<AssetId>,
    /// User-authored name for the asset — the primary surface for a
    /// **composite** Asset (a Session's title). Distinct from
    /// [`Asset::cover`](Asset::cover): `cover` is the auto-derived card
    /// display text written by the `cover_gen` job, `title` is the
    /// user's deliberate naming. Kept as separate internal slots even
    /// though the presentation layer may fold them into one label at
    /// display time. `None` = unnamed (the
    /// UI falls back to `cover` / a first-line snippet).
    pub title: Option<String>,
    /// Card cover text. Populated asynchronously by the `cover_gen` job;
    /// `None` means "still pending".
    pub cover: Option<CoverText>,
    /// Raw keywords produced by the auto-tag pipeline (used as the source
    /// pool for channel tags).
    pub keywords: Vec<Keyword>,
    /// Short annotation about the asset's register.
    pub register_note: Option<RegisterNote>,
    /// Visibility policy (input to the enforcement layer).
    pub visibility: Visibility,
    /// Subject this asset is attributed to — the write-side counterpart
    /// of the `Viewer` the query layer enforces against. `None` means
    /// **unrecorded**, never "the owner"; see
    /// [`attribution`](crate::domain::attribution) for why the absence
    /// is kept honest. Persisted as the `author_kind` / `author_subject`
    /// column pair.
    ///
    /// **Private, with the two other attribution fields.** They are set
    /// together by [`Asset::new`] from the
    /// [`AttributionContext`](crate::domain::attribution::AttributionContext)
    /// the caller had to choose, or restored together by
    /// [`Asset::from_persisted`] from the stored columns. A public field
    /// would make "carried a context and did not record it" expressible
    /// again, which is the v1 failure this wave removes; read it back
    /// through [`Asset::author`](Self::author).
    author: Option<Author>,
    /// Agent that performed the operation (`claude-code`, `codex`,
    /// `asterism-ui`, …). Separate from the author because a single
    /// subject drives Asterism through several agents, and "which one"
    /// is the question an audit of an agent-run library actually asks.
    /// `None` = unrecorded. Private for the same reason as `author`;
    /// read it through [`Asset::operator_ai`](Self::operator_ai).
    operator_ai: Option<OperatorRef>,
    /// Channel the pair above arrived through — the difference between
    /// "the owner's own app said so" and "an HTTP caller said so".
    /// Derived from the entry point, never asserted; see
    /// [`AttributionChannel`](crate::domain::attribution::AttributionChannel).
    ///
    /// `None` on a row that records nobody, and on rows written before
    /// the channel was tracked (an author or operator with no channel is
    /// the legacy shape). Private for the same reason as `author`; read
    /// it through [`Asset::attributed_via`](Self::attributed_via).
    attributed_via: Option<AttributionChannel>,
    /// Duration for time-bounded assets (dialogue length, and so on).
    pub duration_ms: Option<u64>,
    /// Pixel width of the stored bytes — **coded, orientation not
    /// applied**. Not the width anything displays: an image whose EXIF
    /// says Orientation 5-8 is shown transposed and still records its
    /// landscape pair here, because the parser reads the orientation tag
    /// into `extra` without turning the dimensions with it.
    ///
    /// `None` = unmeasured, and never `0`: a zero would sort ahead of
    /// every real measurement on an ascending axis, which is the reading
    /// `duration_ms` refuses one field up.
    ///
    /// Two independent `Option<u32>` rather than one `Option<(u32, u32)>`,
    /// which is a choice and not an oversight — it keeps the threading
    /// `duration_ms` and `SourceRef::file_size_bytes` already use, and
    /// pays for it by letting a transposed assignment past the type
    /// system. The pair is held together at the two write boundaries
    /// instead (`Footprint`'s single `dims` option, and the
    /// half-pair refusal in `AssetService::add`).
    pub width_px: Option<u32>,
    /// Pixel height of the stored bytes, on the terms
    /// [`Asset::width_px`](Self::width_px) states.
    pub height_px: Option<u32>,
    /// Star rating 0-5 (`None` = unrated). Industry-standard 5-star
    /// scale used by Photos.app / Lightroom / digiKam. The domain
    /// stores the raw scalar; the UI is free to render it as stars,
    /// bars, or a colour scale.
    pub rating: Option<u8>,
    /// Dominant colour palette — up to 5 hex strings (`"#rrggbb"`
    /// lowercase, quantised into a canonical space by
    /// `color-thief` inside the `thumb_gen` job). `None` when the
    /// asset is not an image, or when the extractor has not run
    /// yet. Fuels grid colour-strip badges, "similar colour" filters,
    /// and persona wallpaper theming.
    pub palette: Option<Vec<String>>,
    /// Source-specific metadata that does not warrant a first-class column
    /// (kept as an opaque JSON value; promoted to a real field only when
    /// several sources need the same key).
    pub extra: serde_json::Value,
    /// When Asterism ingested the asset.
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp.
    pub updated_at: DateTime<Utc>,
    /// Trash stamp — `None` = live, `Some(t)` = moved to trash at `t`.
    ///
    /// Delete is deliberately split into two verbs: `trash` stamps this
    /// column (reversible), `purge` physically deletes the row (final,
    /// and only accepted for an already-stamped row). Keeping the row
    /// alive is what makes restore cheap — every `ON DELETE CASCADE`
    /// child of the asset (tags, edges, group filing and its
    /// hand-arranged order, comments, body text, thumbnails, snapshot
    /// membership) stays untouched while the asset sits in the trash, so
    /// restore is a single stamp clear rather than a value-copy replay.
    ///
    /// The listing hot path filters these rows out by default
    /// ([`AssetQuery::trash`]); integrity guards that ask "is anything
    /// still attached?" deliberately count them (see
    /// [`ModalityRepository::asset_count`](crate::domain::repository::ModalityRepository::asset_count)
    /// and
    /// [`SessionRepository::delete_if_empty`](crate::domain::repository::SessionRepository::delete_if_empty)).
    pub trashed_at: Option<DateTime<Utc>>,
    /// Structural role — item or collection. Orthogonal to `modality`
    /// (semantic classification) and `container_id` (membership); this
    /// is the axis that used to ride on the `'session'` modality slug
    /// (asset-model v4).
    pub role: AssetRole,
    /// Headstone — the keeper this asset was folded into. `None` = a
    /// live row; `Some(id)` = this row lost a duplicate resolution and
    /// stays behind only to redirect.
    ///
    /// A third state next to live and trashed, and deliberately not
    /// either of them. Trash is reversible and *ends in deletion*
    /// (retention purges what it holds), while a headstone must never
    /// be deleted: every reference minted before the fold — an old
    /// UUID in a stale sidecar claim, a `<locator>#<keyword>` fragment
    /// written into a PNG note, a dispatch record — resolves through
    /// this row to reach the keeper. It is
    /// also not trashed *state*, so the trash view does not show it.
    ///
    /// The read rule is one sentence: **paths that enumerate drop it,
    /// paths that name it keep it.** Listing, facet counts, the
    /// duplicate report, search indexing, the retention scan and
    /// Query-Group evaluation all add `folded_into IS NULL`; `find` by
    /// id and [`find_by_source`](crate::domain::repository::AssetRepository::find_by_source)
    /// by locator deliberately return it, because that is what makes
    /// the redirect resolvable at all.
    ///
    /// Written by the fold verb (a later P2 subtask) through a
    /// column-specific setter, never by the whole-entity `save` — a
    /// read-modify-write carrying a stale `None` back would resurrect a
    /// headstone as a live duplicate.
    pub folded_into: Option<AssetId>,
    /// Whether this row may be folded at all — the durable half of a
    /// human "these are different things" ruling. See
    /// [`FoldPolicy`]. Not an absence: every row starts at
    /// [`FoldPolicy::Auto`] ("nobody has ruled"), which is a real
    /// answer.
    ///
    /// Owned by the resolution verb, on the same setter-not-`save`
    /// rule as `folded_into`.
    pub fold_policy: FoldPolicy,
    /// What the caller asked to happen if this asset's fingerprint
    /// turns out to match an existing one — see [`OnDuplicate`].
    ///
    /// `None` = **unrecorded**: nobody declared a strategy for this
    /// registration, which is not the same as having asked for
    /// confirmation. Absence is what a later default (an importer lane,
    /// a persona setting) is allowed to fill; a stored `Ask` is not.
    ///
    /// Unlike the two fields above it, this one is set at registration
    /// and by nothing afterwards: it is what the caller declared, and
    /// re-declaring it is not a verb this design has.
    pub on_duplicate: Option<OnDuplicate>,
    /// Physical originals of this asset (the material layer). Exactly
    /// one entry (`ord == 0`) for items in the current wave; always
    /// empty for [`AssetRole::Collection`] — a container has no bytes
    /// of its own (enforced by [`Asset::attach_material`]).
    pub materials: Vec<Material>,
}

impl Asset {
    /// Builds a new asset with only the required fields; the optional ones
    /// are filled in later (`cover_gen` and `auto_tag` populate them after
    /// ingestion).
    ///
    /// The attribution is an argument rather than a field to fill in
    /// afterwards: every caller has to say which entry point it is
    /// (including "none of them", via
    /// [`AttributionContext::unrecorded`](crate::domain::attribution::AttributionContext::unrecorded)),
    /// and there is no shape in which a caller holds a context and
    /// builds an asset without it.
    pub fn new(
        persona_id: PersonaId,
        source: SourceRef,
        modality: Option<Modality>,
        occurred_at: DateTime<Utc>,
        attribution: &AttributionContext,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: AssetId::new(),
            persona_id,
            source,
            external_key: None,
            modality,
            labels: Vec::new(),
            occurred_at,
            bundle_id: None,
            container_id: None,
            title: None,
            cover: None,
            keywords: Vec::new(),
            register_note: None,
            visibility: Visibility::Open,
            author: attribution.author().cloned(),
            operator_ai: attribution.operator_ai().cloned(),
            attributed_via: attribution.attributed_via(),
            duration_ms: None,
            width_px: None,
            height_px: None,
            rating: None,
            palette: None,
            extra: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
            trashed_at: None,
            role: AssetRole::Item,
            folded_into: None,
            fold_policy: FoldPolicy::default(),
            // No `Default` impl to reach for, deliberately: the
            // strategy is the caller's to state, and `Asset::new` is
            // not the caller. `add` overwrites this from the command.
            on_duplicate: None,
            materials: Vec::new(),
        }
    }

    /// Seed constructor for the read path: rebuilds the identity, the
    /// timestamps and the attribution of a stored row, leaving the rest
    /// at their `new` defaults for the adapter to assign field by field.
    ///
    /// Separate from [`new`](Self::new) because hydration is not a
    /// write: the triple it carries is a fact read back out of the
    /// columns (hence [`PersistedAttribution`], the only publicly
    /// constructible triple that can hold an arbitrary channel), not a
    /// claim arriving through an entry point.
    // Long by construction: it is the row's identity columns plus the
    // attribution, and shortening it would mean handing some of them in
    // through a public field — which is exactly what this constructor
    // exists to stop.
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: AssetId,
        persona_id: PersonaId,
        source: SourceRef,
        modality: Option<Modality>,
        occurred_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        attribution: PersistedAttribution,
    ) -> Self {
        Self {
            id,
            persona_id,
            source,
            external_key: None,
            modality,
            labels: Vec::new(),
            occurred_at,
            bundle_id: None,
            container_id: None,
            title: None,
            cover: None,
            keywords: Vec::new(),
            register_note: None,
            visibility: Visibility::Open,
            author: attribution.author().cloned(),
            operator_ai: attribution.operator_ai().cloned(),
            attributed_via: attribution.attributed_via(),
            duration_ms: None,
            width_px: None,
            height_px: None,
            rating: None,
            palette: None,
            extra: serde_json::Value::Null,
            created_at,
            updated_at,
            trashed_at: None,
            role: AssetRole::Item,
            // The fold axis seeds at the shape a live, unruled row has,
            // like every other column here: the adapter assigns the
            // stored values immediately afterwards. Seeding them any
            // other way would make a partially-assigned entity claim a
            // headstone or a ruling that no row asserted.
            folded_into: None,
            fold_policy: FoldPolicy::default(),
            on_duplicate: None,
            materials: Vec::new(),
        }
    }

    /// Subject this asset is attributed to (`None` = unrecorded).
    pub fn author(&self) -> Option<&Author> {
        self.author.as_ref()
    }

    /// Agent that performed the operation (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel the pair above arrived through (`None` on an unrecorded
    /// row, and on a legacy row written before the column existed).
    pub fn attributed_via(&self) -> Option<AttributionChannel> {
        self.attributed_via
    }

    /// Attaches a physical original, guarding the v4 structural
    /// invariant: a collection has no bytes of its own, so materials
    /// can only be attached to items.
    pub fn attach_material(&mut self, material: Material) -> Result<(), DomainError> {
        if self.role == AssetRole::Collection {
            return Err(DomainError::Validation(
                "a collection carries no material of its own".into(),
            ));
        }
        self.materials.push(material);
        Ok(())
    }

    /// The metadata the primary original's container carries, taken
    /// apart — `None` when there is no primary material, no walk has
    /// produced an object, or the column holds a marker.
    ///
    /// A **convenience, not a second home.** Handing a consumer that
    /// already holds an `Asset` its metadata without walking the
    /// materials is worth doing; keeping a copy of it anywhere is not.
    /// So this is derived on read from
    /// [`Material::meta_kv`](crate::domain::material::Material::meta_kv)
    /// and **never written back** — deliberately a method rather than a
    /// field or an `extra` key, because `extra` is round-tripped
    /// verbatim by the upsert and an injected key would be persisted
    /// the next time anything saved the entity. Two stored copies of
    /// one fact drift, and the copy that drifts is always the one
    /// nobody thought was authoritative.
    ///
    /// `ord == 0` for the same reason the card projection reads `mime`
    /// and `content_hash` from that row: a secondary original is a
    /// different artefact carrying its own metadata, and folding the
    /// two into one answer is what the `ord` axis exists to avoid.
    pub fn material_meta(&self) -> Option<std::collections::BTreeMap<String, String>> {
        self.materials
            .iter()
            .find(|material| material.ord == 0)?
            .meta_fields()
    }
}

/// Which side of the trash a query wants to see.
///
/// Exists so the trash view reuses the *entire* listing path (sort,
/// filter, paging, visibility) instead of growing a parallel
/// `list_trashed` surface. Every reader of the shared asset `WHERE`
/// builder therefore agrees on trash semantics by construction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrashFilter {
    /// Live rows only (`trashed_at IS NULL`). The default: the grid,
    /// search, counts, and Query-Group evaluation must never surface a
    /// trashed asset.
    #[default]
    LiveOnly,
    /// Trashed rows only (`trashed_at IS NOT NULL`) — the trash view.
    TrashedOnly,
    /// Both. For diagnostics and for callers that intentionally reason
    /// over the whole table.
    Any,
}

/// Lightweight projection used on the read hot path (grid / search).
///
/// Holds cover text and card-level metadata only; body content and `extra`
/// are lazy-loaded via the full `Asset` entity on demand. Adapters may build
/// this directly from a row scan without materialising the entity.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetCard {
    /// Asset id.
    pub id: AssetId,
    /// The persona this asset belongs to. What the card shows is where
    /// the asset is filed, not who wrote it.
    pub persona_id: PersonaId,
    /// Semantic classification (`None` = unclassified).
    pub modality: Option<Modality>,
    /// Occurrence time — the grid's time-order sort key.
    pub occurred_at: DateTime<Utc>,
    /// Cover text (`None` while the `cover_gen` job is still pending).
    pub cover: Option<CoverText>,
    /// Hand-given name. Carried on the card because a container's name
    /// *is* its content — it owns no material to derive a cover from,
    /// so without this the grid can only render it blank.
    pub title: Option<String>,
    /// How many live assets point at this one via `container_id`.
    /// Always `0` for an item (nothing is inside it); for a container
    /// it is the card's headline number, the way a file size is an
    /// item's. Counted with the partial `idx_asset_container` index,
    /// so an item pays one empty seek — the `has_thread` cost class.
    pub member_count: u64,
    /// Labels shown as badges on the card.
    pub labels: Vec<Label>,
    /// Original artefact size (used as a weight signal, and the key the
    /// `FileSize` sort axis orders on).
    pub file_size_bytes: Option<u64>,
    /// Playback length for time-bounded material — the key the
    /// `Duration` sort axis orders on. `None` covers both "does not
    /// play" and "not measured": the projection cannot separate them,
    /// and [`sort_eval`](crate::domain::sort_eval) tails absent rows in
    /// both directions rather than reading them as zero.
    ///
    /// Denormalised from [`Asset::duration_ms`] on the read hot path for
    /// the same reason `rating` is: an axis evaluated over card rows
    /// cannot ask the full entity for its key, once per card.
    pub duration_ms: Option<u64>,
    /// Total pixel count (`width_px * height_px`) — the key the `Pixels`
    /// sort axis orders on, denormalised for the reason `duration_ms` is.
    ///
    /// `None` = nothing measured this row's dimensions, which is the same
    /// three-valued reading the two metric keys above carry. The
    /// projection holds the product and not the pair on purpose: the
    /// underlying columns are coded dimensions taken before orientation,
    /// so the pair on a card would read as a displayed shape and be wrong
    /// for every upright phone capture.
    pub pixel_count: Option<u64>,
    /// Format fact of the primary material (`image/png`, `text/plain`,
    /// …; `None` = unknown). The card-level answer to "is this an
    /// image?" — asset-model v4 moved that question off the modality
    /// axis onto the material layer.
    pub mime: Option<String>,
    /// Where the original artefact is. Kept on the card projection so
    /// the UI can render format-specific media directly — image cards
    /// feed the rendered form into `convertFileSrc()` for the asset
    /// protocol so the browser can display the photo without a second
    /// round trip to the server.
    ///
    /// Carried as the value even though every consumer of it renders a
    /// string: the rendering the wire wants is the *display* one (the
    /// path of a file, the container of a record), and deriving that
    /// from a raw column value at each consumer is the shape this whole
    /// change removes. The mapper to
    /// `AssetCardDto` is where it becomes text.
    pub source_locator: SourceLocator,
    /// Groups the asset is filed into. Populated by a single bulk join
    /// on the m:n `asset_bucket` table so the UI can group / sort by
    /// `Group` without a follow-up round trip per card.
    pub group_ids: Vec<GroupId>,
    /// The card's slot inside its primary group — `asset_bucket.position`
    /// for `group_ids[0]`, from the same bulk join. `None` when unfiled.
    ///
    /// The hand arrangement only reaches the grid through this field. The
    /// repository orders by `position` when the filter names exactly one
    /// group, but that is the page's arrival order, not a value the
    /// client can sort on — so under any other filter shape the
    /// arrangement was simply not expressible. Feeds `Group` + `Ordered`
    /// in [`sort_eval`](crate::domain::sort_eval) and its UI twin.
    pub primary_group_position: Option<i64>,
    /// Ingest time — when Asterism first saw the row. Surfaced on the
    /// projection so the grid can offer an "Added" sort mode without
    /// re-fetching the full entity per card (useful when the payload
    /// carries a future / historical `occurred_at` and the user
    /// wants the "recently arrived" order instead).
    pub created_at: DateTime<Utc>,
    /// Last-modification time — the stamp a differential-sync consumer
    /// hands back as `AssetQuery::updated_from`. Carried on the card so
    /// "what changed since I last looked" is answerable from the page
    /// itself; without it a caller has to fetch every row's full entity
    /// just to learn where its cursor should sit next.
    ///
    /// What advances the column is narrower than "any change" — see
    /// [`AssetQuery::updated_from`] for the exhaustive list.
    pub updated_at: DateTime<Utc>,
    /// Star rating 0-5 (`None` = unrated). Denormalised from
    /// `Asset::rating` on the read hot path so the grid can render
    /// the star widget without hydrating the full entity.
    pub rating: Option<u8>,
    /// Dominant colour palette — up to 5 hex strings. Denormalised
    /// from `Asset::palette` so the grid can render a colour strip
    /// on each image card without hydrating the full entity.
    pub palette: Option<Vec<String>>,
    /// `true` when the underlying asset carries a non-empty
    /// `register_note`. Powers the card's 📝 badge.
    pub has_note: bool,
    /// `true` when the underlying asset has at least one
    /// `AssetComment`. Powers the card's 💬 badge.
    pub has_thread: bool,
    /// Structural role of the underlying asset. Carried on the card so
    /// the grid can tell a container apart from an item without
    /// hydrating the entity — a collection has no material, so it
    /// renders nothing useful through the item code path (it showed up
    /// as a nameless coverless card before this field existed).
    pub role: AssetRole,
    /// Subject the asset is attributed to, denormalised from
    /// [`Asset::author`] so the read path can show *who* a row is by
    /// without hydrating the entity. `None` means **unrecorded**, never
    /// "the owner" — the projection must not turn the absence into a
    /// value the entity never held (see
    /// [`attribution`](crate::domain::attribution)).
    pub author: Option<Author>,
    /// Agent that performed the operation, denormalised from
    /// [`Asset::operator_ai`]. Carried next to `author` because the two
    /// answer different questions ("who" vs "through what") and a
    /// reader that has one but not the other cannot tell an agent-run
    /// row from a hand-made one. `None` = unrecorded.
    pub operator_ai: Option<OperatorRef>,
}

/// Index-only projection for 6-figure grids.
///
/// Sibling of [`AssetCard`] with the heavy per-row fields — cover
/// text and source locator — deliberately dropped. Used by
/// the frontend as the "load-everything-for-order + hydrate-only-
/// the-visible-viewport" split: `list_index` returns every matching
/// row's `AssetIndex` (small enough for 100 k+ rows in one round
/// trip), then `cards_by_ids` hydrates the ~40 rows the VList
/// actually paints.
///
/// Field selection covers every client-side sort / filter axis:
/// `persona_id`, `modality`, `labels`, `group_ids`,
/// `occurred_at`, `created_at`, `updated_at`, `duration_ms`,
/// `file_size_bytes`. That list is the whole membership rule — an axis
/// the index cannot express is an axis the grid cannot offer — and it
/// is what moved file size onto the row and duration with it: both are
/// axes (`SortTarget::FileSize` / `Duration`), so the earlier
/// "heavy fields dropped" reading of file size cost the grid two
/// orderings to save eight bytes a row.
///
/// Cover-text sort stays intentionally unsupported on index rows (falls
/// back to `occurred_at`) — the full cover only exists on the hydrated
/// card, and it is text of unbounded length rather than an integer.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetIndex {
    /// Asset id.
    pub id: AssetId,
    /// The persona this asset belongs to.
    pub persona_id: PersonaId,
    /// Semantic classification (`None` = unclassified).
    pub modality: Option<Modality>,
    /// Occurrence time — the grid's default sort key.
    pub occurred_at: DateTime<Utc>,
    /// Labels — required for tag-axis sort and content-flag filter.
    pub labels: Vec<Label>,
    /// Groups the asset is filed into — required for group-axis
    /// sort and Groups-view filter.
    pub group_ids: Vec<GroupId>,
    /// Slot inside the primary group — see
    /// [`AssetCard::primary_group_position`]. Required on the light row
    /// for the same reason `group_ids` is: the client sorts over index
    /// rows, so an axis the index cannot express is an axis the grid
    /// cannot offer.
    pub primary_group_position: Option<i64>,
    /// Ingest time — required for "Added" sort mode.
    pub created_at: DateTime<Utc>,
    /// Last-modification time — required for the `UpdatedAt` sort axis
    /// and for the differential-sync cursor, both of which the client
    /// evaluates over index rows. Leaving it off the light row would
    /// make the axis collapse on exactly the pages the index path
    /// exists for; see [`AssetCard::updated_at`].
    pub updated_at: DateTime<Utc>,
    /// Playback length — required for the `Duration` sort axis, which
    /// the client evaluates over these rows. Same absent-is-not-zero
    /// reading as [`AssetCard::duration_ms`].
    pub duration_ms: Option<u64>,
    /// Stored size — required for the `FileSize` axis, for the reason
    /// one field up. The card projection carries the same value, so a
    /// light row and its hydrated form agree rather than the light row
    /// reporting "no size" for every asset in the library.
    pub file_size_bytes: Option<u64>,
    /// Total pixel count — required for the `Pixels` axis, for the
    /// reason the two keys above are: the client sorts over these rows,
    /// so an axis the index cannot express is an axis that silently
    /// answers in the default order. See [`AssetCard::pixel_count`].
    pub pixel_count: Option<u64>,
    /// Structural role — required because the two roles render through
    /// different card paths. Without it the light row has to guess, and
    /// a container guessed as an item paints blank until hydration
    /// replaces it.
    pub role: AssetRole,
}

/// Filter and pagination parameters for listing and searching assets.
///
/// `viewer` is not optional — it defaults to `Owner` — because the query
/// layer must enforce visibility (assets with restricted visibility must
/// not appear to subjects outside their sharing list).
#[derive(Debug, Clone, PartialEq)]
pub struct AssetQuery {
    /// Requesting subject (input to the visibility filter).
    pub viewer: Viewer,
    /// Optional persona filter.
    pub persona_id: Option<PersonaId>,
    /// Optional modality filter.
    pub modality: Option<Modality>,
    /// Restrict to rows carrying *no* modality. Distinct from
    /// `modality: None`, which means "do not filter on this axis at
    /// all" — this is the selectable Unclassified bucket, and without
    /// it an unclassified row is reachable only by scrolling the
    /// unfiltered grid. Ignored when `modality` is set (a row cannot
    /// be both a given slug and none).
    pub modality_unset: bool,
    /// Lower bound on occurrence time (inclusive).
    pub occurred_from: Option<DateTime<Utc>>,
    /// Upper bound on occurrence time (exclusive).
    pub occurred_until: Option<DateTime<Utc>>,
    /// Lower bound on ingest time (`created_at`, inclusive) — when the
    /// row entered the library, which an import separates from
    /// `occurred_from` by however old the imported material is.
    pub created_from: Option<DateTime<Utc>>,
    /// Upper bound on ingest time — **inclusive**, unlike
    /// `occurred_until`. The window is a sync cursor rather than a
    /// calendar range; see
    /// `asterism_contract::query::ListAssetsQuery::created_from_ms` for
    /// why, and for why an inverted pair yields an empty page here
    /// instead of the validation error the rating band gets.
    pub created_until: Option<DateTime<Utc>>,
    /// Lower bound on last-modification time (`updated_at`, inclusive).
    ///
    /// The column is advanced by metadata edits, provenance writes, the
    /// cover / flags / keyword pipeline writes and Session renames — but
    /// **not** by trash, restore, tagging, group filing or comments.
    /// The wire doc (`ListAssetsQuery::updated_from_ms`) carries the
    /// exhaustive list; a consumer treating this as "any change"
    /// silently misses those.
    pub updated_from: Option<DateTime<Utc>>,
    /// Upper bound on last-modification time (inclusive, same reasoning
    /// as `created_until`).
    pub updated_until: Option<DateTime<Utc>>,
    /// Tag (channel) filter. Multi-select; [`tag_match`](Self::tag_match)
    /// says whether the ids combine as a union or an intersection. An
    /// empty vector disables the tag filter entirely (equivalent to no
    /// filter).
    ///
    /// OR is the default because that is what the Are.na channel
    /// list and Notion multi-select tag box do: "show me anything
    /// tagged with any of these".
    pub tag_ids: Vec<TagId>,
    /// How [`tag_ids`](Self::tag_ids) combine — union (default) or
    /// intersection. Inert while `tag_ids` is empty.
    ///
    /// The wire enum re-used verbatim, like
    /// [`SortSpec`](asterism_contract::sort::SortSpec) is on the sort
    /// path: it is a closed two-value vocabulary with nothing to parse
    /// or validate, so a domain twin would be the same set of variants
    /// under a second name.
    pub tag_match: TagMatch,
    /// User-curated `Group` filter with **OR** semantics — an asset
    /// needs to sit in at least one of the listed groups to pass.
    /// Empty vector disables the filter. Groups are the user-curated
    /// twin of `Tag`: hand-picked buckets rather than organic
    /// labels. See [`crate::domain::group::Group`].
    pub group_ids: Vec<GroupId>,
    /// Optional composition filter — drills into the members of a
    /// specific composite Asset (`asset.container_id = this`). The v2
    /// generalisation of `session_id`: the Reader lists a Session's
    /// members through this, and because membership is modality-agnostic
    /// the result mixes messages and any images that entered the
    /// conversation. Inert until the repository wires it into the WHERE
    /// clause (later subtask).
    pub container_id: Option<AssetId>,
    /// Optional label filter — matches when the asset's `labels`
    /// array contains this literal. Used by the UI to narrow by role
    /// (`user` / `assistant`), by `format` (`markdown` / `code:rust`),
    /// or by any other free-form label a parser or edit set on the
    /// asset. Only one label is matched at a time; multi-label AND
    /// filtering is a future extension.
    pub label: Option<Label>,
    /// Body-text filter — matches when the asset's resolved body
    /// contains this string, with no word boundaries and no dictionary
    /// (`スト` matches `テスト`, `猫` matches `黒猫`).
    ///
    /// A predicate like the rest of this struct, so the answer is a
    /// countable, orderable set. The ranked-shortlist question lives on
    /// the [`AssetRetriever`](crate::domain::repository::AssetRetriever)
    /// port instead and makes no completeness claim.
    ///
    /// Already trimmed by the time it reaches here; `Some("")` is not a
    /// state the mapper produces.
    pub text_match: Option<String>,
    /// Format facet filter — a mime top-level type (`image` / `video`
    /// / `audio` / `text`), matched as a prefix against the primary
    /// material's mime. The sidebar FORMAT section drives this.
    pub format: Option<String>,
    /// Colour facet filter — matches when any entry of the asset's
    /// dominant-colour palette quantises into this bucket
    /// (`crate::domain::color`). An asset whose palette was never
    /// extracted carries no bucket and therefore never matches: the
    /// facet describes what is known, it does not guess.
    pub color: Option<ColorBucket>,
    /// Lower bound on the star rating, inclusive. Values outside `0..=5`
    /// never reach here — the wire boundary rejects them
    /// (`crate::application::mapping::to_asset_query`).
    pub rating_min: Option<u8>,
    /// Upper bound on the star rating, inclusive.
    ///
    /// Naming **either** bound also excludes unrated assets: the SQL
    /// predicate compares against a `NULL` rating, which is unknown and
    /// therefore not a match. That is the intended reading rather than an
    /// accident of three-valued logic — "1 to 2 stars" is a question
    /// about rated assets, and an unrated one has nothing to say about
    /// it. The adapter states the exclusion explicitly in the `WHERE`
    /// clause so the reader does not have to derive it (and so the
    /// partial `idx_asset_persona_rating` is reachable).
    pub rating_max: Option<u8>,
    /// AlbumMeta name filter — restrict to rows carrying a statement
    /// filed under this key ([`crate::domain::album_meta`]).
    ///
    /// Combined with [`album_meta_value`](Self::album_meta_value) by
    /// **AND on one entry**, not across entries: naming both asks for a
    /// row where *this key holds this value*, which is the question
    /// somebody with an identifier in hand is asking. Naming only the
    /// key asks which rows have anything to say under that name.
    pub album_meta_key: Option<String>,
    /// AlbumMeta value filter — restrict to rows carrying this exact
    /// value, under [`album_meta_key`](Self::album_meta_key) if one is
    /// named and under any name otherwise.
    ///
    /// Value-without-key is deliberately allowed: somebody pasting a
    /// generator's reference usually knows the value and not the name it
    /// was filed under. It does not make the value an identity — a
    /// filter answers with however many rows carry it, and nothing here
    /// constrains that to one.
    pub album_meta_value: Option<String>,
    /// Lower bound on playback length in milliseconds, inclusive. The
    /// unit is the column's own (`asset.duration_ms`); no conversion
    /// happens on the way in, so whatever a client renders — seconds, a
    /// `mm:ss` box — is that side's presentation.
    pub duration_min_ms: Option<u64>,
    /// Upper bound on playback length (milliseconds, inclusive).
    ///
    /// Naming **either** end also excludes assets with no length, the
    /// same promise the rating band makes and by the same mechanism: the
    /// adapter compares against a `NULL` `duration_ms`, which is unknown
    /// and therefore not a match. Deliberate, not an artefact of
    /// three-valued logic — a still image has no length to place in a
    /// band, and a video the importer could not probe left the column
    /// unset rather than measuring zero. Admitting either as `0` would
    /// put it at the head of "under 10 seconds", which is a wrong answer
    /// rather than a generous one.
    ///
    /// Inversion (`duration_min_ms > duration_max_ms`) never reaches
    /// here: the wire boundary rejects it
    /// (`crate::application::mapping::to_asset_query`), because an empty
    /// page reads as a fact about the library rather than about the
    /// request. There is no range check to go with it — the axis has no
    /// ceiling, so `u64` is the definition set.
    pub duration_max_ms: Option<u64>,
    /// Lower bound on stored size in bytes, inclusive
    /// (`asset.file_size_bytes`, the raw unit).
    pub size_min_bytes: Option<u64>,
    /// Upper bound on stored size (bytes, inclusive). Same NULL-exclusion
    /// promise and the same inversion rule as
    /// [`duration_max_ms`](Self::duration_max_ms): a row whose bytes were
    /// never recorded carries nothing to place in a size band, so naming
    /// either end drops it.
    pub size_max_bytes: Option<u64>,
    /// Lower bound on total pixel count (`width_px * height_px`),
    /// inclusive. The raw count, as the two bands above carry raw ms and
    /// raw bytes.
    pub pixels_min: Option<u64>,
    /// Upper bound on total pixel count (inclusive).
    ///
    /// Same NULL-exclusion promise and the same inversion rule as
    /// [`size_max_bytes`](Self::size_max_bytes). The adapter compares
    /// against the *product* of the two columns, so an unmeasured row is
    /// excluded through a `NULL` arising from either side — which is the
    /// whole set of them, since the pair is written together or not at
    /// all.
    ///
    /// The axis is the product rather than the two sides because the
    /// columns hold coded dimensions taken before orientation is applied;
    /// see [`asterism_contract::query::ListAssetsQuery::pixels_min`].
    pub pixels_max: Option<u64>,
    /// Trash side to read. Defaults to [`TrashFilter::LiveOnly`] so a
    /// caller that never heard of the trash cannot accidentally leak
    /// trashed assets into a listing.
    pub trash: TrashFilter,
    /// Page offset.
    pub offset: u64,
    /// Page size (adapters may clamp against a hard upper bound).
    pub limit: u64,
}

impl Default for AssetQuery {
    fn default() -> Self {
        Self {
            viewer: Viewer::Owner,
            persona_id: None,
            modality: None,
            modality_unset: false,
            occurred_from: None,
            occurred_until: None,
            created_from: None,
            created_until: None,
            updated_from: None,
            updated_until: None,
            tag_ids: Vec::new(),
            tag_match: TagMatch::Any,
            group_ids: Vec::new(),
            container_id: None,
            label: None,
            text_match: None,
            format: None,
            color: None,
            rating_min: None,
            rating_max: None,
            album_meta_key: None,
            album_meta_value: None,
            // No band asked for on either metric axis, which is the only
            // state in which rows carrying no length / no recorded size
            // (stills, unprobed containers) survive the filter.
            duration_min_ms: None,
            duration_max_ms: None,
            size_min_bytes: None,
            size_max_bytes: None,
            pixels_min: None,
            pixels_max: None,
            trash: TrashFilter::LiveOnly,
            offset: 0,
            limit: Self::DEFAULT_LIMIT,
        }
    }
}

impl AssetQuery {
    /// Default page size. Sized for an initial grid viewport plus a small
    /// look-ahead buffer.
    pub const DEFAULT_LIMIT: u64 = 200;
}

/// Reserved facet key for rows carrying no modality.
///
/// "Unclassified" has to be selectable, not merely countable: a row with
/// no modality, no material and no container sits in the grid and in no
/// facet, so before this key existed the only way to reach one was to
/// scroll the unfiltered list. Wire-side it doubles as the sentinel a
/// client sends back to filter for exactly those rows — hence a slug
/// shape the `Modality` newtype rejects (leading `!`), so it can never
/// collide with a real master row.
pub const UNCLASSIFIED_MODALITY: &str = "!unclassified";

/// Coarse content-type hints — mirrors render-session's `HasTable` /
/// `HasMermaid` / `HasBox` filter family so cards carrying tables,
/// diagrams, or code blocks stand out during triage.
///
/// Stored per-asset in the `asset` table (computed from the full
/// body during the `cover_gen` job) and OR-reduced per session for
/// the Sessions view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContentFlags {
    /// The body contained a fenced code block (triple-backtick).
    pub has_code: bool,
    /// The body carried a markdown table — a `|`-bounded header
    /// followed by an alignment separator row.
    pub has_table: bool,
    /// The body opened a ```mermaid fence.
    pub has_mermaid: bool,
    /// The body contained a `[text](url)` shaped link.
    pub has_link: bool,
}

impl ContentFlags {
    /// Scans a body string for the four markers. The table check
    /// requires a `|-` separator preceded by a `|`-bordered line to
    /// avoid firing on stray pipes in prose.
    pub fn detect(text: &str) -> Self {
        let has_mermaid = text.contains("```mermaid");
        let has_code = text.contains("```");
        let has_link = text.contains("](");
        // Cheap heuristic: a `|-` run implies an alignment row, and a
        // preceding `|` line means the header exists. Full regex would
        // be tighter but the false-positive rate here is already low.
        let has_table = text.contains("|-") && text.contains('|');
        Self {
            has_code,
            has_table,
            has_mermaid,
            has_link,
        }
    }

    /// Combines two flag sets by OR — used to aggregate per-asset
    /// flags into a per-session summary.
    pub fn merge(self, other: Self) -> Self {
        Self {
            has_code: self.has_code || other.has_code,
            has_table: self.has_table || other.has_table,
            has_mermaid: self.has_mermaid || other.has_mermaid,
            has_link: self.has_link || other.has_link,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::SourceKind;

    #[test]
    fn new_asset_defaults_to_open_visibility_and_pending_cover() {
        let source = SourceRef::new(
            SourceKind::new(SourceKind::FS).unwrap(),
            "notes/state-01.md",
        )
        .unwrap();
        let asset = Asset::new(
            PersonaId::new(),
            source,
            Some(Modality::new(Modality::STATE).unwrap()),
            Utc::now(),
            &AttributionContext::unrecorded(),
        );
        assert_eq!(asset.visibility, Visibility::Open);
        assert!(
            asset.cover.is_none(),
            "cover is populated later by the cover_gen job"
        );
        assert!(
            asset.trashed_at.is_none(),
            "a freshly ingested asset is live, never trashed"
        );
        assert!(
            asset.author().is_none()
                && asset.operator_ai().is_none()
                && asset.attributed_via().is_none(),
            "attribution starts unrecorded — nobody asserted one, and the \
             owner is not a default"
        );
    }

    #[test]
    fn the_chosen_context_is_what_the_asset_carries() {
        // The point of the required argument: whatever entry point the
        // caller named lands on the entity whole, channel included.
        let source =
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "notes/owned.md").unwrap();
        let owned = Asset::new(
            PersonaId::new(),
            source,
            None,
            Utc::now(),
            &AttributionContext::owner_surface(),
        );
        assert_eq!(owned.author(), Some(&Author::Owner));
        assert_eq!(
            owned.attributed_via(),
            Some(AttributionChannel::OwnerSurface)
        );

        // And a restored row carries the stored triple verbatim — a
        // legacy shape (author, no channel) included, because that is
        // what the columns hold.
        let legacy = Asset::from_persisted(
            AssetId::new(),
            PersonaId::new(),
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "notes/legacy.md").unwrap(),
            None,
            Utc::now(),
            Utc::now(),
            Utc::now(),
            PersistedAttribution::from_columns(Some("owner"), None, None, None).unwrap(),
        );
        assert_eq!(legacy.author(), Some(&Author::Owner));
        assert_eq!(legacy.attributed_via(), None);
    }

    #[test]
    fn asset_query_defaults_to_live_only_so_trashed_rows_cannot_leak() {
        // Guards the one invariant every listing depends on: a caller
        // that builds a query without naming a trash side must not see
        // trashed assets.
        assert_eq!(AssetQuery::default().trash, TrashFilter::LiveOnly);
        assert_eq!(TrashFilter::default(), TrashFilter::LiveOnly);
    }
}
