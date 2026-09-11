//! `SendService` — putting a release's stamped set on a destination's
//! host.
//!
//! One verb: [`send`](SendService::send) reads the release's file rows,
//! builds the transport's params from the caller's profile plus that
//! list, starts a dispatch over the release's own snapshot, and records
//! that it happened.
//!
//! # The bytes are the release's copies, never the library's
//!
//! Stamping runs after harvest, on the copies the `file` exporter
//! reported — [`outbound_stamp`](crate::application_support::outbound_stamp)
//! is where that ordering is decided. A transport sends its bytes inside
//! `dispatch`, before harvest, so a transport pointed at the frozen
//! members would send the library's own unstamped files. The stamped set
//! already exists by then and the release names every copy by path, so
//! the paths are what this hands over and the ordering falls out with
//! nothing in the runner to change.
//!
//! The dispatch still runs over the release's snapshot, which is what
//! makes the exporter's inputs the same assets the copies were made
//! from. That is what the sidecar's columns are rendered against; it is
//! not where the bytes come from.
//!
//! # Where the file list lives in the params
//!
//! Under [`RESERVED_KEY`], written here and refused from a profile. The
//! alternative was a second argument to `Exporter::dispatch` beside
//! `params`, which would widen the port every adapter is written against
//! for one adapter's convenience. A reserved key costs the profile
//! author one word they may not use, and the refusal below is what tells
//! them so — a profile that set it would otherwise decide which bytes
//! left.
//!
//! # A send with nothing to send is refused
//!
//! Two states get the same answer, [`Blocked`](crate::error::ConflictKind::Blocked),
//! because both are the state being in the way rather than the request
//! being malformed: a release whose run has not written its files yet,
//! and a release one of whose copies is no longer where it was written.
//! Sending the first would put an empty directory on somebody's host;
//! sending the second would send a set that is not the set the release
//! stamped.

use std::sync::Arc;

use crate::application::dispatch_service::DispatchService;
use crate::domain::attribution::AttributionContext;
use crate::domain::forge::boundary::actors::Actors;
use crate::domain::forge::clock::Clock;
use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::release::Release;
use crate::domain::repository::{ReleaseRepository, SendRepository};
use crate::domain::send::ReleaseSend;
use crate::domain::value::{ReleaseId, SendId};
use crate::error::DomainError;

/// The exporter a send writes through.
///
/// Named here rather than taken from the caller, as
/// [`ReleaseService`](crate::application::ReleaseService) names its own:
/// which adapter puts the bytes on a host is this service's answer, and
/// letting a caller choose would make "send" mean whatever the caller
/// passed. Which *host*, and how it is spoken to, is the profile's.
const TRANSFER_EXPORTER: &str = "transfer";

/// The action that exporter takes.
const PUT_ACTION: &str = "put";

/// The params key this service writes the file list under, and which a
/// profile may not set.
pub const RESERVED_KEY: &str = "release";

/// Recording a send.
pub struct SendService {
    releases: Arc<dyn ReleaseRepository>,
    sends: Arc<dyn SendRepository>,
    dispatches: Arc<DispatchService>,
    actors: Arc<dyn Actors>,
    clock: Arc<dyn Clock>,
}

impl SendService {
    /// Wires the service around its ports.
    pub fn new(
        releases: Arc<dyn ReleaseRepository>,
        sends: Arc<dyn SendRepository>,
        dispatches: Arc<DispatchService>,
        actors: Arc<dyn Actors>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            releases,
            sends,
            dispatches,
            actors,
            clock,
        }
    }

    /// Sends one release to the destination the profile describes.
    ///
    /// `destination` is the label the send is remembered by and nothing
    /// else — [`ReleaseSend::destination`] says why it is not a
    /// vocabulary. `profile` is the transport's params as the caller
    /// wrote them, and this adds the file list to it.
    ///
    /// # Refusals
    ///
    /// - [`NotFound`](DomainError::NotFound) — no such release.
    /// - [`Validation`](DomainError::Validation) — the profile is not a
    ///   JSON object, or it sets [`RESERVED_KEY`].
    /// - [`Conflict`](DomainError::Conflict), [`Blocked`](crate::error::ConflictKind::Blocked)
    ///   — the release has written no files yet, or one of the files it
    ///   wrote is no longer there.
    /// - Whatever dispatching refuses.
    pub async fn send(
        &self,
        release: &ReleaseId,
        destination: &str,
        profile: &serde_json::Value,
        by: &AttributionContext,
    ) -> Result<ReleaseSend, DomainError> {
        let released = self
            .releases
            .find(release)
            .await?
            .ok_or_else(|| DomainError::not_found("release", release))?;
        let params = with_file_list(profile, &released)?;

        let dispatch = self
            .dispatches
            .create(
                asterism_contract::command::CreateDispatchCommand {
                    snapshot_id: released.snapshot().to_string(),
                    exporter_slug: TRANSFER_EXPORTER.into(),
                    action: PUT_ACTION.into(),
                    params_json: params.to_string(),
                    operator_ai: by.operator_ai().map(|op| op.as_str().to_string()),
                },
                by,
            )
            .await?;

        let send = ReleaseSend::new(
            released.id(),
            destination.to_string(),
            crate::application::mapping::parse_dispatch_id(&dispatch.id)?,
            self.act(by).await?,
        );
        self.sends.record(&send).await?;
        Ok(send)
    }

    /// Reads one send back.
    pub async fn get(&self, id: &SendId) -> Result<ReleaseSend, DomainError> {
        self.sends
            .find(id)
            .await?
            .ok_or_else(|| DomainError::not_found("release send", id))
    }

    /// Every send of one release, most recent first.
    ///
    /// The release is read first, so a send list asked for under an id
    /// nothing has is a refusal rather than an empty answer.
    pub async fn of_release(&self, release: &ReleaseId) -> Result<Vec<ReleaseSend>, DomainError> {
        self.releases
            .find(release)
            .await?
            .ok_or_else(|| DomainError::not_found("release", release))?;
        self.sends.of_release(release).await
    }

    /// Stamps an act: now, by whoever this write is from.
    async fn act(&self, by: &AttributionContext) -> Result<Act, DomainError> {
        Ok(Act::new(
            self.clock.now(),
            Actor::User(self.actors.resolve(by).await?),
        ))
    }
}

/// The caller's profile with the release's file list written into it.
///
/// One entry per copy, in the order the release recorded them, each
/// naming the library row the copy was made from, where the copy is now,
/// and what it is called. The exporter pairs an entry to an input by
/// `asset_id`; `name` is what it puts the copy under unless the profile
/// renames it.
fn with_file_list(
    profile: &serde_json::Value,
    release: &Release,
) -> Result<serde_json::Value, DomainError> {
    let serde_json::Value::Object(fields) = profile else {
        return Err(DomainError::Validation(
            "a transfer profile is a JSON object of the transport's params".into(),
        ));
    };
    if fields.contains_key(RESERVED_KEY) {
        return Err(DomainError::Validation(format!(
            "a transfer profile may not set {RESERVED_KEY:?}: it is where the send \
             writes the release's own file list, and a profile that set it would \
             decide which bytes leave"
        )));
    }
    if release.files().is_empty() {
        return Err(DomainError::blocked(format!(
            "release {} has written no files yet, so there is nothing to send; the \
             run that writes them is what fills that in",
            release.id()
        )));
    }
    let mut files = Vec::with_capacity(release.files().len());
    for stamp in release.files() {
        let path = std::path::Path::new(&stamp.path);
        if !path.is_file() {
            return Err(DomainError::blocked(format!(
                "release {} names a file that is no longer there, so what would be \
                 sent is not the set it stamped: {}",
                release.id(),
                stamp.path
            )));
        }
        files.push(serde_json::json!({
            "asset_id": stamp.asset.to_string(),
            "path": stamp.path,
            "name": path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| stamp.path.clone()),
        }));
    }
    let mut params = fields.clone();
    params.insert(
        RESERVED_KEY.to_string(),
        serde_json::json!({ "files": files }),
    );
    Ok(serde_json::Value::Object(params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::disclosure::{Half, Skipped, Stamped};
    use crate::domain::forge::model::act::Actor;
    use crate::domain::forge::model::value::{ActorId, ChangePointId, LineId};
    use crate::domain::release::FileStamp;
    use crate::domain::value::{AssetId, DispatchId, SnapshotId};
    use chrono::{TimeZone, Utc};

    fn act() -> Act {
        Act::new(
            Utc.with_ymd_and_hms(2026, 9, 11, 12, 0, 0).unwrap(),
            Actor::User(ActorId::new()),
        )
    }

    /// A release that wrote `paths`, each of which exists on disk.
    fn a_release_that_wrote(paths: &[std::path::PathBuf]) -> Release {
        Release::restored(
            crate::domain::value::ReleaseId::new(),
            LineId::new(),
            ChangePointId::new(),
            SnapshotId::new(),
            DispatchId::new(),
            act(),
            paths
                .iter()
                .map(|path| FileStamp {
                    asset: AssetId::new(),
                    path: path.display().to_string(),
                    outcome: Stamped::new(Half::Written, Half::Skipped(Skipped::NoSigningIdentity)),
                })
                .collect(),
        )
    }

    fn a_written_file(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"bytes").expect("write a copy");
        path
    }

    #[test]
    fn the_file_list_names_every_copy_in_the_order_it_was_written() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = vec![
            a_written_file(tmp.path(), "0.png"),
            a_written_file(tmp.path(), "1.png"),
        ];
        let release = a_release_that_wrote(&paths);

        let params = with_file_list(&serde_json::json!({ "endpoint": "sftp://h/d" }), &release)
            .expect("a file list");

        assert_eq!(params["endpoint"], "sftp://h/d");
        let files = params[RESERVED_KEY]["files"].as_array().expect("files");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["name"], "0.png");
        assert_eq!(files[0]["path"], paths[0].display().to_string());
        assert_eq!(files[0]["asset_id"], release.files()[0].asset.to_string());
        assert_eq!(files[1]["name"], "1.png");
    }

    /// The reserved key is the whole of what a profile may not decide —
    /// see the module docs.
    #[test]
    fn a_profile_that_sets_the_reserved_key_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let release = a_release_that_wrote(&[a_written_file(tmp.path(), "0.png")]);

        let refused = with_file_list(
            &serde_json::json!({ RESERVED_KEY: { "files": [] } }),
            &release,
        );

        assert!(matches!(refused, Err(DomainError::Validation(_))));
    }

    #[test]
    fn a_release_that_has_written_nothing_is_refused() {
        let release = Release::new(
            LineId::new(),
            ChangePointId::new(),
            SnapshotId::new(),
            DispatchId::new(),
            act(),
        );

        let refused = with_file_list(&serde_json::json!({}), &release);

        assert!(matches!(
            refused,
            Err(DomainError::Conflict {
                kind: crate::error::ConflictKind::Blocked,
                ..
            })
        ));
    }

    /// What would be sent has to be the set the release stamped, so a
    /// copy that has been moved away since stops the send rather than
    /// shrinking it.
    #[test]
    fn a_copy_that_is_no_longer_there_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = a_written_file(tmp.path(), "0.png");
        let release = a_release_that_wrote(std::slice::from_ref(&path));
        std::fs::remove_file(&path).expect("take the copy away");

        let refused = with_file_list(&serde_json::json!({}), &release);

        assert!(matches!(
            refused,
            Err(DomainError::Conflict {
                kind: crate::error::ConflictKind::Blocked,
                ..
            })
        ));
    }

    #[test]
    fn a_profile_that_is_not_an_object_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let release = a_release_that_wrote(&[a_written_file(tmp.path(), "0.png")]);

        let refused = with_file_list(&serde_json::json!("sftp://host/dir"), &release);

        assert!(matches!(refused, Err(DomainError::Validation(_))));
    }
}
