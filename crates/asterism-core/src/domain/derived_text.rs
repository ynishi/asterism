//! Derived text — the one string an asset offers a full-text index,
//! assembled from everything the row already says about itself.
//!
//! # The gap this fills
//!
//! Until this module existed, "what does search see" had exactly one
//! answer: the bytes of the original, when those bytes were text. A
//! conversation transcript was searchable and a picture was not, and
//! the picture was not searchable *even though the library already
//! held sentences about it* — a title somebody typed, the alt text an
//! importer lifted out of the page it came from, the keywords the
//! auto-tag pass wrote, the generation prompt sitting in a PNG `tEXt`
//! chunk, a note a person left in the comment thread. Every one of
//! those is text, on the row, already stored. None of them reached the
//! index, so the honest description of the search surface was "text
//! files only", and a library that is mostly pictures had mostly
//! nothing to search.
//!
//! The fix is not a new store. It is to stop treating "the body" as a
//! synonym for "the file's bytes": the body an index wants is a
//! *projection* of the asset, and the file is one field of it.
//!
//! # Why a pure function
//!
//! Composition happens here, in the domain, and not in the job that
//! writes the index, for the ordinary reason — the rule for what is
//! searchable about an asset is a statement about assets, and a rule
//! that lives inside an infrastructure handler can only be tested by
//! standing up a queue. The handler's job is to *fetch* (the file, the
//! comment thread) and to *write* (the body cache, the two indexes);
//! deciding what the text is belongs to the entity that has the
//! fields.
//!
//! The two inputs the asset cannot supply itself are arguments rather
//! than ports, for the same reason: `file_body` lives behind a reader
//! of the outside world and `comment_bodies` behind a second
//! aggregate's repository. Handing them in keeps this function total,
//! synchronous, and free of a trait object.
//!
//! # What is deliberately left out
//!
//! - **`_trace` apart from `meta`.** The trace bag holds Album's own
//!   bookkeeping — provenance claims, a declared hash and its verdict,
//!   resolution flags. Those are assertions *about the record*, not
//!   words about the subject, and indexing them would put an internal
//!   status word in the same haystack as a person's sentence. Only
//!   [`album_meta::META_KEY`] — the zone whose whole purpose is "a
//!   statement somebody made, under a name they chose" — is read.
//! - **The `source` / `operator` / `declared_at_ms` fields of a
//!   declared-meta entry.** They describe the statement, not its
//!   content; `manual` appearing in every document is a term that
//!   matches everything and distinguishes nothing.
//! - **Identifiers.** No `AssetId`, no locator, no persona id. A UUID
//!   is not a word, and a locator is already the address the row is
//!   found by.
//! - **Tags.** The one exclusion here that is a *judgement* rather than
//!   a type argument, so it is written down. A tag is already a precise
//!   instrument: the sidebar filters on it, `tag_counts` counts it, and
//!   a person who tagged something can get back exactly the set they
//!   tagged. Full text is the opposite instrument — it exists for the
//!   words somebody would guess, which is why a picture needs it and a
//!   tag does not [Furnas et al. 1987: two people choose the same term
//!   for one thing under 20% of the time, which is the case *for*
//!   indexing prose and *against* diluting it with a controlled
//!   vocabulary that already answers exactly]. Folding tags in would
//!   also make every tag rename a re-composition of every document that
//!   carried it.
//!
//!   Reversing this is a decision somebody may make, and it is not a
//!   one-line change: it needs [`COMPOSITION_VERSION`] raised (so the
//!   walk re-composes the library) and a re-index wired into all seven
//!   tag verbs on `AssetService` — `attach_tag`, `detach_tag`, their two
//!   batch forms, `rename_tag`, `delete_tag` and `merge_tags` — since
//!   none of them touches the asset row today.
//!
//! # Ordering
//!
//! Sections come out in a fixed order — file body, title, cover,
//! labels, keywords, register note, material metadata (recovered text
//! then the digest's body), declared meta, comments — so the derived
//! string is a function of the asset's state
//! and nothing else. Two runs over an unchanged row produce the same
//! bytes, which is what makes "the body cache is stale" a decidable
//! question rather than a diff of two orderings.
//!
//! Nothing here is a ranking signal: the joined string is one flat
//! field, so a title does not outweigh a comment. Field-weighted
//! scoring is a change to the index schema, not to this function.

use crate::domain::album_meta;
use crate::domain::asset::Asset;
use crate::domain::provenance::TRACE_KEY;

/// Which reading of an asset a cached body was composed by.
///
/// A body cache is a *derived* value, and the thing it is derived by is
/// this function — so a row holding a body says nothing useful unless
/// it also says which version of the composition produced it. Without
/// that, "already indexed" and "indexed by a build that only ever read
/// file bytes" are the same state, and the backfill can only find rows
/// that have no body at all. That is exactly the gap the first
/// derivation walk left: every picture was visible to it (no body) and
/// every text asset was invisible (a body composed from the file
/// alone), so a transcript's title, keywords and comment thread stayed
/// out of its own document.
///
/// Stored on `asset_body.derived_version`. `NULL` there is version 0 —
/// a body written before this constant existed — and any value below
/// the current one marks the row as work for
/// `index_rebuild_batch`. Raise it by one whenever
/// [`derive_text`] starts composing from a section it did not read
/// before; that is the whole protocol, and it is what makes a new
/// surface reach documents that already exist.
pub const COMPOSITION_VERSION: i64 = 1;

/// Builds the indexable text for one asset, or `None` when the row has
/// nothing to say.
///
/// `file_body` is the original's text when the original *is* text
/// (`None` for a picture, an unreadable file, or a record that
/// addresses something inside a container). `comment_bodies` is the
/// asset's comment thread in reading order; an empty slice is the
/// ordinary case.
///
/// `None` is returned only when every section is empty — an asset with
/// no title, no cover, no labels, no keywords, no note, no material
/// metadata, no declared meta, no comments, and no readable text of
/// its own. That is a real state (a freshly imported picture, before
/// any job has run) and it is distinct from the empty string: the
/// caller uses it to decide *not to write a document at all*, where an
/// empty body would mean "we looked and the answer is nothing".
///
/// Blank sections are dropped rather than joined, so the result never
/// carries a run of empty lines, and a value that is only whitespace
/// counts as absent.
pub fn derive_text(
    asset: &Asset,
    file_body: Option<&str>,
    comment_bodies: &[String],
) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();

    // The original's own text, first and verbatim. It is the longest
    // section by far when it exists, and keeping it unmodified means
    // the pre-existing behaviour for text assets is preserved exactly:
    // a transcript indexed before this module existed derives to the
    // same leading bytes afterwards.
    push(&mut sections, file_body);
    push(&mut sections, asset.title.as_deref());
    push(&mut sections, asset.cover.as_ref().map(|c| c.as_str()));
    for label in &asset.labels {
        push(&mut sections, Some(label.as_str()));
    }
    for keyword in &asset.keywords {
        push(&mut sections, Some(keyword.as_str()));
    }
    push(
        &mut sections,
        asset.register_note.as_ref().map(|n| n.as_str()),
    );

    // Material metadata — for a generated image this is where the
    // prompt is. Both columns hold the same canonical JSON shape (keys
    // sorted, values exactly as the container stated them), so both
    // halves of each entry are worth having: the key is the container's
    // own word for the field (`parameters`, `workflow`, `Description`)
    // and a person searching for what they typed is searching the
    // value.
    //
    // Two columns rather than one because they are two readings of the
    // same chunks, and neither contains the other. `meta_text` is the
    // generous one — it carries the `zTXt` / `iTXt` the digest excludes
    // and the Latin-1 bytes it replaces — but it is `NULL` on every row
    // no recovery pass has reached yet, and `meta_kv` has been filled
    // since the meta axis landed. Reading both and letting the set
    // union them is what makes this correct during the backfill instead
    // of after it; the duplicate terms an overlapping key produces cost
    // a repeated word in one document and nothing else.
    for material in &asset.materials {
        for raw in [material.meta_text.as_deref(), material.meta_kv.as_deref()]
            .into_iter()
            .flatten()
        {
            match serde_json::from_str::<serde_json::Value>(raw) {
                Ok(serde_json::Value::Object(fields)) => {
                    for (key, value) in fields {
                        push(&mut sections, Some(&key));
                        push(&mut sections, Some(&scalar_text(&value)));
                    }
                }
                // Anything else — a parse failure, or valid JSON that is
                // not an object — falls back to the raw column. The digest
                // beside it was taken over exactly these bytes, so they are
                // what the container carried; refusing to index them
                // because this function could not take them apart would
                // lose the words over a shape disagreement.
                _ => push(&mut sections, Some(raw)),
            }
        }
    }

    // Declared meta: what a person or an agent said about this asset,
    // under a name they chose. Only the stated value is text somebody
    // wrote — the rest of the entry describes the statement.
    for (key, entry) in declared_meta(asset) {
        push(&mut sections, Some(key));
        if let Some(value) = entry.get("value") {
            push(&mut sections, Some(&scalar_text(value)));
        }
    }

    for body in comment_bodies {
        push(&mut sections, Some(body));
    }

    if sections.is_empty() {
        return None;
    }
    Some(sections.join("\n"))
}

/// Appends `value` when it carries a non-blank string.
fn push(sections: &mut Vec<String>, value: Option<&str>) {
    if let Some(text) = value
        && !text.trim().is_empty()
    {
        sections.push(text.to_string());
    }
}

/// Renders a JSON value as the text a reader would have typed.
///
/// A JSON string is unwrapped rather than re-serialised, because
/// `"a prompt"` with its quotes is not the string somebody searches
/// for. Everything else keeps its JSON spelling — a number, a bool,
/// or a nested structure has no more faithful flat rendering, and
/// walking further into it would be inventing a schema for a bag that
/// deliberately has none.
fn scalar_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Iterates the `extra._trace.meta` object, or nothing when the asset
/// carries no declared meta.
fn declared_meta(asset: &Asset) -> impl Iterator<Item = (&str, &serde_json::Value)> {
    asset
        .extra
        .get(TRACE_KEY)
        .and_then(|trace| trace.get(album_meta::META_KEY))
        .and_then(|meta| meta.as_object())
        .into_iter()
        .flat_map(|map| map.iter().map(|(k, v)| (k.as_str(), v)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::attribution::AttributionContext;
    use crate::domain::material::Material;
    use crate::domain::source_locator::SourceLocator;
    use crate::domain::value::{CoverText, Label, PersonaId, SourceKind, SourceRef};
    use chrono::Utc;

    /// A bare asset with no derivable text — the shape each test adds
    /// exactly the fields it is about.
    fn asset() -> Asset {
        Asset::new(
            PersonaId::new(),
            SourceRef::new(SourceKind::new("fs").unwrap(), "/pics/a.png").unwrap(),
            None,
            Utc::now(),
            &AttributionContext::unrecorded(),
        )
    }

    /// Attaches a primary material carrying the two metadata columns
    /// verbatim — the digest's body and the recovered text.
    fn with_material_meta(asset: &mut Asset, meta_kv: Option<&str>, meta_text: Option<&str>) {
        let mut material = Material::primary(
            SourceLocator::from_wire("/pics/a.png").unwrap(),
            None,
            asset.created_at,
        );
        material.meta_kv = meta_kv.map(str::to_string);
        material.meta_text = meta_text.map(str::to_string);
        asset.materials.push(material);
    }

    /// The common case: only the digest's body is filled.
    fn with_meta_kv(asset: &mut Asset, raw: &str) {
        with_material_meta(asset, Some(raw), None);
    }

    /// The case the module exists for: a picture has no bytes an index
    /// can read, and every word about it still reaches the document.
    #[test]
    fn a_picture_derives_from_what_the_row_says_about_it() {
        let mut asset = asset();
        asset.cover = Some(CoverText::new("a lighthouse at dusk").unwrap());
        asset.labels = vec![Label::new("keeper").unwrap()];
        with_meta_kv(
            &mut asset,
            r#"{"parameters":"lighthouse, dusk, long exposure"}"#,
        );

        let derived = derive_text(&asset, None, &[]).expect("the row carries text");

        assert!(derived.contains("a lighthouse at dusk"), "{derived}");
        assert!(derived.contains("keeper"), "{derived}");
        assert!(derived.contains("parameters"), "{derived}");
        assert!(
            derived.contains("lighthouse, dusk, long exposure"),
            "the prompt is the point: {derived}"
        );
        // The prompt is the value of a JSON string, and it lands
        // without the quotes it was stored inside.
        assert!(!derived.contains(r#""lighthouse, dusk"#), "{derived}");
    }

    /// An asset nobody has said anything about yet is not an empty
    /// document — it is no document, which is what lets the caller skip
    /// the write instead of storing a blank body.
    #[test]
    fn an_asset_with_nothing_to_say_derives_nothing() {
        assert_eq!(derive_text(&asset(), None, &[]), None);
        // Whitespace is not something to say either.
        let mut blank = asset();
        blank.title = Some("   ".into());
        assert_eq!(derive_text(&blank, Some("\n\n"), &[]), None);
    }

    /// A declared statement contributes its value and its name, and
    /// nothing that merely describes the statement. `manual` in every
    /// document is a term that separates no two rows.
    #[test]
    fn a_declared_statement_contributes_its_value_and_not_its_provenance() {
        let mut asset = asset();
        asset.extra = serde_json::json!({
            "_trace": {
                "meta": {
                    "workflow-id": album_meta::entry(
                        "harbour-v3",
                        "manual",
                        Some("claude-code"),
                        1_700_000_000_000,
                    ),
                },
                // A sibling of `meta` inside the same bag: bookkeeping,
                // and deliberately not indexed.
                "resolved": false,
            },
        });

        let derived = derive_text(&asset, None, &[]).expect("the statement is text");

        assert!(derived.contains("workflow-id"), "{derived}");
        assert!(derived.contains("harbour-v3"), "{derived}");
        assert!(
            !derived.contains("manual"),
            "the source is not text: {derived}"
        );
        assert!(
            !derived.contains("claude-code"),
            "the operator is not text: {derived}"
        );
        assert!(!derived.contains("declared_at_ms"), "{derived}");
        assert!(!derived.contains("resolved"), "{derived}");
    }

    /// A note on the thread is a sentence about the asset, so it is
    /// part of what the asset is findable by — and a thread edited
    /// after the fact is why the index has to be rebuilt on comment
    /// writes rather than at ingest alone.
    #[test]
    fn a_comment_is_part_of_what_the_asset_is_findable_by() {
        let asset = asset();
        let derived = derive_text(
            &asset,
            None,
            &["the one we printed for the hallway".to_string()],
        )
        .expect("a comment alone is enough");
        assert_eq!(derived, "the one we printed for the hallway");
    }

    /// The recovered column carries what the digest's body cannot —
    /// a compressed chunk, an accented word — and reaches the document
    /// on the same terms.
    #[test]
    fn recovered_metadata_reaches_the_document_beside_the_digests_body() {
        let mut asset = asset();
        with_material_meta(
            &mut asset,
            // What the meta axis saw: `tEXt` only, read lossily.
            Some(r#"{"Software":"a generator"}"#),
            // What the recovery saw: the same file's `zTXt`, plus the
            // byte the lossy read had replaced.
            Some(
                r#"{"Software":"a generator","comment":"Café window","parameters":"a lighthouse"}"#,
            ),
        );

        let derived = derive_text(&asset, None, &[]).expect("the row carries text");

        assert!(
            derived.contains("a lighthouse"),
            "the compressed chunk: {derived}"
        );
        assert!(
            derived.contains("Café window"),
            "the recovered byte: {derived}"
        );
        assert!(derived.contains("a generator"), "{derived}");
    }

    /// A row the recovery pass has reached and found nothing in is not
    /// a row with nothing to say — the two columns are read as a set,
    /// so an empty one subtracts nothing.
    #[test]
    fn an_empty_recovery_does_not_hide_the_digests_body() {
        let mut asset = asset();
        with_material_meta(
            &mut asset,
            Some(r#"{"parameters":"a lighthouse"}"#),
            Some("{}"),
        );

        let derived = derive_text(&asset, None, &[]).expect("the digest's body is still text");
        assert!(derived.contains("a lighthouse"), "{derived}");
    }

    /// The metadata column is whatever the container carried. When it
    /// does not parse as an object, the words in it are still the
    /// words somebody would search for.
    #[test]
    fn unparseable_material_metadata_is_indexed_as_it_stands() {
        let mut asset = asset();
        with_meta_kv(&mut asset, "Steps: 30, Sampler: Euler a — not JSON");

        let derived = derive_text(&asset, None, &[]).expect("raw metadata is still text");
        assert_eq!(derived, "Steps: 30, Sampler: Euler a — not JSON");
    }

    /// The file's own text still leads, unmodified — a transcript
    /// indexed before this module existed derives to the same leading
    /// bytes afterwards.
    #[test]
    fn the_originals_text_still_comes_first() {
        let mut asset = asset();
        asset.title = Some("session notes".into());
        let derived = derive_text(&asset, Some("what we said"), &[]).unwrap();
        assert_eq!(derived, "what we said\nsession notes");
    }
}
