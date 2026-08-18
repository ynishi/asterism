//! `Line` — a project's canonical set, and the forge identity that
//! makes "the living one" a derivable fact (#63 decisions 1–3).
//!
//! Raw asset ids are one-off: a replacement is a different row, and
//! nothing in the core says the new row *is* the old thing, newer. The
//! entry is that statement — a minted identity above asset ids — and
//! the four merge verbs (add / replace / delete / rename) are the only
//! things that ever move it. Liveness, current name, and current
//! version all derive on read from the verb sequence, exactly like
//! `PursuitStanding`: latest event per entry wins, and history is the
//! sequence itself, not a second record of it.
//!
//! # Shape
//!
//! - [`Line`] is one named line of a project — the branch of the git
//!   analogy. v1 mints exactly one per project, named [`Line::MAIN`]
//!   (application-enforced), so "the mainline" is a description — the
//!   line named `main` — rather than a type of its own, and the
//!   schema admits siblings before the code does (the V82 admit-ahead
//!   stance).
//! - [`LineEntry`] is the identity; it carries no name column —
//!   the current name is the latest naming verb's, so renames are
//!   history like everything else.
//! - [`LineEvent`] is one verb applied to one entry.
//!   [`LineVerb`] carries each verb's payload so a caller cannot
//!   file an add without a name or a delete with an asset (the
//!   `RestampSubject` stance); storage enforces the same pairing with
//!   two-way CHECKs.
//! - [`Merge`] is the record that one satisfied close applied its
//!   verbs — approval *is* the merge event, so every event names the
//!   merge it landed under, and who approved derives through the
//!   close event's attribution rather than being copied here.
//!
//! # The boundary, restated
//!
//! A line *references* asset ids; it never annotates or mutates
//! an asset (the PR #62 rule). A dead entry's asset row stays live and
//! restorable — `delete` is a statement about the canonical set, not
//! about bytes, the same distance `CullVerdict::Reject` keeps from
//! trash.
//!
//! # Invariants (service-enforced, entity-checked where local)
//!
//! - Verb payload pairing and non-blank names are checked here.
//! - Living-name uniqueness within a line is an application rule
//!   checked at merge time — dead names are reusable, so it cannot be
//!   a schema constraint.
//! - An entry's first event is an `add`; later events land on an
//!   existing entry. The write path (P3) enforces this; on read the
//!   derive tolerates a dangling tail by answering `None` on the axes
//!   the missing `add` would have filled. That tolerance is *weaker*
//!   than the ledger's, which drops a dangling gesture's asset from
//!   membership outright (`tx.rs`) — deliberately so: an event row
//!   names an entry that exists, so deriving its presence states
//!   nothing false, where a membership would.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::value::{
    AssetId, LineEntryId, LineEventId, LineId, MergeId, PersonaId, ProjectId, PursuitEventId,
};
use crate::error::DomainError;

/// Rejects a blank name so "named" means something; returns the
/// trimmed form storage keeps.
fn required_name(name: String, what: &str) -> Result<String, DomainError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(DomainError::Validation(format!(
            "{what} name must not be blank"
        )));
    }
    Ok(name)
}

/// One named line of a project. v1 restricts a project to exactly one,
/// named [`Self::MAIN`] and minted in the same transaction as the
/// project — an application rule, so the day named lines arrive the
/// schema is already there.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Surrogate id (UUID v7).
    pub id: LineId,
    /// The project this line belongs to.
    pub project_id: ProjectId,
    /// Line name, unique within the project (schema-enforced — lines
    /// have no death, so a UNIQUE index is honest here where it would
    /// not be for living names).
    pub name: String,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

impl Line {
    /// The one line name v1 mints.
    pub const MAIN: &'static str = "main";

    /// Builds a project's main line — the only constructor until
    /// named lines are a modelled thing, so the restriction has one
    /// author.
    pub fn main(project_id: ProjectId, now: DateTime<Utc>) -> Self {
        Self {
            id: LineId::new(),
            project_id,
            name: Self::MAIN.to_string(),
            created_at: now,
        }
    }

    /// Read-path twin: restores a stored row as a fact.
    pub fn from_persisted(
        id: LineId,
        project_id: ProjectId,
        name: String,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            project_id,
            name,
            created_at,
        }
    }
}

/// The forge identity above raw asset ids (#63 decision 1). Deliberately
/// name-less and version-less: both derive from the verb sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineEntry {
    /// Surrogate id (UUID v7).
    pub id: LineEntryId,
    /// The line this identity lives on.
    pub line_id: LineId,
    /// Redundant persona copy (the `pursuit_event.persona_id`
    /// precedent).
    pub persona_id: PersonaId,
    /// Creation time — the moment of its first `add`.
    pub created_at: DateTime<Utc>,
}

impl LineEntry {
    /// Builds a fresh entry, minted alongside the `add` that names it.
    pub fn new(line_id: LineId, persona_id: PersonaId, now: DateTime<Utc>) -> Self {
        Self {
            id: LineEntryId::new(),
            line_id,
            persona_id,
            created_at: now,
        }
    }

    /// Read-path twin: restores a stored row as a fact.
    pub fn from_persisted(
        id: LineEntryId,
        line_id: LineId,
        persona_id: PersonaId,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            line_id,
            persona_id,
            created_at,
        }
    }
}

/// The closed set of merge verbs, payload included (#63 decision 2).
/// An enum rather than `(kind, Option, Option)` columns so a caller
/// cannot file an `add` without a name or a `delete` carrying an asset
/// — the `RestampSubject` stance; the schema states the same pairing
/// as two-way CHECKs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineVerb {
    /// A new entry enters the canonical set, named.
    Add {
        /// The version that enters as the living one.
        asset_id: AssetId,
        /// The entry's name at birth.
        name: String,
    },
    /// The living version moves to a new asset; identity and name
    /// stay. Whole-object — creative files do not partial-merge.
    Replace {
        /// The version that becomes the living one.
        asset_id: AssetId,
    },
    /// The entry leaves the canonical set. Its asset rows stay live —
    /// this is a statement about the set, not about bytes.
    Delete,
    /// The entry's name moves; identity and version stay.
    Rename {
        /// The name from this event on.
        name: String,
    },
}

impl LineVerb {
    /// Storage slug for the `verb` column.
    pub fn kind_slug(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Replace { .. } => "replace",
            Self::Delete => "delete",
            Self::Rename { .. } => "rename",
        }
    }

    /// The payload asset, for the nullable `asset_id` column.
    pub fn asset_id(&self) -> Option<&AssetId> {
        match self {
            Self::Add { asset_id, .. } | Self::Replace { asset_id } => Some(asset_id),
            Self::Delete | Self::Rename { .. } => None,
        }
    }

    /// The payload name, for the nullable `name` column.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Add { name, .. } | Self::Rename { name } => Some(name),
            Self::Replace { .. } | Self::Delete => None,
        }
    }

    /// Parses the stored `(verb, asset_id, name)` columns (closed set —
    /// an unknown verb or a mismatched payload is a corrupt row, not a
    /// forward-compat case).
    pub fn from_columns(
        verb: &str,
        asset_id: Option<Uuid>,
        name: Option<String>,
    ) -> Result<Self, DomainError> {
        let mismatch = |what: &str| {
            DomainError::Validation(format!("line verb {verb:?}: payload mismatch ({what})"))
        };
        match (verb, asset_id, name) {
            ("add", Some(asset), Some(name)) => Ok(Self::Add {
                asset_id: AssetId::from_uuid(asset),
                name,
            }),
            ("replace", Some(asset), None) => Ok(Self::Replace {
                asset_id: AssetId::from_uuid(asset),
            }),
            ("delete", None, None) => Ok(Self::Delete),
            ("rename", None, Some(name)) => Ok(Self::Rename { name }),
            ("add" | "replace" | "delete" | "rename", asset, name) => Err(mismatch(&format!(
                "asset {}, name {}",
                if asset.is_some() { "present" } else { "absent" },
                if name.is_some() { "present" } else { "absent" },
            ))),
            (other, _, _) => Err(DomainError::Validation(format!(
                "unknown line verb: {other:?}"
            ))),
        }
    }
}

/// One verb applied to one entry, filed under the merge that landed it.
#[derive(Debug, Clone, PartialEq)]
pub struct LineEvent {
    /// Surrogate id (UUID v7) — the tie-break that makes "latest event"
    /// total when two verbs share a `created_at`.
    pub id: LineEventId,
    /// The entry this verb moves.
    pub entry_id: LineEntryId,
    /// Redundant persona copy.
    pub persona_id: PersonaId,
    /// What happened, payload included.
    pub verb: LineVerb,
    /// The merge this verb landed under. Every event has one —
    /// approval is the merge event, and there is no other author of
    /// line change.
    pub merge_id: MergeId,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

impl LineEvent {
    /// Builds a fresh event. Names arriving on the verb are trimmed
    /// and must be non-blank — a nameless add has nothing to answer
    /// "what is alive" with.
    pub fn new(
        entry_id: LineEntryId,
        persona_id: PersonaId,
        verb: LineVerb,
        merge_id: MergeId,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        let verb = match verb {
            LineVerb::Add { asset_id, name } => LineVerb::Add {
                asset_id,
                name: required_name(name, "entry")?,
            },
            LineVerb::Rename { name } => LineVerb::Rename {
                name: required_name(name, "entry")?,
            },
            other => other,
        };
        Ok(Self {
            id: LineEventId::new(),
            entry_id,
            persona_id,
            verb,
            merge_id,
            created_at: now,
        })
    }

    /// Read-path twin: restores a stored row as a fact rather than a
    /// request to accept.
    pub fn from_persisted(
        id: LineEventId,
        entry_id: LineEntryId,
        persona_id: PersonaId,
        verb: LineVerb,
        merge_id: MergeId,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            entry_id,
            persona_id,
            verb,
            merge_id,
            created_at,
        }
    }
}

/// The record that one satisfied close applied its verbs (#63 decision
/// 3). One per close event (UNIQUE in storage); an empty merge — a
/// satisfied close that landed nothing — is a defined state, not a
/// missing row, so "this close was an approval act" stays readable
/// even when nothing changed.
///
/// No attribution triple here: who approved *is* who closed, and the
/// close event already carries the triple — copying it would mint a
/// second author for one act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Merge {
    /// Surrogate id (UUID v7).
    pub id: MergeId,
    /// The `ClosedSatisfied` event this merge is the landing of.
    pub pursuit_event_id: PursuitEventId,
    /// Redundant persona copy.
    pub persona_id: PersonaId,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

impl Merge {
    /// Builds a fresh merge record for a satisfied close.
    pub fn new(
        pursuit_event_id: PursuitEventId,
        persona_id: PersonaId,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: MergeId::new(),
            pursuit_event_id,
            persona_id,
            created_at: now,
        }
    }

    /// Read-path twin: restores a stored row as a fact.
    pub fn from_persisted(
        id: MergeId,
        pursuit_event_id: PursuitEventId,
        persona_id: PersonaId,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            pursuit_event_id,
            persona_id,
            created_at,
        }
    }
}

/// One entry's derived position: alive or dead, and what it currently
/// answers to. All three axes derive independently — liveness from the
/// latest event of any verb, the name from the latest naming verb, the
/// version from the latest asset-bearing verb — so a delete does not
/// erase what the entry was called (dead names are readable, merely
/// reusable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryState {
    /// Whether the latest event leaves the entry in the canonical set.
    /// An entry with no events yet is not alive — nothing has landed.
    pub alive: bool,
    /// The latest naming verb's name (`None` only on a dangling tail
    /// that never saw an `add`).
    pub name: Option<String>,
    /// The latest asset-bearing verb's asset (`None` under the same
    /// tolerance).
    pub asset_id: Option<AssetId>,
}

/// Derives one entry's state from its events: latest by
/// `(created_at, id)` wins, per axis. The input does not need to be
/// sorted. A sequence that never saw an `add` is a dangling tail —
/// the write path refuses to create one; on read it derives with
/// `None` where the missing `add` would have answered, and yields no
/// phantom liveness beyond what its verbs state.
pub fn entry_state<'a, I>(events: I) -> EntryState
where
    I: IntoIterator<Item = &'a LineEvent>,
{
    let mut latest: Option<(DateTime<Utc>, LineEventId, bool)> = None;
    let mut latest_name: Option<(DateTime<Utc>, LineEventId, &str)> = None;
    let mut latest_asset: Option<(DateTime<Utc>, LineEventId, &AssetId)> = None;
    for event in events {
        let key = (event.created_at, event.id);
        let dead = matches!(event.verb, LineVerb::Delete);
        if latest.map(|(t, i, _)| key > (t, i)).unwrap_or(true) {
            latest = Some((key.0, key.1, dead));
        }
        if let Some(name) = event.verb.name()
            && latest_name.map(|(t, i, _)| key > (t, i)).unwrap_or(true)
        {
            latest_name = Some((key.0, key.1, name));
        }
        if let Some(asset) = event.verb.asset_id()
            && latest_asset.map(|(t, i, _)| key > (t, i)).unwrap_or(true)
        {
            latest_asset = Some((key.0, key.1, asset));
        }
    }
    EntryState {
        alive: latest.map(|(_, _, dead)| !dead).unwrap_or(false),
        name: latest_name.map(|(_, _, name)| name.to_string()),
        asset_id: latest_asset.map(|(_, _, asset)| *asset),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 17, 12, minute, 0).unwrap()
    }

    fn event(verb: LineVerb, minute: u32) -> LineEvent {
        LineEvent::new(
            LineEntryId::from_uuid(Uuid::nil()),
            PersonaId::new(),
            verb,
            MergeId::new(),
            at(minute),
        )
        .unwrap()
    }

    fn add(name: &str, minute: u32) -> (AssetId, LineEvent) {
        let asset = AssetId::new();
        (
            asset,
            event(
                LineVerb::Add {
                    asset_id: asset,
                    name: name.into(),
                },
                minute,
            ),
        )
    }

    #[test]
    fn an_add_makes_an_entry_alive_named_and_versioned() {
        let (asset, add) = add("key visual", 0);
        let state = entry_state([&add]);
        assert_eq!(
            state,
            EntryState {
                alive: true,
                name: Some("key visual".into()),
                asset_id: Some(asset),
            }
        );
    }

    #[test]
    fn no_events_is_not_alive() {
        assert_eq!(
            entry_state([]),
            EntryState {
                alive: false,
                name: None,
                asset_id: None,
            }
        );
    }

    #[test]
    fn a_delete_kills_but_keeps_the_name_readable_and_a_later_add_revives() {
        let (_, born) = add("key visual", 0);
        let deleted = event(LineVerb::Delete, 1);
        let state = entry_state([&born, &deleted]);
        assert!(!state.alive);
        assert_eq!(state.name.as_deref(), Some("key visual"));

        let (revived_asset, revived) = add("key visual", 2);
        let state = entry_state([&born, &deleted, &revived]);
        assert!(state.alive);
        assert_eq!(state.asset_id, Some(revived_asset));
    }

    #[test]
    fn replace_and_rename_move_their_own_axis_only() {
        let (_, born) = add("draft", 0);
        let replacement = AssetId::new();
        let replaced = event(
            LineVerb::Replace {
                asset_id: replacement,
            },
            1,
        );
        let renamed = event(
            LineVerb::Rename {
                name: "final".into(),
            },
            2,
        );
        let state = entry_state([&born, &replaced, &renamed]);
        assert_eq!(
            state,
            EntryState {
                alive: true,
                name: Some("final".into()),
                asset_id: Some(replacement),
            }
        );
    }

    #[test]
    fn a_shared_timestamp_falls_back_to_the_id_tie_break() {
        let (_, born) = add("draft", 0);
        let a = event(LineVerb::Delete, 1);
        let b = event(
            LineVerb::Replace {
                asset_id: AssetId::new(),
            },
            1,
        );
        // v7 ids order by mint time; whichever minted later wins, and
        // the answer is the same whatever order the scan visits.
        let expect_alive = b.id > a.id;
        assert_eq!(entry_state([&born, &a, &b]).alive, expect_alive);
        assert_eq!(entry_state([&born, &b, &a]).alive, expect_alive);
    }

    #[test]
    fn a_dangling_tail_answers_none_where_the_missing_add_would_have() {
        let replacement = AssetId::new();
        let tail = event(
            LineVerb::Replace {
                asset_id: replacement,
            },
            0,
        );
        let state = entry_state([&tail]);
        assert!(state.alive, "a replace states presence, not birth");
        assert_eq!(state.name, None, "nothing ever named this entry");
        assert_eq!(state.asset_id, Some(replacement));

        let deleted = event(LineVerb::Delete, 0);
        assert!(!entry_state([&deleted]).alive);
    }

    #[test]
    fn a_blank_name_is_refused_on_add_and_rename() {
        let refused = LineEvent::new(
            LineEntryId::new(),
            PersonaId::new(),
            LineVerb::Add {
                asset_id: AssetId::new(),
                name: "   ".into(),
            },
            MergeId::new(),
            at(0),
        );
        assert!(refused.is_err());
        let refused = LineEvent::new(
            LineEntryId::new(),
            PersonaId::new(),
            LineVerb::Rename { name: "".into() },
            MergeId::new(),
            at(0),
        );
        assert!(refused.is_err());
    }

    #[test]
    fn from_columns_is_a_closed_set_and_refuses_mismatched_payloads() {
        let asset = Uuid::now_v7();
        assert!(LineVerb::from_columns("add", Some(asset), Some("a".into())).is_ok());
        assert!(LineVerb::from_columns("replace", Some(asset), None).is_ok());
        assert!(LineVerb::from_columns("delete", None, None).is_ok());
        assert!(LineVerb::from_columns("rename", None, Some("b".into())).is_ok());

        assert!(LineVerb::from_columns("add", Some(asset), None).is_err());
        assert!(LineVerb::from_columns("replace", None, None).is_err());
        assert!(LineVerb::from_columns("delete", Some(asset), None).is_err());
        assert!(LineVerb::from_columns("rename", None, None).is_err());
        assert!(LineVerb::from_columns("fold", None, None).is_err());
    }

    #[test]
    fn the_main_line_is_named_main() {
        let line = Line::main(ProjectId::new(), at(0));
        assert_eq!(line.name, Line::MAIN);
    }
}
