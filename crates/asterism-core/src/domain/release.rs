//! `Release` — the record that a change point's state was written out.
//!
//! ```text
//!   Line ── History ── ChangePoint          Snapshot ── DispatchJob
//!                          ▲                    ▲            ▲
//!                          └──── Release ───────┴────────────┘
//!                                  act
//!                                  file stamps*
//! ```
//!
//! # A release is not a change point
//!
//! Nothing the line carries changes when its contents go out. Putting a
//! release on the chain would make "the line moved" and "what the line
//! holds was sent somewhere" the same event — the argument
//! [`line`](crate::domain::forge::model::line) already gives for keeping
//! a rename off the history. So a release is a record beside the chain
//! that names one change point, carries its own
//! [`Act`](crate::domain::forge::model::act::Act), and leaves the head
//! where it was.
//!
//! Two releases of one change point are two records. Nothing here keys
//! on the change point, and nothing refuses a second one: a set going
//! out twice is two things that happened, and a store that collapsed
//! them would answer "when did this leave" with one of the two dates.
//!
//! # Why this is not in the forge
//!
//! It is the forge's kind of statement — an operator's account, with an
//! actor and a time — and it names a [`SnapshotId`] and a
//! [`DispatchId`], which the forge may not. `tests/forge_boundary.rs`
//! holds the list of words a contract across that boundary may be
//! written in, and adding two core ids to it would be the reversal
//! #254's "boundary that moves" section describes, paid for a record
//! that does not have to live inside the forge to be written.
//!
//! So it sits here, where both vocabularies are already legal, and the
//! direction the forge's own module doc states is preserved: the
//! outside may name the forge, and the forge may not name the outside.
//! Nothing the forge holds moves, and no core row learns a forge word —
//! the release is a row of its own that names both by id.
//!
//! # What the stamps are, and why they are per file
//!
//! [`Stamped`] is what applying a disclosure to one file achieved, and
//! its two halves fail independently. A release writes out as many
//! files as the change point had live entries, so the honest record is
//! one outcome per file: a build with no certificate configured reports
//! the manifest half [`Skipped`](crate::domain::disclosure::Skipped) on
//! every one of them, and a container that could not take a packet
//! reports it on one. A summary would have to pick which of those to
//! report, and the caller reading it could not get back to the file.

use chrono::{DateTime, Utc};

use crate::domain::disclosure::Stamped;
use crate::domain::forge::model::act::Act;
use crate::domain::forge::model::value::{ChangePointId, LineId};
use crate::domain::value::{AssetId, DispatchId, ReleaseId, SnapshotId};

/// What became of the disclosure on one written-out file.
///
/// The asset is the library's own row the file is a copy of; the path
/// is where the copy landed. Both are recorded because neither answers
/// for the other: two entries of one line can name one asset, and a
/// path outlives nothing — the file it names may be moved by whoever
/// receives it the moment after this is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    /// The library row this file is a copy of.
    pub asset: AssetId,
    /// Where the copy was written.
    pub path: String,
    /// What the disclosure writer reported for it.
    pub outcome: Stamped,
}

/// One change point's state, written out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    id: ReleaseId,
    line: LineId,
    change_point: ChangePointId,
    snapshot: SnapshotId,
    dispatch: DispatchId,
    act: Act,
    files: Vec<FileStamp>,
}

impl Release {
    /// Records that a change point was released: what was frozen, what
    /// carried it, and whose act it was.
    ///
    /// No stamps yet. The files do not exist at this moment — the
    /// dispatch has been started and has not run — so a release is born
    /// saying what it asked for and learns what was written later, when
    /// something has written it.
    pub fn new(
        line: LineId,
        change_point: ChangePointId,
        snapshot: SnapshotId,
        dispatch: DispatchId,
        act: Act,
    ) -> Self {
        Self {
            id: ReleaseId::new(),
            line,
            change_point,
            snapshot,
            dispatch,
            act,
            files: Vec::new(),
        }
    }

    /// Rebuilds a release under the id it was kept with, stamps and all.
    pub fn restored(
        id: ReleaseId,
        line: LineId,
        change_point: ChangePointId,
        snapshot: SnapshotId,
        dispatch: DispatchId,
        act: Act,
        files: Vec<FileStamp>,
    ) -> Self {
        Self {
            id,
            line,
            change_point,
            snapshot,
            dispatch,
            act,
            files,
        }
    }

    /// Which release.
    pub fn id(&self) -> ReleaseId {
        self.id
    }

    /// The line the released change point is on.
    ///
    /// Derivable by looking for the change point, and kept anyway: a
    /// release is read in order to say what it released, and that read
    /// starts from the line's history. Without it the first step would
    /// be a scan of every line for a node.
    pub fn line(&self) -> LineId {
        self.line
    }

    /// What was released.
    pub fn change_point(&self) -> ChangePointId {
        self.change_point
    }

    /// The freeze of what the change point carried.
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    /// The run that carried the bytes out.
    pub fn dispatch(&self) -> DispatchId {
        self.dispatch
    }

    /// When it was released, and by whom.
    pub fn act(&self) -> &Act {
        &self.act
    }

    /// What became of the disclosure on each file that left.
    ///
    /// Empty until the dispatch has written them. That is a state
    /// rather than an absence of information: a release recorded a
    /// moment ago has files on the way, and the run that writes them is
    /// what fills this in.
    pub fn files(&self) -> &[FileStamp] {
        &self.files
    }

    /// When it was released, as the instant its act carries.
    pub fn at(&self) -> DateTime<Utc> {
        self.act.at()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::disclosure::{Half, Skipped};
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::value::ActorId;
    use chrono::TimeZone;

    fn act(minute: u32) -> Act {
        Act::new(
            Utc.with_ymd_and_hms(2026, 9, 11, 12, minute, 0).unwrap(),
            Actor::User(ActorId::new()),
        )
    }

    /// A set going out twice is two things that happened. Nothing here
    /// keys on the change point, so the second release is a record of
    /// its own rather than an overwrite of the first.
    #[test]
    fn two_releases_of_one_change_point_are_two_records() {
        let line = LineId::new();
        let point = ChangePointId::new();
        let first = Release::new(line, point, SnapshotId::new(), DispatchId::new(), act(10));
        let second = Release::new(line, point, SnapshotId::new(), DispatchId::new(), act(20));

        assert_ne!(first.id(), second.id());
        assert_eq!(first.change_point(), second.change_point());
        assert_eq!(first.at(), act(10).at());
        assert_eq!(second.at(), act(20).at());
    }

    #[test]
    fn a_fresh_release_has_written_nothing_yet() {
        let release = Release::new(
            LineId::new(),
            ChangePointId::new(),
            SnapshotId::new(),
            DispatchId::new(),
            act(0),
        );

        assert!(release.files().is_empty());
    }

    /// The skipped manifest half is what an install with no certificate
    /// reports, on every file, and it is carried per file rather than
    /// summarised — see the module docs.
    #[test]
    fn a_restored_release_carries_what_each_file_was_told() {
        let asset = AssetId::new();
        let release = Release::restored(
            ReleaseId::new(),
            LineId::new(),
            ChangePointId::new(),
            SnapshotId::new(),
            DispatchId::new(),
            act(5),
            vec![FileStamp {
                asset,
                path: "/out/key-visual.png".into(),
                outcome: Stamped::new(Half::Written, Half::Skipped(Skipped::NoSigningIdentity)),
            }],
        );

        let stamp = &release.files()[0];
        assert_eq!(stamp.asset, asset);
        assert!(stamp.outcome.discloses());
        assert_eq!(
            stamp.outcome.manifest,
            Half::Skipped(Skipped::NoSigningIdentity)
        );
    }
}
