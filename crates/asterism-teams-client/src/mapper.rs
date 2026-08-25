//! The one mapper — what may travel, and the declaration that decides
//! it (#148 decisions 13 and 14).
//!
//! ## Why there is exactly one
//!
//! Decision 14 puts the local Asset and the projection body behind a
//! single mapper, so this module is the only place in the workspace
//! that knows both. Decision 13 then puts the *declaration* at that
//! same seam, because the body is fed from labels, groups, personas,
//! series, comments, marks and whatever the local model gains next —
//! and a declaration attached to any one of those would cover one
//! input rather than the set.
//!
//! ## An input nobody declared does not travel
//!
//! That is the property, and it is worth being precise about how it is
//! held rather than merely intended.
//!
//! **Nothing here serialises an `Asset`.** There is no
//! `serde_json::to_value(asset)` and no `#[derive(Serialize)]` on
//! anything local. [`projection_body`] builds its output by walking
//! [`DeclaredInput::ALL`] and asking each declaration for its value,
//! so a field added to `Asset` next year produces no key here: it is
//! not that it would be filtered out, it is that nothing would go
//! looking for it.
//!
//! **A declaration cannot be half-added, and the one edit the
//! compiler will not force is worth naming.** Adding an input is four
//! edits in this file: the variant, [`DeclaredInput::key`],
//! [`DeclaredInput::take`], and [`DeclaredInput::ALL`]. The middle two
//! match exhaustively, so a variant without them does not compile.
//! `ALL` is the one that fails quietly — a variant left out of it
//! simply never travels — and quiet in that direction is the safe one:
//! the failure decision 13 names is *forgetting to untick something*,
//! and a design whose slip is "it stayed home" cannot fail that way.
//!
//! **The test says so too.** `every_key_in_a_body_was_declared` builds
//! a subject with everything populated and asserts the body's key set
//! is the declared set plus the version tag, which catches a key
//! written by hand into the assembly rather than through a
//! declaration.
//!
//! ## What is declared today, and what was left out
//!
//! [`DeclaredInput::ALL`] is the answer, and it is eight lines of code
//! away; what follows is why each is there and, more usefully, why the
//! near misses are not.
//!
//! The near misses are left undeclared for one reason — decision 4's
//! argument that what can be re-derived stays home reads on a
//! description as well as on a thumbnail:
//!
//! - **`cover`** is produced by the CoverGen job from a
//!   modality-specific template. The receiving side can generate its
//!   own, and one member's template output is not a description the
//!   team should be handed as though a person wrote it.
//! - **`register_note`** is the same shape: an annotation about tone
//!   that a job fills.
//! - **`labels`** and **`keywords`** are mixed provenance — some a
//!   person applied in the grid, some an importer wrote as a
//!   `journal_kind:` prefix, and the Asset does not carry which is
//!   which. An input whose provenance is not decidable is one that
//!   cannot be declared honestly, so it is not declared.
//!
//! Any of those becomes a declaration the day the local model can say
//! who wrote it. That is a decision, taken here, in the open.

use asterism_core::domain::asset::Asset;
use asterism_core::domain::material_layer::{LayerOrigin, MaterialLayer};
use asterism_core::domain::material_mark::{MaterialAnchor, MaterialMark};
use asterism_core::error::DomainError;
use serde_json::{Map, Value};

use asterism_teams_wire::projection::PROJECTION_VERSION;

/// The key the body's own version rides under.
///
/// Decision 14 puts the branch at the mapper and the mapper reads the
/// body, so the body carries a version of its own — beside the
/// envelope's copy, which exists so that everything between the two
/// mappers can stay incurious. Short because it is in every body ever
/// written.
pub const VERSION_KEY: &str = "v";

/// One mark a person wrote, as it travels.
///
/// **Decision 4's filter is applied before one of these exists, and
/// the fields are private so that stays true.** A mark's origin is a
/// property of the layer it sits on rather than of the mark, so the
/// filter has to be applied by something that can see both — which is
/// [`PromotedMark::gather`], the only constructor there is. A caller
/// cannot assemble one of these out of a mark it liked the look of,
/// which is what makes "a `PromotedMark` is a mark whose layer origin
/// was [`LayerOrigin::User`]" a fact about the type rather than a note
/// about how to use it.
///
/// What travels is the span and the words. Not the mark's id, not its
/// layer's, not its author record — the receiving side mints its own
/// ids for everything (decision 6), and an id from here would be one
/// more thing that means nothing there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotedMark {
    start_ms: u64,
    end_ms: Option<u64>,
    body: String,
}

impl PromotedMark {
    /// Keeps the marks whose layer origin is [`LayerOrigin::User`],
    /// and drops the rest.
    ///
    /// **This is where decision 4 is enforced, and it is enforced
    /// once.** A person disagreeing with the container's chapters and
    /// writing their own is the contribution rather than a description
    /// of it, so those travel; a mark an importer wrote or a job
    /// inferred is re-derivable from the material and does not. A mark
    /// whose layer is not in `layers` is dropped rather than kept: an
    /// unknown origin is not a user origin, and the safe direction here
    /// is the one that sends less.
    pub fn gather(layers: &[MaterialLayer], marks: &[MaterialMark]) -> Vec<Self> {
        marks
            .iter()
            .filter(|mark| {
                layers
                    .iter()
                    .find(|layer| layer.id == mark.layer_id)
                    .is_some_and(|layer| layer.origin == LayerOrigin::User)
            })
            .map(|mark| {
                // An anchor is temporal and nothing else today. Written
                // as a pattern rather than an accessor so that a second
                // kind of anchor stops compiling here — what a mark is
                // anchored to is part of what travels, and a new one
                // arriving silently as "no span" would be a mark
                // crossing with its meaning left behind.
                let MaterialAnchor::Temporal(span) = mark.anchor;
                Self {
                    start_ms: span.start_ms(),
                    end_ms: span.end_ms(),
                    body: mark.body.clone(),
                }
            })
            .collect()
    }

    /// Where on the timeline the mark starts, ms.
    pub const fn start_ms(&self) -> u64 {
        self.start_ms
    }

    /// Where it ends, ms, or nothing for an instant.
    pub const fn end_ms(&self) -> Option<u64> {
        self.end_ms
    }

    /// What the person wrote.
    pub fn said(&self) -> &str {
        &self.body
    }
}

/// Everything the mapper is allowed to look at.
///
/// A struct rather than a bare `&Asset` because the marks do not live
/// on the Asset, and because the *inputs* are what a declaration
/// declares over — a caller assembling one of these can see the whole
/// set the mapper could possibly read.
#[derive(Debug, Clone, Copy)]
pub struct LocalSubject<'a> {
    /// The Asset being promoted.
    pub asset: &'a Asset,
    /// The marks a person wrote on it, already filtered by
    /// [`PromotedMark::gather`].
    pub user_marks: &'a [PromotedMark],
}

/// One input that has been declared shareable (#148 decision 13).
///
/// Adding a variant is adding a declaration, and the compiler makes it
/// a complete one: [`Self::key`] and [`Self::take`] both match
/// exhaustively, and [`Self::ALL`] is what [`projection_body`] walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeclaredInput {
    /// What the person called it.
    Title,
    /// The marks the person wrote on the material.
    UserMarks,
}

impl DeclaredInput {
    /// Every declaration, and the only thing [`projection_body`]
    /// iterates.
    pub const ALL: &'static [Self] = &[Self::Title, Self::UserMarks];

    /// The key this input travels under.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::UserMarks => "marks",
        }
    }

    /// This input's value for a subject, or nothing when the subject
    /// has none.
    ///
    /// Nothing means the key is absent rather than null: a projection
    /// says what there is to say, and a body full of nulls would make
    /// "not set here" and "cleared" the same statement.
    fn take(self, subject: &LocalSubject<'_>) -> Option<Value> {
        match self {
            Self::Title => subject
                .asset
                .title
                .as_ref()
                .filter(|title| !title.trim().is_empty())
                .map(|title| Value::String(title.clone())),
            Self::UserMarks => {
                if subject.user_marks.is_empty() {
                    return None;
                }
                Some(Value::Array(
                    subject
                        .user_marks
                        .iter()
                        .map(|mark| {
                            let mut one = Map::new();
                            one.insert("start_ms".into(), Value::from(mark.start_ms));
                            if let Some(end) = mark.end_ms {
                                one.insert("end_ms".into(), Value::from(end));
                            }
                            one.insert("said".into(), Value::String(mark.body.clone()));
                            Value::Object(one)
                        })
                        .collect(),
                ))
            }
        }
    }
}

/// Builds the body a projection travels in, or nothing when no
/// declared input had a value.
///
/// Nothing rather than `{"v": 1}`: an entry with nothing to say has no
/// projection, which is the same distinction
/// `ProjectionBody::parse` refuses an empty body over on the other
/// plane.
pub fn projection_body(subject: &LocalSubject<'_>) -> Result<Option<String>, DomainError> {
    let mut declared_values = Map::new();
    for declared in DeclaredInput::ALL {
        if let Some(value) = declared.take(subject) {
            declared_values.insert(declared.key().to_string(), value);
        }
    }
    if declared_values.is_empty() {
        return Ok(None);
    }
    // The version goes in first, so it is rendered first: this
    // workspace turns on serde_json's `preserve_order`, which makes
    // insertion order the rendered order. A reader that does not
    // recognise the version must be able to find it without
    // understanding anything else in here, and that is easiest when it
    // is the first key rather than a key somewhere.
    let mut body = Map::new();
    body.insert(VERSION_KEY.to_string(), Value::from(PROJECTION_VERSION));
    body.append(&mut declared_values);
    serde_json::to_string(&Value::Object(body))
        .map(Some)
        .map_err(|err| DomainError::Infra(anyhow::anyhow!("rendering a projection body: {err}")))
}

/// One mark as a body carried it.
///
/// A different type from [`PromotedMark`] on purpose. A `PromotedMark`
/// is a mark this machine's own layers said a person wrote; this is
/// text that arrived from somebody else's mapper, and the two must not
/// be interchangeable — a mark read off the wire has no local layer,
/// no origin this machine verified, and nothing here promotes it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadMark {
    /// Where on the timeline the mark starts, ms.
    pub start_ms: u64,
    /// Where it ends, ms, or nothing for an instant.
    pub end_ms: Option<u64>,
    /// What the promoter's mark said.
    pub said: String,
}

/// A projection body, as the mapper for its version understands it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionView {
    /// The version the body declared for itself.
    pub version: u32,
    /// What the promoter called it, if they declared a title.
    pub title: Option<String>,
    /// The marks the promoter wrote, if they declared any.
    pub marks: Vec<ReadMark>,
}

/// Reads a projection body — the half of decision 14 that says *the
/// body carries a version, and the mapper branches on it*.
///
/// **This is the only thing in the workspace that opens a body**, and
/// that is the whole point: the teams plane stores it unread, the wire
/// carries it as a string, and understanding it is one function on the
/// member's side. A caller that reaches for `serde_json` on an
/// [`EntryProjectionDto`](asterism_teams_wire::projection::EntryProjectionDto)
/// body has stepped around the seam.
///
/// The branch is on the body's own `v` rather than on the envelope's
/// copy. The envelope is the transport's declaration and the body's is
/// the author's; where they disagree the author is right, because the
/// author is what was actually written. Nothing compares them — see
/// `teams_core::domain::projection`.
///
/// A version this build does not know is an error rather than a
/// best-effort read: a newer mapper may have moved what a key means,
/// and guessing at that is how a description starts saying something
/// its author did not.
///
/// Within a version, a key nobody here recognises is **ignored**. That
/// is not the declaration leaking — the declaration governs what
/// leaves this machine, and refusing to show a person the title they
/// can plainly see because a stray key sat beside it would be a worse
/// answer than reading what is understood.
pub fn read_projection_body(body: &str) -> Result<ProjectionView, DomainError> {
    let parsed: Value = serde_json::from_str(body).map_err(|err| {
        DomainError::Validation(format!(
            "a projection body is JSON, and this one is not: {err}"
        ))
    })?;
    let version = parsed
        .get(VERSION_KEY)
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            DomainError::Validation(format!(
                "a projection body says which mapper wrote it under {VERSION_KEY:?}, and this \
                 one does not"
            ))
        })?;

    match version {
        1 => Ok(ProjectionView {
            version: 1,
            title: parsed
                .get(DeclaredInput::Title.key())
                .and_then(Value::as_str)
                .map(str::to_string),
            marks: parsed
                .get(DeclaredInput::UserMarks.key())
                .and_then(Value::as_array)
                .map(|marks| {
                    marks
                        .iter()
                        .filter_map(|mark| {
                            Some(ReadMark {
                                start_ms: mark.get("start_ms")?.as_u64()?,
                                end_ms: mark.get("end_ms").and_then(Value::as_u64),
                                said: mark.get("said")?.as_str()?.to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }),
        other => Err(DomainError::Validation(format!(
            "this build reads projection bodies at version {PROJECTION_VERSION} and this one \
             is version {other}; a newer mapper may have moved what a key means, so it is not \
             read at all"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::asset_comment::CommentAuthor;
    use asterism_core::domain::attribution::AttributionContext;
    use asterism_core::domain::material_layer::LayerRole;
    use asterism_core::domain::material_mark::TimelineSpan;
    use asterism_core::domain::value::{PersonaId, SourceKind, SourceRef};
    use chrono::Utc;

    fn an_asset() -> Asset {
        let source = SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "/tmp/x.png")
            .expect("a source");
        let mut asset = Asset::new(
            PersonaId::new(),
            source,
            None,
            Utc::now(),
            &AttributionContext::asserted(None, None).expect("stating nobody is always valid"),
        );
        asset.title = Some("A title a person wrote".to_string());
        asset
    }

    fn a_mark(layer: &MaterialLayer, said: &str) -> MaterialMark {
        MaterialMark::new(
            layer.asset_id,
            layer.id,
            MaterialAnchor::Temporal(TimelineSpan::new(1_000, Some(2_000)).unwrap()),
            CommentAuthor::User,
            said,
            Utc::now(),
        )
        .expect("a mark")
    }

    fn a_layer(asset: &Asset, origin: LayerOrigin) -> MaterialLayer {
        MaterialLayer::new(asset.id, 0, origin, LayerRole::Annotation, false, 0).expect("a layer")
    }

    #[test]
    fn every_key_in_a_body_was_declared() {
        // The check decision 13 asks for, run against a subject with
        // every declared input populated: a key in the output that no
        // declaration named would be one that started travelling
        // without anybody saying it could.
        let asset = an_asset();
        let layer = a_layer(&asset, LayerOrigin::User);
        let marks = vec![a_mark(&layer, "the chapters are wrong here")];
        let user_marks = PromotedMark::gather(std::slice::from_ref(&layer), &marks);
        let subject = LocalSubject {
            asset: &asset,
            user_marks: &user_marks,
        };

        let rendered = projection_body(&subject).unwrap().expect("a body");
        let parsed: Value = serde_json::from_str(&rendered).unwrap();
        let keys: Vec<&str> = parsed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();

        for key in &keys {
            assert!(
                *key == VERSION_KEY || DeclaredInput::ALL.iter().any(|d| d.key() == *key),
                "{key:?} is in the body and nothing declared it"
            );
        }
        assert!(keys.contains(&"title"));
        assert!(keys.contains(&"marks"));
        assert_eq!(parsed[VERSION_KEY], Value::from(PROJECTION_VERSION));
    }

    #[test]
    fn an_undeclared_field_does_not_travel() {
        // `cover` and `register_note` are set on the Asset and are not
        // declared. The mapper never looks at them, so they are not in
        // the body — and this is the assertion that would fail if
        // somebody replaced the walk with a serialisation of the
        // Asset.
        let mut asset = an_asset();
        asset.cover =
            Some(asterism_core::domain::value::CoverText::new("a generated cover line").unwrap());
        asset.register_note =
            Some(asterism_core::domain::value::RegisterNote::new("wry, understated").unwrap());
        let subject = LocalSubject {
            asset: &asset,
            user_marks: &[],
        };

        let rendered = projection_body(&subject).unwrap().expect("a body");
        assert!(!rendered.contains("a generated cover line"), "{rendered}");
        assert!(!rendered.contains("wry, understated"), "{rendered}");
        assert!(!rendered.contains("cover"), "{rendered}");
        assert!(!rendered.contains("register_note"), "{rendered}");
    }

    #[test]
    fn marks_travel_only_from_a_user_layer() {
        // Decision 4: a mark an importer wrote or a job inferred is
        // re-derivable and stays home.
        let asset = an_asset();
        let mine = a_layer(&asset, LayerOrigin::User);
        let theirs = a_layer(&asset, LayerOrigin::Imported);
        let machine = a_layer(&asset, LayerOrigin::Machine);
        let marks = vec![
            a_mark(&mine, "mine"),
            a_mark(&theirs, "the container's"),
            a_mark(&machine, "inferred"),
        ];

        let gathered = PromotedMark::gather(&[mine, theirs, machine], &marks);
        assert_eq!(gathered.len(), 1);
        assert_eq!(gathered[0].body, "mine");
    }

    #[test]
    fn a_mark_on_an_unknown_layer_is_dropped() {
        // An origin nobody can name is not a user origin.
        let asset = an_asset();
        let orphan = a_layer(&asset, LayerOrigin::User);
        let marks = vec![a_mark(&orphan, "whose is this?")];
        assert!(PromotedMark::gather(&[], &marks).is_empty());
    }

    #[test]
    fn nothing_to_say_is_no_projection_rather_than_an_empty_one() {
        let mut asset = an_asset();
        asset.title = None;
        let subject = LocalSubject {
            asset: &asset,
            user_marks: &[],
        };
        assert!(projection_body(&subject).unwrap().is_none());
    }

    #[test]
    fn the_version_is_the_first_key_a_reader_meets() {
        // `preserve_order` makes insertion order the rendered order,
        // and a reader that recognises nothing else has to be able to
        // find the version.
        let asset = an_asset();
        let subject = LocalSubject {
            asset: &asset,
            user_marks: &[],
        };
        let rendered = projection_body(&subject).unwrap().expect("a body");
        assert!(rendered.starts_with(r#"{"v":1"#), "{rendered}");
    }

    #[test]
    fn a_body_this_mapper_wrote_is_a_body_it_reads() {
        let asset = an_asset();
        let layer = a_layer(&asset, LayerOrigin::User);
        let marks = vec![a_mark(&layer, "the chapters are wrong here")];
        let user_marks = PromotedMark::gather(std::slice::from_ref(&layer), &marks);
        let subject = LocalSubject {
            asset: &asset,
            user_marks: &user_marks,
        };

        let rendered = projection_body(&subject).unwrap().expect("a body");
        let view = read_projection_body(&rendered).expect("its own body");

        assert_eq!(view.version, PROJECTION_VERSION);
        assert_eq!(view.title.as_deref(), Some("A title a person wrote"));
        assert_eq!(view.marks.len(), 1);
        assert_eq!(view.marks[0].said, "the chapters are wrong here");
        assert_eq!(view.marks[0].start_ms, 1_000);
        assert_eq!(view.marks[0].end_ms, Some(2_000));
    }

    #[test]
    fn a_version_this_build_does_not_know_is_not_guessed_at() {
        // A newer mapper may have moved what a key means. Reading it
        // anyway is how a description starts saying something its
        // author did not.
        let newer = r#"{"v":2,"title":"something a v2 mapper meant differently"}"#;
        let refused = read_projection_body(newer).expect_err("v2 is not read");
        assert!(format!("{refused}").contains("version 2"), "{refused}");
    }

    #[test]
    fn a_body_with_no_version_is_refused_rather_than_assumed_current() {
        let anonymous = r#"{"title":"who wrote me?"}"#;
        assert!(read_projection_body(anonymous).is_err());
    }

    #[test]
    fn an_unrecognised_key_does_not_stop_the_rest_being_read() {
        // The declaration governs what leaves this machine. Refusing
        // to show a person the title they can plainly see because a
        // stray key sat beside it is the worse answer.
        let odd = r#"{"v":1,"title":"still readable","something_else":42}"#;
        let view = read_projection_body(odd).expect("read what is understood");
        assert_eq!(view.title.as_deref(), Some("still readable"));
    }

    #[test]
    fn a_blank_title_is_not_a_title() {
        let mut asset = an_asset();
        asset.title = Some("   ".to_string());
        let subject = LocalSubject {
            asset: &asset,
            user_marks: &[],
        };
        assert!(projection_body(&subject).unwrap().is_none());
    }
}
