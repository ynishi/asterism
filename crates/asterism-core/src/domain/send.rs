//! `ReleaseSend` — the record that a release was put on a destination's
//! host.
//!
//! ```text
//!   Release ── snapshot ── DispatchJob
//!      ▲                        ▲
//!      └──── ReleaseSend ───────┘
//!               act
//!               destination
//! ```
//!
//! # Why it hangs off a release and not off a change point
//!
//! What travels is the stamped set, and the stamped set is what a
//! release left behind: [`Release::files`](crate::domain::release::Release::files)
//! names every copy by asset and by path. A send anchored one node
//! higher would have to freeze and stamp a second time to know what its
//! bytes were, and the two stamped sets would then differ by whenever
//! the second pass ran.
//!
//! # Two sends of one release are two records
//!
//! Nothing here keys on the release, and nothing refuses a second send:
//! a re-submission after a rejection is the case this record exists to
//! hold, and a store that collapsed the two would answer "when did this
//! go out" with one of the dates.
//!
//! # What a send does not carry
//!
//! What became of the put. The dispatch it names is where that lives —
//! its state, and the attempt record naming each file and what the
//! server answered — and nothing in this process would maintain a
//! second copy: the runner does not know what a send is, which is the
//! same reason
//! [`OutboundStamping`](crate::application_support::OutboundStamping)
//! is a port rather than a call.
//!
//! # Why the type is not called `Send`
//!
//! `Send` is a trait in the prelude, and traits and structs share the
//! type namespace. A `struct Send` here would shadow the bound in every
//! module that imported it, so `Send + Sync` on the ports beside it
//! would stop naming the auto trait. The record's own word is in the
//! module name, the id, the repository and the verb; only the struct
//! carries the qualifier.

use chrono::{DateTime, Utc};

use crate::domain::forge::model::act::Act;
use crate::domain::value::{DispatchId, ReleaseId, SendId};

/// One release, put on a destination's host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseSend {
    id: SendId,
    release: ReleaseId,
    destination: String,
    dispatch: DispatchId,
    act: Act,
}

impl ReleaseSend {
    /// Records that a release was sent: where to, what carried it, and
    /// whose act it was.
    pub fn new(release: ReleaseId, destination: String, dispatch: DispatchId, act: Act) -> Self {
        Self {
            id: SendId::new(),
            release,
            destination,
            dispatch,
            act,
        }
    }

    /// Rebuilds a send under the id it was kept with.
    pub fn restored(
        id: SendId,
        release: ReleaseId,
        destination: String,
        dispatch: DispatchId,
        act: Act,
    ) -> Self {
        Self {
            id,
            release,
            destination,
            dispatch,
            act,
        }
    }

    /// Which send.
    pub fn id(&self) -> SendId {
        self.id
    }

    /// The release whose stamped set went out.
    pub fn release(&self) -> ReleaseId {
        self.release
    }

    /// Where it went, as the label the caller gave it.
    ///
    /// A label and nothing more. What an agency's intake asks for
    /// changes on that agency's schedule, so a vocabulary here would
    /// encode somebody else's policy; the profile the send was given is
    /// where a destination is actually described, and it is on the
    /// dispatch this names.
    pub fn destination(&self) -> &str {
        &self.destination
    }

    /// The run that carried the bytes.
    pub fn dispatch(&self) -> DispatchId {
        self.dispatch
    }

    /// When it was sent, and by whom.
    pub fn act(&self) -> &Act {
        &self.act
    }

    /// When it was sent, as the instant its act carries.
    pub fn at(&self) -> DateTime<Utc> {
        self.act.at()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::value::ActorId;
    use chrono::TimeZone;

    fn act(minute: u32) -> Act {
        Act::new(
            Utc.with_ymd_and_hms(2026, 9, 11, 12, minute, 0).unwrap(),
            Actor::User(ActorId::new()),
        )
    }

    /// A re-submission after a rejection is the case the record exists
    /// to hold — see the module docs.
    #[test]
    fn two_sends_of_one_release_are_two_records() {
        let release = ReleaseId::new();
        let first = ReleaseSend::new(release, "adobe-stock".into(), DispatchId::new(), act(10));
        let second = ReleaseSend::new(release, "adobe-stock".into(), DispatchId::new(), act(20));

        assert_ne!(first.id(), second.id());
        assert_eq!(first.release(), second.release());
        assert_ne!(first.dispatch(), second.dispatch());
        assert_eq!(first.at(), act(10).at());
        assert_eq!(second.at(), act(20).at());
    }

    #[test]
    fn a_restored_send_carries_what_it_was_kept_with() {
        let id = SendId::new();
        let release = ReleaseId::new();
        let dispatch = DispatchId::new();
        let send = ReleaseSend::restored(id, release, "dreamstime".into(), dispatch, act(5));

        assert_eq!(send.id(), id);
        assert_eq!(send.release(), release);
        assert_eq!(send.destination(), "dreamstime");
        assert_eq!(send.dispatch(), dispatch);
    }
}
