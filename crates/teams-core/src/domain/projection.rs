//! The captured projection — descriptive metadata, keyed by entry and
//! opaque all the way down (#148 decisions 12, 13 and 14).
//!
//! ## Outside the forge, on purpose
//!
//! The forge has three axes and #102 forbids a column that answers
//! what the history already answers, so a description is not a fourth
//! axis and does not go on a change point. It lives beside the forge
//! on this plane, keyed `(line, entry)`. The consequence the design
//! actually wants from that: **a projection can be lost without the
//! line lying.** Everything else here follows from it — including why
//! the write does not share the forge's transaction, and why nothing
//! appends to the ledger when one is captured.
//!
//! ## Captured, not owned
//!
//! It is what the promoter said at the time, on the same discipline as
//! an `ActorStamp` capturing a display name at write time. The team
//! does not edit it. Only a forge op replaces one — which is why the
//! write rides on the round push (#148 decision 19) rather than
//! getting a verb of its own, so no second editing surface grows
//! beside the verbs.
//!
//! ## Opaque, and this module is where that is kept honest
//!
//! Decision 14 gives the test: *if a port signature, a column, or a
//! DTO ever names something inside the body, this decision has been
//! broken.* [`ProjectionBody`] exists so that the test is easy to
//! apply here — it is a newtype over a string that nothing on this
//! plane parses, whose one accessor hands back the whole of it, and
//! which has no `serde_json` anywhere near it. This plane stores it,
//! hands it back, and never learns what is in it.

use uuid::Uuid;

use crate::DomainError;

/// A description, as bytes this plane does not read.
///
/// Deliberately not a `serde_json::Value`: parsing it here would make
/// its schema an architectural fact of the teams plane, which is
/// exactly what decision 14 refuses. It is not validated as JSON
/// either — a body that does not parse is a member's client's bug and
/// this plane is not the place it is caught, because catching it would
/// mean opening the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionBody(String);

impl ProjectionBody {
    /// The largest body this plane will store, in bytes.
    ///
    /// A bound rather than a schema. The plane has to decide *how much*
    /// it is willing to hold per entry even though it will not look at
    /// what it holds, for the reason every unbounded caller-sized input
    /// on this surface gets a ceiling (#148 decision 18's argument,
    /// applied to a write): the alternative is a client deciding how
    /// large this instance's database is. 64 KiB is far past what any
    /// description written by a person needs and far short of anything
    /// that would be used as storage.
    pub const MAX_BYTES: usize = 64 * 1024;

    /// Takes a body, refusing only what is empty or past the ceiling.
    ///
    /// Those two are the whole of the validation, and both are facts
    /// about the string rather than about its contents: an empty body
    /// is a projection that says nothing, which is what *absent* is
    /// for.
    pub fn parse(raw: impl Into<String>) -> Result<Self, DomainError> {
        let raw = raw.into();
        if raw.trim().is_empty() {
            return Err(DomainError::Validation(
                "a projection body says something; an entry with nothing to say has no \
                 projection rather than an empty one"
                    .to_string(),
            ));
        }
        if raw.len() > Self::MAX_BYTES {
            return Err(DomainError::Validation(format!(
                "a projection body is at most {} bytes and this one is {}",
                Self::MAX_BYTES,
                raw.len()
            )));
        }
        Ok(Self(raw))
    }

    /// The body, verbatim — for storing it and for handing it back.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One entry's captured projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryProjection {
    /// The line the entry is on.
    pub line_id: Uuid,
    /// The entry the description is about.
    pub entry_id: Uuid,
    /// The team whose line it is.
    ///
    /// Not part of the key — a line id is unique across teams, so the
    /// key is already exact. What it is for is the read: a caller
    /// arrives holding one team's session, and a store that could not
    /// say whose row it was holding would answer with anybody's.
    pub team_id: Uuid,
    /// Which mapper wrote [`Self::body`].
    ///
    /// **A fact about the envelope, not a field of the body**, and
    /// this is the distinction the rest of the tree points at. It lets
    /// this plane keep and hand back bodies it has never opened. The
    /// body carries its own version too, because decision 14 puts the
    /// branch at the mapper and the mapper reads the body; nothing on
    /// this plane compares the two, and nothing may start to, because
    /// comparing them means opening the body.
    pub version: u32,
    /// The description.
    pub body: ProjectionBody,
    /// The member whose push captured it.
    pub promoted_by: Uuid,
    /// When it was captured, epoch ms.
    pub pushed_at_ms: i64,
}
