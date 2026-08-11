//! Convenience shape for parser output plus the mapping to the wire
//! `AddAssetCommand`.

use asterism_contract::command::AddAssetCommand;
use chrono::{DateTime, Utc};

/// What a parser hands back to the pipeline.
///
/// The persona binding and the ingest source stay orthogonal — the
/// parser attributes the item to a modality and (when known) a session
/// / labels / metadata; the persona id comes from the importer's CLI
/// flag and is set by [`spec_to_command`].
#[derive(Debug, Clone)]
pub struct AssetSpec {
    /// Source kind slug (typically the same as the scanner's).
    pub source_kind: String,
    /// Source-side locator (usually the scanner's `locator`).
    pub locator: String,
    /// Semantic classification slug (`state`, `tape`, …). `None` =
    /// unclassified (asset-model v4) — the right value for
    /// conversation messages (structure rides on
    /// `external_session_key`) and for raw media (format rides on the
    /// server-side material layer, derived from the locator).
    pub modality: Option<String>,
    /// Occurrence time.
    pub occurred_at: DateTime<Utc>,
    /// Session.id UUID direct — reserved for callers that already
    /// hold a materialised `Session.id`. Importers cannot know the
    /// server-side surrogate id, so they always leave this `None`
    /// and hand the raw session key through `external_session_key`
    /// instead. Mutually exclusive with `external_session_key`.
    pub session_id: Option<String>,
    /// External session identifier the importer supplies (Claude Code
    /// session UUID, JSONL file stem, …). The server resolves each
    /// unique `(persona_id, external_session_key)` pair to a
    /// `Session.id` via `SessionService::find_or_create_by_external_key`,
    /// so re-imports collapse onto the same Session row. Membership is
    /// modality-agnostic (asset-model v4).
    pub external_session_key: Option<String>,
    /// What the source calls this record — carried from
    /// [`FootprintSource::external_id`](crate::footprint::FootprintSource::external_id)
    /// and landing on `asset.external_key`. External linkage only:
    /// nothing about matching or minting reads it, and it carries no
    /// uniqueness (a source states one key for a record that arrives
    /// twice, and two platforms number a record alike).
    pub external_key: Option<String>,
    /// Constellation-edge grouping key for assets that are not members
    /// of a Session container (tape / journal / image / future slot).
    pub bundle_id: Option<String>,
    /// Free-form labels.
    pub labels: Vec<String>,
    /// Register / tone annotation.
    pub register_note: Option<String>,
    /// Originating platform name.
    pub platform: Option<String>,
    /// Original artefact size.
    pub file_size_bytes: Option<u64>,
    /// Duration for time-bounded assets.
    pub duration_ms: Option<u64>,
    /// Pixel width of the bytes the parser read — the **coded**
    /// dimension, with no orientation applied.
    ///
    /// This is what a parser has: the image side takes EXIF dims or the
    /// decoded header and reads the orientation tag into `extra_json`
    /// separately, so an Orientation 5-8 photo carries its landscape pair
    /// here and displays transposed. The video side carries container
    /// pixel dimensions with no rotation or pixel aspect beside them.
    ///
    /// `None` = the parser could not measure it, never `0`.
    ///
    /// **The two halves are filled together.** Every
    /// [`Footprint`](crate::footprint::Footprint) variant that has
    /// dimensions holds them as one `Option<(u32, u32)>`, so
    /// `into_asset_spec` writes both or neither; a spec built by hand can
    /// break that, and the server refuses the half when it arrives.
    pub width_px: Option<u32>,
    /// Pixel height of those same bytes, on the terms
    /// [`AssetSpec::width_px`] states.
    pub height_px: Option<u32>,
    /// Source-specific extension bag, serialised as a JSON string.
    pub extra_json: Option<String>,
    /// Optional display text the parser already knows. When set, the
    /// server writes it verbatim as the asset cover and skips the
    /// server-side `cover_gen` heuristic.
    pub cover_hint: Option<String>,
    /// Declared origin of this artefact (`asset:<uuid>` /
    /// `dispatch:<uuid>` / `sidecar`).
    ///
    /// Scanning a source cannot tell you that a file came back from an
    /// outside generator — but the *file itself* sometimes can: an
    /// exported artefact travels with a `<name>.meta.json` sidecar
    /// naming the export it came out of. A parser that finds one sets
    /// `sidecar` here and the server does the resolving.
    pub derived_from: Option<String>,
    /// Attribution kind for the subject the item is by — `"owner"` or
    /// `"subject"` (see [`AddAssetCommand::author_kind`]). Passed
    /// through verbatim; the server rejects a pair that does not hold
    /// together.
    ///
    /// A scanner rarely knows this, and `None` is the right answer when
    /// it does not: unrecorded, not the owner. An importer that *does*
    /// know (a corpus exported per subject, a parser reading an author
    /// field out of the artefact) is the one caller that can say so.
    pub author_kind: Option<String>,
    /// Subject token when `author_kind = "subject"`.
    pub author_subject: Option<String>,
    /// Agent that performed this import — the importer's own name, or
    /// whatever drove it. `None` = unrecorded; nothing is filled in on
    /// the importer's behalf.
    pub operator_ai: Option<String>,
    /// AlbumMeta statements to file on the row, keyed by names the
    /// importer chose ([`AddAssetCommand::album_meta`]).
    ///
    /// This and `extra_json` are both places a parser can put something
    /// it read out of the source, and the choice between them is the
    /// distinction `asterism_core::domain::album_meta` is built on.
    /// `extra_json` is the importer's zone: facts about the artefact,
    /// reported by whatever was holding it. This is a statement made
    /// under a name somebody chose — and the difference that matters in
    /// practice is that a statement is *findable*, because the server
    /// keeps a secondary index over these and none over the bag.
    ///
    /// So: an identifier a parser digs out of an artefact goes here when
    /// the point is to find the row by it later. A camera model or an
    /// exposure setting goes in `extra_json`, where a reader looking for
    /// what the source said will expect it.
    ///
    /// Filled in from
    /// [`Footprint::into_asset_spec`](crate::footprint::Footprint::into_asset_spec)
    /// for the three media variants, which are the artefacts that carry
    /// a metadata block to read one out of
    /// ([`Image::album_meta`](crate::footprint::Image::album_meta)); the
    /// other five come out empty and a caller that knows better sets it
    /// on the spec, the same place the attribution fields are filled in.
    ///
    /// It is empty in practice today all the same, and that is measured
    /// rather than pending — the reasoning is on `Image::album_meta`.
    pub album_meta: std::collections::BTreeMap<String, String>,
    /// A digest of the artefact's bytes, stated by the importer that
    /// read them (`sha256:<64 hex>`).
    ///
    /// The importer is already holding the payload at scan time, so a
    /// digest costs it a CPU pass and no extra I/O — and stating one
    /// lets the server propose a duplicate at ingest without opening
    /// the file itself. It buys the exact-copy case and only that: a
    /// re-export differing by a metadata chunk is the content axis,
    /// which needs the container walker and cannot be declared from
    /// outside.
    ///
    /// **It is a proposal, never a fold.** The server treats it as an
    /// unverified assertion: the hashing job re-reads the file, records
    /// whether the two agreed, and a lane that asked for an automatic
    /// fold gets one from the pass that measured the bytes rather than
    /// from the claim.
    ///
    /// `None` is the right answer for anything whose locator has no
    /// bytes of its own — one record inside a container file, a remote
    /// address, a caller-minted name. The server refuses a declaration
    /// on those, because nothing would ever check it.
    ///
    /// **The pipeline fills this in for you**, and the two conditions
    /// it fills it under are the ones above stated as code: the scanner
    /// says its payload is the whole artefact
    /// ([`SourceScanner::payload_is_whole_artefact`](crate::SourceScanner::payload_is_whole_artefact)),
    /// and this spec still carries the address that payload came from.
    /// See [`run_import`](crate::runner::run_import). A spec built by
    /// hand, outside that loop, is on its own — and can state one, with
    /// [`asterism_contract::digest::of_bytes`], which is where the
    /// notation lives so that an importer can spell a digest without
    /// depending on the domain.
    pub declared_content_hash: Option<String>,
}

/// Turns an `AssetSpec` into the wire command.
pub fn spec_to_command(spec: AssetSpec, persona_id: &str) -> AddAssetCommand {
    AddAssetCommand {
        persona_id: persona_id.to_string(),
        source_kind: spec.source_kind,
        locator: spec.locator,
        modality: spec.modality,
        occurred_at_ms: spec.occurred_at.timestamp_millis(),
        session_id: spec.session_id,
        external_session_key: spec.external_session_key,
        external_key: spec.external_key,
        bundle_id: spec.bundle_id,
        labels: spec.labels,
        register_note: spec.register_note,
        platform: spec.platform,
        file_size_bytes: spec.file_size_bytes,
        duration_ms: spec.duration_ms,
        // Both halves or neither: the spec's own pairing is what carries
        // the invariant across this hop, and the server refuses a half.
        width_px: spec.width_px,
        height_px: spec.height_px,
        extra_json: spec.extra_json,
        cover_hint: spec.cover_hint,
        // The mapper crate cannot know the ingest side's Dir layout,
        // so we leave auto-organize disabled here. Importers that
        // want to file assets on the fly set it at the batch layer
        // (`AddAssetBatchCommand::auto_organize_base_dir`) so a
        // single sweep amortises across the whole run.
        auto_organize_base_dir: None,
        derived_from: spec.derived_from,
        author_kind: spec.author_kind,
        author_subject: spec.author_subject,
        operator_ai: spec.operator_ai,
        // No `AssetSpec` field behind this, deliberately. What to do
        // about a duplicate is a policy about a run, not something a
        // parser can find in an artefact — an `AssetSpec` states what
        // was found, which is why `Footprint::into_asset_spec` leaves
        // every assertion `None` too. Per-item is also the wrong grain
        // for it: "this import lane folds silently" is the lane layer
        // of the resolution ladder, and spreading it over every spec
        // would make each item restate a setting that belongs to the
        // run. Until that layer exists, an SDK-driven import declares
        // nothing and the server treats it as undeclared.
        on_duplicate: None,
        // Passed through rather than pinned to `None`: unlike
        // `on_duplicate` above, this *is* something the side that read
        // the artefact can state, and it is a statement about what was
        // found rather than a policy about the run.
        declared_content_hash: spec.declared_content_hash,
        // Unlike the two above, this one *is* carried: what a parser
        // found in an artefact is exactly the kind of statement the
        // field exists for, so the spec's own value travels through
        // rather than being blanked here.
        album_meta: spec.album_meta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprint::{Footprint, FootprintSource, Image, Note};

    fn a_note() -> AssetSpec {
        Footprint::Note(Note {
            source: FootprintSource {
                kind: "test".into(),
                locator: "/pics/a.png#tEXt@0".into(),
                platform: None,
                external_id: None,
            },
            occurred_at: Utc::now(),
            body: "a note".into(),
            source_app: None,
            labels: Vec::new(),
            bundle_id: None,
            extra: serde_json::json!({"camera": "X100"}),
        })
        .into_asset_spec()
    }

    /// A variant with no metadata block to read states nothing, and
    /// the spec is still where a caller can add one.
    #[test]
    fn a_footprint_states_nothing_and_the_spec_is_where_one_is_added() {
        let spec = a_note();
        assert!(spec.album_meta.is_empty());
        assert!(spec.author_kind.is_none() && spec.operator_ai.is_none());

        let mut spec = spec;
        spec.album_meta
            .insert("workflow-id".into(), "wf-sdk-1".into());
        let command = spec_to_command(spec, "p-1");
        assert_eq!(
            command.album_meta.get("workflow-id").map(String::as_str),
            Some("wf-sdk-1"),
            "a statement the parser found has to reach the wire, or the \
             SDK is the one ingest road that cannot record one"
        );
    }

    /// The whole road a parser's statement takes: `Footprint` → spec →
    /// wire command.
    ///
    /// This is the path every SDK-driven importer runs on
    /// (`runner.rs` calls `spec_to_command(fp.into_asset_spec(), …)`),
    /// so a parser that reads an identifier out of a file can only
    /// record it if the value survives *both* hops. It did not until
    /// the media variants grew the field — the spec could carry a
    /// statement that nothing was able to put there.
    #[test]
    fn a_statement_a_parser_read_survives_both_hops_to_the_wire() {
        let mut image = Image {
            source: FootprintSource {
                kind: "fs".into(),
                locator: "/pics/a.png".into(),
                platform: None,
                external_id: None,
            },
            occurred_at: Utc::now(),
            external_session_key: None,
            alt: None,
            dims: None,
            file_size_bytes: None,
            labels: Vec::new(),
            bundle_id: None,
            extra: serde_json::json!({}),
            derived_from: None,
            album_meta: Default::default(),
        };
        image.album_meta.insert("catalogue".into(), "c-12".into());

        let command = spec_to_command(Footprint::Image(image).into_asset_spec(), "p-1");
        assert_eq!(
            command.album_meta.get("catalogue").map(String::as_str),
            Some("c-12")
        );
    }

    /// The two fields next to it stay blank, and for a reason that does
    /// **not** apply to `album_meta`: what to do about a duplicate is a
    /// policy about a run, and a declared digest is an assertion this
    /// note parser never made. An identifier read out of an
    /// artefact is neither — it is exactly what the parser found.
    #[test]
    fn the_assertions_a_parser_cannot_make_stay_absent() {
        let mut spec = a_note();
        spec.album_meta.insert("plate".into(), "offwhite".into());
        let command = spec_to_command(spec, "p-1");
        assert!(command.on_duplicate.is_none());
        assert!(command.declared_content_hash.is_none());
        assert_eq!(command.album_meta.len(), 1);
        // …and the importer's own zone is untouched by any of it: the
        // camera reading is a fact the source reported, and it stays
        // where a reader looking for what the source said will find it.
        assert!(
            command
                .extra_json
                .as_deref()
                .is_some_and(|bag| bag.contains("X100")),
            "{:?}",
            command.extra_json
        );
    }
}
