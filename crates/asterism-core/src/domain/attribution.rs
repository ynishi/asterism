//! Attribution — *who* a record is by, *what* operated on their behalf,
//! and *through which channel* that answer arrived.
//!
//! This module is the source of truth for how a write is attributed. One
//! triple travels together: `(author, operator, via)`. It is carried on
//! the write path by [`AttributionContext`] and restored from the stored
//! columns by [`PersistedAttribution`]; the individual values are
//! [`Author`], [`OperatorRef`] and [`AttributionChannel`].
//!
//! - [`Author`] — the subject a record is attributed to. The write-side
//!   mirror of [`Viewer`](crate::domain::value::Viewer): the reader asks
//!   "who is looking", this answers "who was writing", and both name a
//!   subject with the same token.
//! - [`OperatorRef`] — the agent that performed the operation
//!   (`claude-code`, `codex`, `asterism-ui`, …). Under a single
//!   authenticated subject the interesting question is often not *who*
//!   but *through what*, and one field cannot hold both answers.
//! - [`AttributionChannel`] — the channel the pair above arrived
//!   through. Derived from the entry point, never asserted.
//!
//! # Five roles, of which two are attribution
//!
//! Several types in this codebase name somebody. Only the first two
//! answer "whose write is this":
//!
//! | concept | role | attribution |
//! |---|---|---|
//! | **author** ([`Author`]) | whose write this is — a person or a subject on a service | first axis |
//! | **operator** ([`OperatorRef`]) | the agent that carried the operation out | second axis |
//! | **register** (persona) | a choice of voice at presentation time — [`CommentAuthor::Persona`](crate::domain::asset_comment::CommentAuthor::Persona), [`thread::Author::Persona`](crate::domain::thread::Author::Persona). Same sense as [`Asset::register_note`](crate::domain::asset::Asset::register_note) ("the asset's register / tone"), not the *registry* sense the word carries elsewhere in this codebase | out of scope — speaking in a voice is not authorship |
//! | **the persona an asset belongs to** ([`Asset::persona_id`](crate::domain::asset::Asset::persona_id)) | every asset belongs to exactly one persona. Membership says nothing about who wrote it | out of scope — the persona shown on a card is where the asset is filed, not who made it |
//! | **transcript role** (`ChatRole` in `asterism-importer-sdk`) | a role written *inside* imported conversation material (`user` / `assistant` / …). A fact about the content | out of scope — the `user` of an imported chat is not necessarily this instance's owner |
//!
//! This is why [`Author::parse`] refuses a `"persona"` kind: a persona
//! is a voice something can be said in, and a place an asset belongs to.
//! Neither is a subject a write is attributed to.
//!
//! The mapping for [`thread::Author`](crate::domain::thread::Author),
//! which folds "human vs agent" into one enum, is: `Human` ≈
//! `(author = Owner, operator = none)`, `ClaudeCode` / `Agent(s)` ≈
//! `(author = unrecorded, operator = s)`, `Persona` ≈ register.
//! [`CommentAuthor::User`](crate::domain::asset_comment::CommentAuthor::User)
//! is the comment-side alias of [`Author::Owner`]. Unifying the types
//! is deliberately not attempted here — settling which value means
//! what comes first.
//!
//! # Who the owner is
//!
//! [`Author::Owner`] is an indirect reference to the single
//! [`InstanceIdentity`](crate::domain::instance::InstanceIdentity) row:
//! one profile database has exactly one owner. Today
//! `instance.owner_subject` is unbound, so `Owner` reads as "whoever
//! this instance belongs to" and resolves to no token
//! ([`InstanceIdentity::resolve_owner`](crate::domain::instance::InstanceIdentity::resolve_owner)).
//! Authentication binds the subject once, and only then does `Owner`
//! resolve to a name. Sharing adds subjects; it never adds owners.
//!
//! Author subjects and viewer (sharing) subjects are **one namespace**.
//! "shared with alice" and "written by alice" must be the same alice,
//! or a hosted deployment cannot reconcile who may look with who wrote.
//!
//! # `None` means unrecorded
//!
//! An absent author is **not** "authored by the owner". Defaulting to
//! the owner would make the assertion and the default indistinguishable
//! the moment a second subject exists, which is the state a hosted
//! migration would have to un-guess. The absence is the same kind of
//! absence as [`content_hash`](crate::domain::content_hash) `NULL`: a
//! question nobody has answered yet, not a value. Splitting "operated
//! by a human directly" out of "unrecorded" is not modelled today; it
//! can land later as a reserved marker (the shape
//! [`UNHASHABLE`](crate::domain::content_hash::UNHASHABLE) uses)
//! without disturbing the columns.
//!
//! # The same shape as `_trace.source`, on a different subject
//!
//! [`provenance::source`](crate::domain::provenance::source) records
//! which channel a **provenance claim** arrived through (embedded /
//! pushed / manual). [`AttributionChannel`] records which channel an
//! **attribution** arrived through. Both are bookkeeping of arrival,
//! derived from the entry point rather than asserted; they are not two
//! answers to one question, they are the same question asked about two
//! different values, and one request can legitimately write both (a
//! provenance verb stamps `source = manual` on the claim and
//! `via = asserted` on the operator it records).

use crate::error::DomainError;

/// The subject a record is attributed to.
///
/// Write-side mirror of [`Viewer`](crate::domain::value::Viewer). The
/// wire / column form is a pair — a kind slug plus an optional subject
/// token — following the split
/// [`CommentAuthor`](crate::domain::asset_comment::CommentAuthor)
/// already uses:
///
/// | kind | subject | meaning |
/// |---|---|---|
/// | `owner` | absent | the owner of this Asterism instance |
/// | `subject` | present | a named subject, the same token
///   `Visibility::Restricted.sharing` and `Viewer::Subject` carry |
///
/// Any other combination is a corrupt pair and is rejected at the
/// mapping boundary ([`Author::parse`]) rather than degrading into a
/// plausible-looking value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Author {
    /// The owner of this Asterism instance — an indirect reference to
    /// the single
    /// [`InstanceIdentity`](crate::domain::instance::InstanceIdentity)
    /// row, not a token of its own. Unbound today; authentication binds
    /// `instance.owner_subject` once and the reference resolves from
    /// then on (see the module docs).
    Owner,
    /// A named subject, identified by the same token used in
    /// [`Viewer::Subject`](crate::domain::value::Viewer::Subject) and in
    /// the sharing list of
    /// [`Visibility::Restricted`](crate::domain::value::Visibility::Restricted).
    Subject(String),
}

impl Author {
    /// Slug used on the wire and in the `author_kind` column
    /// (`"owner"` / `"subject"`).
    pub fn kind_slug(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Subject(_) => "subject",
        }
    }

    /// Subject token when the author is a named subject.
    pub fn subject(&self) -> Option<&str> {
        match self {
            Self::Owner => None,
            Self::Subject(subject) => Some(subject),
        }
    }

    /// Splits the author into its stored pair — `(kind, subject)`.
    /// Inverse of [`Author::parse`].
    pub fn encode(&self) -> (&'static str, Option<&str>) {
        (self.kind_slug(), self.subject())
    }

    /// Rebuilds an author from a stored `(kind, subject)` pair.
    ///
    /// Rejects every combination [`Author::encode`] cannot produce — an
    /// owner carrying a subject, a subject without one (or with a blank
    /// one), and an unknown kind — so a bad row surfaces here instead of
    /// silently becoming the owner. SQLite cannot express this
    /// constraint on a column added by `ALTER TABLE`, which is why the
    /// check lives in the domain.
    ///
    /// `"persona"` lands in the unknown-kind arm on purpose: a persona
    /// is a voice something can be said in, and a place an asset belongs
    /// to — not a subject a write is attributed to (module docs,
    /// five-role table).
    pub fn parse(kind: &str, subject: Option<&str>) -> Result<Self, DomainError> {
        match (kind, subject) {
            ("owner", None) => Ok(Self::Owner),
            ("owner", Some(subject)) => Err(DomainError::Validation(format!(
                "author kind \"owner\" carries no subject, got {subject:?}"
            ))),
            ("subject", Some(subject)) if !subject.trim().is_empty() => {
                Ok(Self::Subject(subject.to_string()))
            }
            ("subject", Some(_)) => Err(DomainError::Validation(
                "author kind \"subject\" requires a non-empty subject".into(),
            )),
            ("subject", None) => Err(DomainError::Validation(
                "author kind \"subject\" requires a subject".into(),
            )),
            (other, _) => Err(DomainError::Validation(format!(
                "unknown author kind: {other:?}"
            ))),
        }
    }

    /// Reads the nullable column pair: both absent means **unrecorded**
    /// (`Ok(None)`), a subject without a kind is a half-written row and
    /// is rejected.
    pub fn from_columns(
        kind: Option<&str>,
        subject: Option<&str>,
    ) -> Result<Option<Self>, DomainError> {
        match (kind, subject) {
            (None, None) => Ok(None),
            (None, Some(subject)) => Err(DomainError::Validation(format!(
                "author subject {subject:?} without an author kind"
            ))),
            (Some(kind), subject) => Self::parse(kind, subject).map(Some),
        }
    }
}

/// The agent that performed an operation — `claude-code`, `codex`,
/// `asterism-ui`, an importer's own name.
///
/// An **open** slug on purpose: the set of things that can drive
/// Asterism is not closed, and a closed enum would force every new
/// client to ship a migration before it could say what it is. Only
/// emptiness is rejected, the same bar
/// [`SourceRef::new`](crate::domain::value::SourceRef::new) sets for a
/// locator — a blank operator is an assertion that says nothing, and it
/// must not be storable as one that does.
///
/// Caller-asserted; see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OperatorRef(String);

impl OperatorRef {
    /// Builds the reference, rejecting empty / whitespace input.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DomainError::Validation(
                "OperatorRef must not be empty".into(),
            ));
        }
        Ok(Self(value))
    }

    /// Returns the underlying slug.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OperatorRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The channel an attribution arrived through.
///
/// **Structurally derived, never asserted.** A caller cannot state its
/// own channel — the value comes from *which entry point ran*, which is
/// the only thing that makes the difference between "the owner's own
/// app said so" and "an HTTP client said so" survive into the row. The
/// enforcement is the type: [`AttributionContext`] has private fields,
/// named constructors, no `via` setter, and no serialisation
/// derive, so the value cannot be carried in from the wire. The channel
/// travels *out* (the read-side `attributed_via` field) but never *in*
/// — no command carries a `via` field.
///
/// Without this column an authenticated deployment cannot tell an
/// authenticated author from a caller that simply claimed one, so every
/// row written before authentication would have to be treated as
/// unresolvable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttributionChannel {
    /// An operation surface that belongs to the owner (the Tauri IPC of
    /// the desktop app). The owner-ness is a property of the surface,
    /// not a guess about the caller: this app's controls are the
    /// owner's. Prefixed `owner-` because "surface" alone is a general
    /// word for "entry point" throughout the codebase.
    OwnerSurface,
    /// A caller stating its own author / operator over HTTP or MCP —
    /// including the importer SDK, which is an HTTP client rather than
    /// an in-process injection point. Self-assertion, believed but
    /// labelled as such.
    Asserted,
    /// Established by an authentication layer. Reserved for the auth
    /// wave; nothing produces it yet.
    Authenticated,
}

impl AttributionChannel {
    /// Slug used on the wire and in the `attributed_via` column.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::OwnerSurface => "owner-surface",
            Self::Asserted => "asserted",
            Self::Authenticated => "authenticated",
        }
    }

    /// Rebuilds the channel from its stored slug.
    ///
    /// An unknown slug is a corrupt row rather than a channel this
    /// build has not heard of — the same bar [`Author::parse`] sets.
    pub fn parse(slug: &str) -> Result<Self, DomainError> {
        match slug {
            "owner-surface" => Ok(Self::OwnerSurface),
            "asserted" => Ok(Self::Asserted),
            "authenticated" => Ok(Self::Authenticated),
            other => Err(DomainError::Validation(format!(
                "unknown attribution channel: {other:?}"
            ))),
        }
    }

    /// Reads the nullable column: absent means the channel was not
    /// recorded, which is the shape every row written before the column
    /// existed carries.
    pub fn from_column(slug: Option<&str>) -> Result<Option<Self>, DomainError> {
        slug.map(Self::parse).transpose()
    }
}

/// An attribution triple restored from the stored columns.
///
/// The only publicly constructible form of a triple that carries a
/// channel. [`AttributionContext`] can be built by name (and each name
/// fixes its own channel), so the sole way to obtain an arbitrary
/// `(author, operator, via)` combination is to read one back from the
/// database through [`from_columns`](Self::from_columns) — which is
/// exactly the boundary where such a combination is a fact rather than
/// a claim.
///
/// Legacy rows are accepted verbatim: an author or an operator with no
/// channel is what the columns that predate `attributed_via` hold, and
/// refusing to read them would make old rows unreadable rather than
/// honest. All three absent is not legacy at all — it is an ordinary
/// unrecorded row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedAttribution {
    author: Option<Author>,
    operator_ai: Option<OperatorRef>,
    attributed_via: Option<AttributionChannel>,
}

impl PersistedAttribution {
    /// Infallible constructor over already-validated values. Private to
    /// the domain: it is how a value that has *been* through validation
    /// (an in-memory entity handing its own attribution back out) is
    /// rebuilt without pretending to re-parse it — the same asymmetry
    /// [`Author::encode`] has against [`Author::parse`].
    pub(in crate::domain) fn recorded(
        author: Option<Author>,
        operator_ai: Option<OperatorRef>,
        attributed_via: Option<AttributionChannel>,
    ) -> Self {
        Self {
            author,
            operator_ai,
            attributed_via,
        }
    }

    /// Reads the four stored columns.
    ///
    /// Each value is validated by its own type ([`Author::from_columns`],
    /// [`OperatorRef::new`], [`AttributionChannel::from_column`]), so a
    /// corrupt row surfaces here instead of degrading into a
    /// plausible-looking triple.
    pub fn from_columns(
        author_kind: Option<&str>,
        author_subject: Option<&str>,
        operator_ai: Option<&str>,
        attributed_via: Option<&str>,
    ) -> Result<Self, DomainError> {
        Ok(Self::recorded(
            Author::from_columns(author_kind, author_subject)?,
            operator_ai.map(OperatorRef::new).transpose()?,
            AttributionChannel::from_column(attributed_via)?,
        ))
    }

    /// Subject the record is attributed to (`None` = unrecorded).
    pub fn author(&self) -> Option<&Author> {
        self.author.as_ref()
    }

    /// Agent that performed the operation (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel the pair arrived through (`None` on rows written before
    /// the column existed, and on rows that record nobody).
    pub fn attributed_via(&self) -> Option<AttributionChannel> {
        self.attributed_via
    }
}

/// The attribution a write carries — request-scoped, chosen by the
/// adapter that received the request.
///
/// Lives in the domain rather than the application layer because
/// domain entities take it as a construction argument, and a domain
/// type cannot depend on an application one (`lib.rs`, dependency
/// inversion). Its request-scoped lifetime is a property of how it is
/// used, not of where it is declared.
///
/// Four rules hold the design together, and each is enforced by shape
/// rather than by discipline:
///
/// 1. **The channel is derived.** Private fields, named constructors,
///    no `via` setter, no `Serialize` / `Deserialize` /
///    `SchemaBridge` derive — the value cannot cross the wire inward.
///    Picking a constructor *is* stating which entry point you are.
/// 2. **An assertion cannot claim the owner.** [`asserted`](Self::asserted)
///    rejects [`Author::Owner`]: being the owner follows from the
///    surface (`owner_surface`) or from authentication, never from
///    saying so.
/// 3. **Unrecorded is a pair, in both directions.** No author and no
///    operator means no channel either, so
///    `asserted(None, None)` is the same value as
///    [`unrecorded`](Self::unrecorded). The other direction — a
///    recorded author or operator always carries a channel — is
///    enforced at the write boundary in the repository.
/// 4. **A system write records nobody.** Background jobs and sweeps use
///    [`unrecorded`](Self::unrecorded): that the app was running is not
///    a subject a write can be attributed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionContext {
    author: Option<Author>,
    operator_ai: Option<OperatorRef>,
    attributed_via: Option<AttributionChannel>,
}

impl AttributionContext {
    /// The owner's own operation surface — the desktop app's IPC.
    ///
    /// Fixed to `(Owner, unrecorded operator, OwnerSurface)`. It takes
    /// no arguments on purpose: this surface knows exactly one author
    /// (the owner) and cannot carry a subject, because a subject
    /// arriving here would be a claim, and claims are
    /// [`asserted`](Self::asserted).
    pub fn owner_surface() -> Self {
        Self {
            author: Some(Author::Owner),
            operator_ai: None,
            attributed_via: Some(AttributionChannel::OwnerSurface),
        }
    }

    /// A caller's self-stated attribution (HTTP / MCP / the importer
    /// SDK).
    ///
    /// Rejects [`Author::Owner`] (rule 2): owner-ness is not something
    /// a caller can state. Stating nothing at all is not an error — it
    /// yields the same value as [`unrecorded`](Self::unrecorded)
    /// (rule 3), because a channel with nothing to attribute records
    /// nothing.
    pub fn asserted(
        author: Option<Author>,
        operator_ai: Option<OperatorRef>,
    ) -> Result<Self, DomainError> {
        if matches!(author, Some(Author::Owner)) {
            return Err(DomainError::Validation(
                "an asserted author cannot be the owner: owner-ness comes from the \
                 owner's own surface or from authentication, not from the claim"
                    .into(),
            ));
        }
        if author.is_none() && operator_ai.is_none() {
            return Ok(Self::unrecorded());
        }
        Ok(Self {
            author,
            operator_ai,
            attributed_via: Some(AttributionChannel::Asserted),
        })
    }

    /// Records nobody — the value background jobs and sweeps write
    /// (rule 4).
    ///
    /// Named for the vocabulary the columns already use ("`None` means
    /// unrecorded"); `system` was avoided because it already names a
    /// message role in two other enums. Crate-visible so that the
    /// choice stays inside `asterism-core`, where the jobs and sweeps
    /// that legitimately record nobody live — an adapter cannot reach
    /// for it to make an awkward attribution question go away.
    pub(crate) fn unrecorded() -> Self {
        Self {
            author: None,
            operator_ai: None,
            attributed_via: None,
        }
    }

    /// Restores a context from stored columns — the dispatch reify path,
    /// which does not receive an attribution from a caller but carries
    /// forward the one recorded on the job row.
    ///
    /// Takes [`PersistedAttribution`] rather than three loose values so
    /// that "any channel" remains unreachable from a caller: the only
    /// way to hold one of those is to have read it back.
    pub(crate) fn from_persisted(persisted: PersistedAttribution) -> Self {
        Self {
            author: persisted.author,
            operator_ai: persisted.operator_ai,
            attributed_via: persisted.attributed_via,
        }
    }

    /// Subject this write is attributed to (`None` = unrecorded).
    pub fn author(&self) -> Option<&Author> {
        self.author.as_ref()
    }

    /// Agent that performed the operation (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel this attribution arrived through (`None` only when
    /// nothing is recorded).
    pub fn attributed_via(&self) -> Option<AttributionChannel> {
        self.attributed_via
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_pair_round_trips_through_encode_and_parse() {
        for author in [Author::Owner, Author::Subject("alice".into())] {
            let (kind, subject) = author.encode();
            assert_eq!(Author::parse(kind, subject).unwrap(), author);
        }
        assert_eq!(Author::Owner.kind_slug(), "owner");
        assert_eq!(Author::Owner.subject(), None);
        assert_eq!(Author::Subject("alice".into()).kind_slug(), "subject");
        assert_eq!(Author::Subject("alice".into()).subject(), Some("alice"));
    }

    #[test]
    fn author_parse_rejects_pairs_encode_cannot_produce() {
        // Owner with a subject: two answers to one question.
        assert!(Author::parse("owner", Some("alice")).is_err());
        // Subject without one (or with a blank one): no answer at all.
        assert!(Author::parse("subject", None).is_err());
        assert!(Author::parse("subject", Some("")).is_err());
        assert!(Author::parse("subject", Some("   ")).is_err());
        // An unknown kind is a corrupt row, not a new author type.
        assert!(Author::parse("persona", Some("aya")).is_err());
        assert!(Author::parse("", None).is_err());
    }

    #[test]
    fn author_columns_read_absence_as_unrecorded_and_reject_half_rows() {
        assert_eq!(Author::from_columns(None, None).unwrap(), None);
        assert_eq!(
            Author::from_columns(Some("owner"), None).unwrap(),
            Some(Author::Owner)
        );
        assert_eq!(
            Author::from_columns(Some("subject"), Some("alice")).unwrap(),
            Some(Author::Subject("alice".into()))
        );
        assert!(
            Author::from_columns(None, Some("alice")).is_err(),
            "a subject with no kind is a half-written row, not an owner"
        );
    }

    #[test]
    fn channel_slugs_round_trip_and_reject_unknown_values() {
        for channel in [
            AttributionChannel::OwnerSurface,
            AttributionChannel::Asserted,
            AttributionChannel::Authenticated,
        ] {
            assert_eq!(AttributionChannel::parse(channel.slug()).unwrap(), channel);
        }
        assert_eq!(AttributionChannel::OwnerSurface.slug(), "owner-surface");
        // A channel this build does not know is a corrupt row, not a
        // newer channel to be tolerated: the whole point of the column
        // is that its values are exhaustively known.
        assert!(AttributionChannel::parse("owner_surface").is_err());
        assert!(AttributionChannel::parse("delegated").is_err());
        assert!(AttributionChannel::parse("").is_err());
        assert_eq!(AttributionChannel::from_column(None).unwrap(), None);
        assert_eq!(
            AttributionChannel::from_column(Some("asserted")).unwrap(),
            Some(AttributionChannel::Asserted)
        );
    }

    #[test]
    fn owner_surface_is_the_owner_through_its_own_surface() {
        let ctx = AttributionContext::owner_surface();
        assert_eq!(ctx.author(), Some(&Author::Owner));
        assert_eq!(
            ctx.operator_ai(),
            None,
            "the surface says who, not through what"
        );
        assert_eq!(ctx.attributed_via(), Some(AttributionChannel::OwnerSurface));
    }

    #[test]
    fn asserted_records_the_claim_and_labels_it_as_one() {
        let ctx = AttributionContext::asserted(
            Some(Author::Subject("alice".into())),
            Some(OperatorRef::new("claude-code").unwrap()),
        )
        .unwrap();
        assert_eq!(ctx.author(), Some(&Author::Subject("alice".into())));
        assert_eq!(
            ctx.operator_ai().map(OperatorRef::as_str),
            Some("claude-code")
        );
        assert_eq!(ctx.attributed_via(), Some(AttributionChannel::Asserted));

        // An operator with no author is still an attribution, and still
        // carries the channel it arrived through.
        let operator_only =
            AttributionContext::asserted(None, Some(OperatorRef::new("codex").unwrap())).unwrap();
        assert_eq!(operator_only.author(), None);
        assert_eq!(
            operator_only.attributed_via(),
            Some(AttributionChannel::Asserted)
        );
    }

    #[test]
    fn asserted_refuses_to_let_a_caller_call_itself_the_owner() {
        // Rule 2: owner-ness comes from the surface or from
        // authentication. If this passed, every HTTP caller would be
        // able to write rows indistinguishable from the owner's own.
        let claimed = AttributionContext::asserted(
            Some(Author::Owner),
            Some(OperatorRef::new("claude-code").unwrap()),
        );
        assert!(claimed.is_err());
        assert!(AttributionContext::asserted(Some(Author::Owner), None).is_err());
        // A named subject is fine — that is what the channel is for.
        assert!(AttributionContext::asserted(Some(Author::Subject("alice".into())), None).is_ok());
    }

    #[test]
    fn asserting_nothing_is_the_same_value_as_recording_nobody() {
        // Rule 3, the "no author + no operator ⇒ no channel" direction:
        // an empty claim must not leave a channel behind, or a row that
        // attributes nobody would still say it was asserted.
        assert_eq!(
            AttributionContext::asserted(None, None).unwrap(),
            AttributionContext::unrecorded()
        );
        assert_eq!(
            AttributionContext::asserted(None, None)
                .unwrap()
                .attributed_via(),
            None
        );
    }

    #[test]
    fn from_persisted_restores_the_stored_triple_verbatim() {
        // The reify path carries the job row's attribution onto its
        // outputs; nothing about it may be re-derived, including the
        // channel (which describes how the *request* arrived).
        let stored = PersistedAttribution::from_columns(
            Some("owner"),
            None,
            Some("asterism-ui"),
            Some("owner-surface"),
        )
        .unwrap();
        let ctx = AttributionContext::from_persisted(stored);
        assert_eq!(ctx.author(), Some(&Author::Owner));
        assert_eq!(
            ctx.operator_ai().map(OperatorRef::as_str),
            Some("asterism-ui")
        );
        assert_eq!(
            ctx.attributed_via(),
            Some(AttributionChannel::OwnerSurface),
            "the owner survives the round trip even though `asserted` could never build it"
        );
    }

    #[test]
    fn persisted_reads_legacy_rows_and_refuses_corrupt_ones() {
        // Rows written before `attributed_via` existed: an author (or an
        // operator) with no channel. Readable, and marked as such by the
        // absent channel rather than by a guessed one.
        let legacy = PersistedAttribution::from_columns(Some("owner"), None, None, None).unwrap();
        assert_eq!(legacy.author(), Some(&Author::Owner));
        assert_eq!(legacy.attributed_via(), None);
        let legacy_operator =
            PersistedAttribution::from_columns(None, None, Some("claude-code"), None).unwrap();
        assert_eq!(
            legacy_operator.operator_ai().map(OperatorRef::as_str),
            Some("claude-code")
        );
        assert_eq!(legacy_operator.attributed_via(), None);

        // Everything absent is an ordinary unrecorded row, not legacy.
        let silent = PersistedAttribution::from_columns(None, None, None, None).unwrap();
        assert_eq!(
            (
                silent.author(),
                silent.operator_ai(),
                silent.attributed_via()
            ),
            (None, None, None)
        );

        // Each half is validated by the type that owns it.
        assert!(
            PersistedAttribution::from_columns(Some("owner"), Some("alice"), None, None).is_err(),
            "an owner carrying a subject is a corrupt pair"
        );
        assert!(
            PersistedAttribution::from_columns(None, Some("alice"), None, None).is_err(),
            "a subject with no kind is a half-written row"
        );
        assert!(
            PersistedAttribution::from_columns(None, None, Some("   "), None).is_err(),
            "a blank operator asserts nothing and must not be storable as one that does"
        );
        assert!(
            PersistedAttribution::from_columns(None, None, None, Some("surface")).is_err(),
            "an unknown channel is a corrupt row"
        );
    }

    #[test]
    fn operator_ref_rejects_blank_assertions() {
        assert_eq!(
            OperatorRef::new("claude-code").unwrap().as_str(),
            "claude-code"
        );
        assert_eq!(
            OperatorRef::new("asterism-ui").unwrap().to_string(),
            "asterism-ui"
        );
        assert!(OperatorRef::new("").is_err());
        assert!(OperatorRef::new("   ").is_err());
        assert!(OperatorRef::new("\t\n").is_err());
    }
}
