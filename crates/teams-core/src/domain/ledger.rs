//! `ledger` — the actor-stamped, append-only event envelope (#83 §2).
//!
//! Each team has one stream. The substrate knows the envelope and
//! nothing inside it: `payload` is a versioned body *per kind* and
//! stays opaque here, `subjects` is the typed index trace queries walk
//! instead of parsing payloads, and `kind` is a namespaced + versioned
//! string so `forge.*` kinds can register after #63 with the envelope
//! unchanged.
//!
//! Two things this module deliberately does **not** do:
//!
//! - **Generate `seq`.** Monotonicity within a team is a storage
//!   guarantee (one SQLite tx, single writer by deployment shape), so
//!   [`EventSeq`] is a newtype the domain validates and carries but
//!   never mints — a domain-side counter would be a second writable
//!   truth, the one forbidden shape.
//! - **Source state from events.** State tables are authoritative and
//!   every state change appends its event in the same tx (audit-log
//!   pattern, not event sourcing — #83 §2 SoT note). Nothing here
//!   replays.
//!
//! ## Erasing a person answers in three places
//!
//! A request to erase somebody is not one deletion, and the reason is
//! that a person is answered for in three records, each holding a
//! different part of the answer — a name in one, an association in the
//! next, a handle in the third. All three have to answer, and an
//! answer that covers two of them has erased nothing:
//!
//! 1. **The [`ActorStamp`](crate::domain::identity::ActorStamp) on
//!    every ledger event they wrote.** The name is captured at write
//!    time precisely so a later rename does not rewrite history — the
//!    property that makes the ledger readable is the property that
//!    makes it hold the name.
//! 2. **Their rows in the subject index.** A `user` subject is a uuid
//!    rather than a name, so what it exposes is the association: which
//!    events touched this person, which is the question the index
//!    exists to answer quickly.
//! 3. **`forge_actor` on the local plane.** The handle a member's
//!    writes resolve to is minted on their own instance, outside this
//!    plane's storage entirely, and the display snapshot it captures
//!    is the same captured-not-referenced value as the stamp above.
//!
//! Three mechanisms can answer, and which one is chosen is a decision
//! rather than a default: **masking at write** (the stamp records an
//! id and no name, which costs every reader the ability to say who
//! without a join, and costs history the name the account had then),
//! **retention under a documented exemption** (the record is kept and
//! the basis for keeping it is written down, which is what an audit
//! log is usually held under), and **crypto-shredding** (the name is
//! stored encrypted under a per-subject key and erasure destroys the
//! key, which turns a rewrite into a delete somewhere the append-only
//! rule does not reach).
//!
//! **The order matters, and this end of it comes first.** Today a row
//! could be rewritten under a migration if the decision demanded it —
//! expensive and against the schema's triggers, but possible. A
//! tamper-evidence chain removes that: once each entry commits to its
//! predecessor, rewriting one invalidates every entry after it, and
//! erasure-by-rewriting is gone permanently rather than merely
//! discouraged. So the mechanism is settled before a chain starts, not
//! after — a chain built first would decide this question by making it
//! unanswerable.

use crate::domain::identity::LedgerActor;
use crate::domain::store;
use crate::error::DomainError;
use uuid::Uuid;

/// Storage-assigned position of an event within its team's stream —
/// monotonic, starting at 1.
///
/// A newtype rather than a bare `i64` so "validated but not generated"
/// is a property of the type: the only constructor checks the value a
/// storage row handed back, and there is no `next()` for domain code
/// to invent sequence numbers with.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "i64", into = "i64")]
pub struct EventSeq(i64);

impl EventSeq {
    /// Accepts a storage-assigned sequence number. Zero and negatives
    /// are refused: no storage scheme this plane admits produces them,
    /// so one arriving means a corrupted read, not a first event.
    pub fn new(raw: i64) -> Result<Self, DomainError> {
        if raw < 1 {
            return Err(DomainError::Validation(format!(
                "event seq {raw} is not a storage-assigned position (positions start at 1)"
            )));
        }
        Ok(Self(raw))
    }

    /// The raw position.
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl TryFrom<i64> for EventSeq {
    type Error = DomainError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<EventSeq> for i64 {
    fn from(value: EventSeq) -> Self {
        value.get()
    }
}

/// A namespaced + versioned event kind — `"teams.membership.added/1"`.
///
/// The shape is `<segment>.<segment>[.<segment>…]/<version>`: at least
/// two dot-separated lowercase segments (`[a-z0-9_]`), then `/`, then
/// a positive integer with no leading zeros. The namespace requirement
/// is what lets `forge.*` kinds land beside `teams.*` ones after #63
/// without collisions; the version is in the *name* because the
/// payload contract is per kind-version, and a reader that knows
/// `…/1` must not be handed a `…/2` body under the same label.
///
/// [`EventKind::parse`] validates the shape and nothing more — whether
/// a kind is *registered* is [`is_v0_kind`]'s question, kept separate
/// so a future kind's events can be carried by an envelope that
/// predates it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EventKind(String);

impl EventKind {
    /// Parses and validates the `namespace.name/version` shape.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let (path, version) = raw.split_once('/').ok_or_else(|| {
            DomainError::Validation(format!(
                "event kind {raw:?} carries no version (expected \"ns.name/N\")"
            ))
        })?;

        let segments: Vec<&str> = path.split('.').collect();
        if segments.len() < 2 {
            return Err(DomainError::Validation(format!(
                "event kind {raw:?} is not namespaced (expected at least \
                 \"namespace.name\" before the version)"
            )));
        }
        for segment in &segments {
            let well_formed = !segment.is_empty()
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !well_formed {
                return Err(DomainError::Validation(format!(
                    "event kind {raw:?} has a malformed segment {segment:?} \
                     (lowercase [a-z0-9_], non-empty)"
                )));
            }
        }

        let parsed: u32 = version.parse().map_err(|_| {
            DomainError::Validation(format!(
                "event kind {raw:?} has a non-numeric version {version:?}"
            ))
        })?;
        // `parse` accepts "01" and "+1"; the round-trip check refuses
        // every spelling but the canonical one, so one kind-version has
        // one string and string equality is kind equality.
        if parsed < 1 || version != parsed.to_string() {
            return Err(DomainError::Validation(format!(
                "event kind {raw:?} has version {version:?}; versions are \
                 positive integers written canonically (1, 2, …)"
            )));
        }

        Ok(Self(raw.to_string()))
    }

    /// The full storage form, e.g. `"teams.membership.added/1"`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EventKind {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<EventKind> for String {
    fn from(value: EventKind) -> Self {
        value.0
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A team came into existence.
pub const TEAM_CREATED: &str = "teams.team.created/1";
/// A team was deleted (owner, or an admin — ledger-stamped).
pub const TEAM_DELETED: &str = "teams.team.deleted/1";
/// A user became a member.
pub const MEMBERSHIP_ADDED: &str = "teams.membership.added/1";
/// A member left or was removed.
pub const MEMBERSHIP_REMOVED: &str = "teams.membership.removed/1";
/// A member's role changed — the payload carries **both** the old and
/// the new value (#83 §1: role changes are first-class events carrying
/// old/new), so the entry reads on its own instead of against its
/// predecessors.
pub const ROLE_CHANGED: &str = "teams.membership.role_changed/1";
/// A promotion's blob copy completed — declared digest verified,
/// bytes in the CAS, link row landing in the same tx (#83 §3).
pub const BLOB_COPY_COMPLETED: &str = "teams.blob.copy_completed/1";
/// A team's blob link was marked for purge (#83 §3 lifecycle, the #95
/// slice): the first half of the trash→purge two-step. The link is
/// hidden from normal reads for the grace window; the payload carries
/// the digest and when the mark landed.
pub const BLOB_LINK_PURGE_MARKED: &str = "teams.blob_link.purge_marked/1";
/// A purge mark was lifted during the grace window — the link is
/// restored intact. The payload carries the digest and the mark it
/// undid.
pub const BLOB_LINK_PURGE_UNMARKED: &str = "teams.blob_link.purge_unmarked/1";
/// Marked links whose grace window elapsed were reclaimed — the second
/// half of the two-step, and the only path that removes links for
/// reclaim's sake. The record survives, the bytes go (the zero-link
/// sweep collects them); the payload carries the digests removed and
/// the window they waited out.
pub const BLOB_LINK_RECLAIMED: &str = "teams.blob_link.reclaimed/1";

/// The v0 kind registry: team lifecycle, membership changes, role
/// changes, blob-copy completed, and the purge two-step. A slice
/// rather than knowledge spread over call sites, for the reason
/// `asterism-core` keeps `RESERVED_VALUES` as a list — whoever needs
/// "every kind v0 ships" (a projection, a migration, a doc generator)
/// walks this, and a kind added later reaches them without an edit on
/// their side.
pub const V0_KINDS: &[&str] = &[
    TEAM_CREATED,
    TEAM_DELETED,
    MEMBERSHIP_ADDED,
    MEMBERSHIP_REMOVED,
    ROLE_CHANGED,
    BLOB_COPY_COMPLETED,
    BLOB_LINK_PURGE_MARKED,
    BLOB_LINK_PURGE_UNMARKED,
    BLOB_LINK_RECLAIMED,
];

/// Whether `kind` is one this build of the plane writes. Shape and
/// registration are separate questions on purpose: a reader must
/// accept well-formed kinds it does not know (a stream written by a
/// newer build, `forge.*` after #63), a *writer* asks this before
/// appending.
pub fn is_v0_kind(kind: &EventKind) -> bool {
    V0_KINDS.contains(&kind.as_str())
}

/// What a forge handle stands for — the four kinds #102 fixed for the
/// local plane's `forge_actor` rows.
///
/// `owner` and `subject` are the two an author has; `unrecorded` is a
/// write that named nobody, which is one actor rather than a fresh one
/// each time; `server` is the instance itself, which is what a line's
/// rule writes as. The teams plane spells them the same way because it
/// is referring to the same rows, not because it re-derives the
/// vocabulary: a word this list does not hold is refused at the
/// boundary rather than carried as a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForgeStandsFor {
    /// The owner of the instance the handle was minted on.
    Owner,
    /// A named subject, identified by the token beside it.
    Subject,
    /// A write that named nobody.
    Unrecorded,
    /// The instance itself, acting as a line's rule.
    Server,
}

impl ForgeStandsFor {
    /// The TEXT form — the word the local plane's `stands_for` column
    /// holds.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Subject => "subject",
            Self::Unrecorded => "unrecorded",
            Self::Server => "server",
        }
    }
}

impl std::fmt::Display for ForgeStandsFor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A forge handle, as a ledger subject: what it stands for, and whom
/// when that is a named subject.
///
/// The pair, not an opaque string. #102 fixed the vocabulary the local
/// plane's `forge_actor` rows are keyed by, so there is a shape to
/// validate and this type validates it: `subject` is present exactly
/// when [`ForgeStandsFor::Subject`] is the kind — the same rule the
/// table states as a `CHECK`, and a pair that breaks it is refused
/// here rather than stored and puzzled over later.
///
/// The subject token is TEXT rather than a UUID because that is what
/// the column holds: the local plane's authors are named by the same
/// token its sharing lists and viewers carry, which is opaque to both
/// planes and is not a uuid.
///
/// The canonical string is what crosses the wire and what the
/// `ledger_subject` index is keyed by, so a trace query encodes a
/// handle exactly as an append wrote it: the bare word for the three
/// kinds that name nobody, and `subject:<token>` for the one that
/// does.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ForgeIdentityRef {
    stands_for: ForgeStandsFor,
    subject: Option<String>,
}

impl ForgeIdentityRef {
    /// The instance owner's handle.
    pub const fn owner() -> Self {
        Self {
            stands_for: ForgeStandsFor::Owner,
            subject: None,
        }
    }

    /// The handle for writes that named nobody.
    pub const fn unrecorded() -> Self {
        Self {
            stands_for: ForgeStandsFor::Unrecorded,
            subject: None,
        }
    }

    /// The instance's own handle.
    pub const fn server() -> Self {
        Self {
            stands_for: ForgeStandsFor::Server,
            subject: None,
        }
    }

    /// A named subject's handle. A blank token is refused: the whole
    /// point of this kind is that it says whom, and one that does not
    /// is [`Self::unrecorded`] wearing the wrong word.
    pub fn subject(token: impl Into<String>) -> Result<Self, DomainError> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(DomainError::Validation(
                "a forge subject handle names whom; use the unrecorded kind for a \
                 write that named nobody"
                    .into(),
            ));
        }
        Ok(Self {
            stands_for: ForgeStandsFor::Subject,
            subject: Some(token),
        })
    }

    /// Parses the canonical string form.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        if let Some((kind, token)) = raw.split_once(':') {
            if kind != ForgeStandsFor::Subject.as_str() {
                return Err(DomainError::Validation(format!(
                    "forge identity {raw:?} names a subject after {kind:?}; only \
                     \"subject\" takes one"
                )));
            }
            return Self::subject(token);
        }
        match raw {
            "owner" => Ok(Self::owner()),
            "unrecorded" => Ok(Self::unrecorded()),
            "server" => Ok(Self::server()),
            "subject" => Err(DomainError::Validation(
                "forge identity \"subject\" names nobody (expected \"subject:<token>\")".into(),
            )),
            other => Err(DomainError::Validation(format!(
                "forge identity {other:?} is not one of \"owner\" / \"subject:<token>\" / \
                 \"unrecorded\" / \"server\""
            ))),
        }
    }

    /// What this handle stands for.
    pub const fn stands_for(&self) -> ForgeStandsFor {
        self.stands_for
    }

    /// The subject token, present exactly when the kind is
    /// [`ForgeStandsFor::Subject`].
    pub fn subject_token(&self) -> Option<&str> {
        self.subject.as_deref()
    }

    /// The canonical string form — what storage and the wire carry.
    pub fn encode(&self) -> String {
        match &self.subject {
            Some(token) => format!("{}:{token}", self.stands_for),
            None => self.stands_for.as_str().to_string(),
        }
    }
}

impl TryFrom<String> for ForgeIdentityRef {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ForgeIdentityRef> for String {
    fn from(value: ForgeIdentityRef) -> Self {
        value.encode()
    }
}

impl std::fmt::Display for ForgeIdentityRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.encode())
    }
}

/// A typed reference an event makes — the index trace queries walk, so
/// "which events touched X" never becomes payload parsing (#83 §2).
///
/// The constructors validate where a shape exists to validate
/// ([`SubjectRef::digest`] / [`SubjectRef::blob`]); the enum's payloads
/// stay public because a tagged serde representation needs them, so
/// construction through the constructors is the convention the tests
/// pin, not a wall the type enforces.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "ref_type", content = "value", rename_all = "snake_case")]
pub enum SubjectRef {
    /// A content digest (`sha256:` form, `asterism-core` notation).
    Digest(String),
    /// A forge handle, in the vocabulary #102 fixed
    /// ([`ForgeIdentityRef`]).
    ForgeIdentity(ForgeIdentityRef),
    /// A user.
    User(Uuid),
    /// A stored blob, addressed the only way blobs are: by digest.
    Blob(String),
}

impl SubjectRef {
    /// A digest subject, validated against the shared notation.
    pub fn digest(raw: &str) -> Result<Self, DomainError> {
        Ok(Self::Digest(store::parse_digest(raw)?))
    }

    /// A blob subject — digest-addressed, same validation.
    pub fn blob(raw: &str) -> Result<Self, DomainError> {
        Ok(Self::Blob(store::parse_digest(raw)?))
    }

    /// A user subject.
    pub const fn user(user_id: Uuid) -> Self {
        Self::User(user_id)
    }

    /// A forge-handle subject.
    pub const fn forge_identity(handle: ForgeIdentityRef) -> Self {
        Self::ForgeIdentity(handle)
    }
}

/// One entry in a team's stream — the envelope, with the payload
/// opaque to it.
///
/// Fields are public: the envelope is a record, and what makes it
/// trustworthy is not encapsulation here but the storage discipline
/// (append-only, no update/delete) that `teams-infra` owes. The
/// constructor exists so that every envelope that passes through
/// domain code has been shape-checked once.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LedgerEvent {
    /// Storage-assigned position within the team's stream.
    pub seq: EventSeq,
    /// Globally unique id of this event.
    pub event_id: Uuid,
    /// The stream — team boundary only; private-space operations never
    /// land in any team's ledger (#83 §2).
    pub team_id: Uuid,
    /// Who acted, stamped at write time and distinguishable as member
    /// or admin ([`LedgerActor`]).
    pub actor: LedgerActor,
    /// When, as milliseconds since the Unix epoch.
    pub occurred_at_ms: i64,
    /// What happened — namespaced + versioned ([`EventKind`]).
    pub kind: EventKind,
    /// What it happened *to* — the typed refs an index is built over.
    pub subjects: Vec<SubjectRef>,
    /// The kind-versioned body. Opaque to the substrate: this crate
    /// neither reads nor validates it beyond being JSON.
    pub payload: serde_json::Value,
}

impl LedgerEvent {
    /// Assembles an envelope from parts that already carry their own
    /// validation (`seq` and `kind` are parsed types) plus the one
    /// check nothing else owns: `occurred_at_ms` must not predate the
    /// epoch — a negative timestamp is a serialization accident, not a
    /// time.
    #[allow(clippy::too_many_arguments)] // The envelope *is* these eight fields (#83 §2); grouping them would invent a ninth name.
    pub fn new(
        seq: EventSeq,
        event_id: Uuid,
        team_id: Uuid,
        actor: LedgerActor,
        occurred_at_ms: i64,
        kind: EventKind,
        subjects: Vec<SubjectRef>,
        payload: serde_json::Value,
    ) -> Result<Self, DomainError> {
        if occurred_at_ms < 0 {
            return Err(DomainError::Validation(format!(
                "occurred_at_ms {occurred_at_ms} predates the epoch"
            )));
        }
        Ok(Self {
            seq,
            event_id,
            team_id,
            actor,
            occurred_at_ms,
            kind,
            subjects,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::identity::{ActorStamp, LedgerActor};

    #[test]
    fn every_v0_kind_parses_and_registers() {
        for raw in V0_KINDS {
            let kind = EventKind::parse(raw)
                .unwrap_or_else(|e| panic!("registered kind {raw} must parse: {e}"));
            assert!(is_v0_kind(&kind));
            assert_eq!(kind.as_str(), *raw);
        }
    }

    #[test]
    fn a_kind_needs_a_namespace_and_a_canonical_version() {
        // A well-formed kind this build does not register still
        // parses — shape and registration are different questions.
        let foreign = EventKind::parse("forge.identity.linked/1").unwrap();
        assert!(!is_v0_kind(&foreign));

        for invalid in [
            "teams.membership.added",    // no version
            "added/1",                   // no namespace
            "teams..added/1",            // empty segment
            "Teams.membership.added/1",  // uppercase
            "teams.membership.added/0",  // versions start at 1
            "teams.membership.added/01", // non-canonical spelling
            "teams.membership.added/+1", // non-canonical spelling
            "teams.membership.added/one",
            "teams.membership added/1", // space
            "/1",
            "",
        ] {
            assert!(
                matches!(EventKind::parse(invalid), Err(DomainError::Validation(_))),
                "{invalid:?} must not parse as an event kind"
            );
        }
    }

    #[test]
    fn seq_is_validated_never_generated() {
        assert_eq!(EventSeq::new(1).unwrap().get(), 1);
        assert!(EventSeq::new(0).is_err());
        assert!(EventSeq::new(-5).is_err());
        // There is deliberately no `next()` to assert the absence of;
        // what this pins is that the boundary value storage would
        // never assign is refused rather than carried.
    }

    #[test]
    fn an_envelope_assembles_from_validated_parts() {
        let actor = LedgerActor::member(ActorStamp {
            user_id: Uuid::now_v7(),
            display_name: "Hoshino".into(),
        });
        let kind = EventKind::parse(ROLE_CHANGED).unwrap();
        let subject = SubjectRef::user(Uuid::now_v7());

        let event = LedgerEvent::new(
            EventSeq::new(7).unwrap(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            actor.clone(),
            1_755_000_000_000,
            kind.clone(),
            vec![subject],
            // Old + new both in the payload — the entry reads on its
            // own. The envelope does not inspect this; the shape is
            // the kind's contract.
            serde_json::json!({ "old": "member", "new": "owner" }),
        )
        .unwrap();
        assert_eq!(event.kind, kind);

        assert!(
            LedgerEvent::new(
                EventSeq::new(1).unwrap(),
                Uuid::now_v7(),
                Uuid::now_v7(),
                actor,
                -1,
                kind,
                vec![],
                serde_json::Value::Null,
            )
            .is_err()
        );
    }

    #[test]
    fn a_forge_handle_is_a_pair_and_round_trips_through_its_canonical_string() {
        let handles = [
            ForgeIdentityRef::owner(),
            ForgeIdentityRef::unrecorded(),
            ForgeIdentityRef::server(),
            ForgeIdentityRef::subject("hoshino").unwrap(),
        ];
        for handle in &handles {
            assert_eq!(ForgeIdentityRef::parse(&handle.encode()).unwrap(), *handle);
        }

        // The three that name nobody encode as the bare word; the one
        // that names somebody carries them after a colon.
        assert_eq!(ForgeIdentityRef::owner().encode(), "owner");
        assert_eq!(
            ForgeIdentityRef::subject("hoshino").unwrap().encode(),
            "subject:hoshino"
        );

        // A token is opaque text, not a uuid, and a colon inside one
        // survives: the split is on the first, so the rest is the
        // token whatever it holds.
        let colonful = ForgeIdentityRef::subject("scheme:opaque").unwrap();
        assert_eq!(colonful.subject_token(), Some("scheme:opaque"));
        assert_eq!(
            ForgeIdentityRef::parse(&colonful.encode()).unwrap(),
            colonful
        );

        // The pair rule the local plane states as a CHECK: a token
        // belongs to the subject kind and to no other.
        assert_eq!(ForgeIdentityRef::owner().subject_token(), None);
        for invalid in [
            "subject",          // names nobody
            "subject:",         // the same, spelled longer
            "owner:hoshino",    // a kind that takes no token
            "server:something", // likewise
            "admin",            // not in the #102 vocabulary
            "",
        ] {
            assert!(
                matches!(
                    ForgeIdentityRef::parse(invalid),
                    Err(DomainError::Validation(_))
                ),
                "{invalid:?} must not parse as a forge handle"
            );
        }
        assert!(ForgeIdentityRef::subject("   ").is_err());
    }

    #[test]
    fn a_forge_subject_serialises_as_the_string_the_index_is_keyed_by() {
        // The index column is TEXT and the wire value is a string, so
        // the typed pair must not serialise as an object — the whole
        // point of the canonical encoding is that a trace query and an
        // append spell one handle one way.
        let subject = SubjectRef::forge_identity(ForgeIdentityRef::subject("hoshino").unwrap());
        let json = serde_json::to_value(&subject).unwrap();
        assert_eq!(json["ref_type"], serde_json::json!("forge_identity"));
        assert_eq!(json["value"], serde_json::json!("subject:hoshino"));
        assert_eq!(serde_json::from_value::<SubjectRef>(json).unwrap(), subject);
    }

    #[test]
    fn digest_subjects_carry_the_shared_notation() {
        let digest = asterism_core::domain::content_hash::of_bytes(b"star");
        assert!(SubjectRef::digest(&digest).is_ok());
        assert!(SubjectRef::blob(&digest).is_ok());

        // A bare hex string is not a digest in this workspace's
        // notation, on either digest-shaped variant.
        for wrong in ["a1b2c3", "cr1-sha256:", ""] {
            assert!(SubjectRef::digest(wrong).is_err());
            assert!(SubjectRef::blob(wrong).is_err());
        }
    }
}
