//! `FsScanner` — filesystem source scanner.
//!
//! Walks a directory tree, optionally filtered by glob-ish extension
//! set, and emits every matching file as a `RawItem`. In `Watch` mode
//! the scanner also stays live and streams filesystem-change events via
//! `notify` — new / modified files are re-emitted, deletions are
//! ignored (deletions on the source do not automatically delete the
//! corresponding asset; that is a policy decision left to the caller).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::stream::StreamExt;
use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use walkdir::WalkDir;

use super::{RawItem, ScanEvent, ScanFuture, ScanMode, SourceScanner};
use crate::port::{SourceError, SyncState};

/// Filesystem scanner.
///
/// - `root` — directory to walk.
/// - `extensions` — file extensions (without the leading dot) to keep.
///   An empty vector means "accept every file".
/// - `source_kind` — slug written to `RawItem.source_kind` (defaults to
///   `"fs"`).
#[derive(Debug, Clone)]
pub struct FsScanner {
    root: PathBuf,
    extensions: Arc<Vec<String>>,
    source_kind: String,
}

impl FsScanner {
    /// Builds a scanner rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            extensions: Arc::new(Vec::new()),
            source_kind: "fs".into(),
        }
    }

    /// Restricts the scanner to files whose extension matches one of
    /// `exts` (case-insensitive, without the leading dot).
    pub fn with_extensions<I, S>(mut self, exts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.extensions = Arc::new(
            exts.into_iter()
                .map(|s| s.into().to_ascii_lowercase())
                .collect(),
        );
        self
    }

    /// Overrides the slug written to `RawItem.source_kind`. Rarely
    /// needed — importers usually stick with `"fs"`.
    pub fn with_source_kind(mut self, slug: impl Into<String>) -> Self {
        self.source_kind = slug.into();
        self
    }

    /// The resumable unit: this scanner's root.
    ///
    /// One walk, one partition. A scanner rooted somewhere else is
    /// scanning something else, which is what makes a state from it
    /// refusable rather than merely unhelpful.
    fn partition(&self) -> String {
        format!("root={}", self.root.display())
    }

    /// The path a resumption point says was the last one handled, or an
    /// answer for a caller that has to be told why it cannot resume.
    ///
    /// Two refusals, both `Config`, because both are about how the
    /// scanner was built rather than about the source. A state from
    /// another root is one; a state whose offset this version does not
    /// understand is the other — a scanner that shrugged and started
    /// over would re-import the tree while looking exactly like one
    /// that resumed.
    fn resume_after(&self, state: Option<SyncState>) -> Result<Option<String>, SourceError> {
        let Some(state) = state else {
            return Ok(None);
        };
        if state.partition != self.partition() {
            return Err(SourceError::Config(format!(
                "cannot resume: the state is for {:?} and this scanner walks {:?}",
                state.partition,
                self.partition()
            )));
        }
        match state.offset.get("after_path").and_then(|v| v.as_str()) {
            Some(path) => Ok(Some(path.to_string())),
            None => Err(SourceError::Config(format!(
                "cannot resume: the state for {:?} carries no `after_path`",
                state.partition
            ))),
        }
    }

    fn accepts(&self, path: &Path) -> bool {
        if self.extensions.is_empty() {
            return true;
        }
        match path.extension().and_then(|s| s.to_str()) {
            Some(ext) => self
                .extensions
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(ext)),
            None => false,
        }
    }

    fn read_item(&self, path: PathBuf) -> Result<RawItem, SourceError> {
        let payload =
            std::fs::read(&path).map_err(|e| SourceError::item(path.display().to_string(), e))?;
        let (occurred_at, size) = std::fs::metadata(&path)
            .map(|m| {
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|d| DateTime::<Utc>::from_timestamp_millis(d.as_millis() as i64));
                (mtime, m.len())
            })
            .unwrap_or((None, 0));
        let extra = serde_json::json!({
            "file_size_bytes": size,
        });
        Ok(RawItem {
            source_kind: self.source_kind.clone(),
            locator: path.display().to_string(),
            payload,
            occurred_at,
            extra,
        })
    }
}

impl SourceScanner for FsScanner {
    /// Yes: [`read_item`](Self::read_item) is a `std::fs::read` of the
    /// very path it writes into `locator`, so the payload is that
    /// file's whole content and nothing was assembled on the way.
    ///
    /// Saying so is all this scanner does about digests — whether one
    /// is computed and whether it survives to the wire is the runner's
    /// call, because that is where it becomes visible whether the
    /// parser kept the file's own address or split the file into
    /// records inside it.
    fn payload_is_whole_artefact(&self) -> bool {
        true
    }

    fn scan(&self, mode: ScanMode, resume_from: Option<SyncState>) -> ScanFuture<'_> {
        let root = self.root.clone();
        let this = self.clone();
        Box::pin(async move {
            // `Config` rather than `Transient`: the directory the
            // caller named is not there, and no amount of waiting
            // makes a path appear. The message is for whoever typed
            // it.
            if !root.exists() {
                return Err(SourceError::Config(format!(
                    "path does not exist: {}",
                    root.display()
                )));
            }

            // Refused before anything is walked: a caller that asked
            // to resume and cannot needs to hear so instead of
            // receiving a whole tree it already has.
            let after_path = this.resume_after(resume_from)?;
            let partition = this.partition();

            // Enumerate the current tree into a channel so both modes
            // can share the same stream shape.
            let (tx, rx) = mpsc::channel::<Result<ScanEvent, SourceError>>(64);
            let enumerate = {
                let this = this.clone();
                let root = root.clone();
                let tx = tx.clone();
                async move {
                    // Walk the tree explicitly — never silently drop
                    // `walkdir::Error` (per-directory read failures on
                    // macOS especially can otherwise strand thousands
                    // of files with no user-visible signal).
                    //
                    // Sorted, which is what makes "after this path"
                    // mean anything. `walkdir`'s default order is the
                    // directory's own, so an unsorted walk could put a
                    // file the resumption point excludes *after* one it
                    // admits, and the second run would skip files it
                    // had never seen. The order is the comparison the
                    // resumption uses, so the two are written beside
                    // each other deliberately.
                    let walk = WalkDir::new(&root).sort_by_file_name();
                    for entry_res in walk {
                        let entry = match entry_res {
                            Ok(e) => e,
                            Err(err) => {
                                let path = err
                                    .path()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_else(|| "<no-path>".into());
                                let _ = tx.send(Err(SourceError::item(path, err))).await;
                                continue;
                            }
                        };
                        if !entry.file_type().is_file() {
                            continue;
                        }
                        if !this.accepts(entry.path()) {
                            continue;
                        }
                        let path = entry.path().to_path_buf();
                        let locator = path.display().to_string();
                        if let Some(after) = &after_path
                            && locator.as_str() <= after.as_str()
                        {
                            continue;
                        }
                        let event = this.read_item(path).map(ScanEvent::Item);
                        if tx.send(event).await.is_err() {
                            return;
                        }
                        // After the file, not before: the checkpoint
                        // says everything already yielded is dealt
                        // with, and this file has been.
                        let checkpoint = ScanEvent::Checkpoint(SyncState::new(
                            partition.clone(),
                            serde_json::json!({ "after_path": locator }),
                        ));
                        if tx.send(Ok(checkpoint)).await.is_err() {
                            return;
                        }
                    }
                }
            };

            match mode {
                ScanMode::Enumerate => {
                    tokio::spawn(async move {
                        enumerate.await;
                    });
                }
                ScanMode::Watch => {
                    // Enumerate first, then keep the channel open and
                    // push filesystem events as they arrive.
                    let this_watch = this.clone();
                    let root_watch = root.clone();
                    tokio::spawn(async move {
                        enumerate.await;
                        let (evt_tx, mut evt_rx) =
                            mpsc::channel::<Result<notify::Event, notify::Error>>(64);
                        let mut watcher = match notify::recommended_watcher(move |res| {
                            let _ = evt_tx.blocking_send(res);
                        }) {
                            Ok(w) => w,
                            Err(err) => {
                                // Nothing about this run gets further:
                                // the platform refused a watcher, so
                                // there is no second half of the scan
                                // to wait for.
                                let _ = tx
                                    .send(Err(SourceError::Source(format!(
                                        "watcher init failed: {err}"
                                    ))))
                                    .await;
                                return;
                            }
                        };
                        if let Err(err) = watcher.watch(&root_watch, RecursiveMode::Recursive) {
                            let _ = tx
                                .send(Err(SourceError::Source(format!(
                                    "watch failed: {} : {err}",
                                    root_watch.display()
                                ))))
                                .await;
                            return;
                        }
                        while let Some(res) = evt_rx.recv().await {
                            let event = match res {
                                Ok(ev) => ev,
                                Err(err) => {
                                    // `Source`, and then the task ends.
                                    // What reaches this arm is the
                                    // watch itself failing — on inotify
                                    // a read error on the descriptor,
                                    // or a watch refused because the
                                    // process is at its limit as a new
                                    // subdirectory appears; the macOS
                                    // backend hands this channel
                                    // nothing but events, so there the
                                    // arm is unreachable. Where it is
                                    // reached, the stream would
                                    // otherwise go quiet while the
                                    // directory kept changing, which is
                                    // worse than ending.
                                    //
                                    // Not `Item`: there is no item to
                                    // name, and `Item` is the class a
                                    // caller carries on past, so a
                                    // watch that had begun failing
                                    // would report this for as long as
                                    // it lived.
                                    let _ = tx
                                        .send(Err(SourceError::Source(format!(
                                            "watching {}: {err}",
                                            root_watch.display()
                                        ))))
                                        .await;
                                    return;
                                }
                            };
                            for path in event.paths {
                                if !path.is_file() || !this_watch.accepts(&path) {
                                    continue;
                                }
                                let event = this_watch.read_item(path).map(ScanEvent::Item);
                                if tx.send(event).await.is_err() {
                                    return;
                                }
                            }
                        }
                    });
                }
            }
            Ok(ReceiverStream::new(rx).boxed())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::Disposition;
    use futures::StreamExt;

    /// Everything the stream yielded, checkpoints included.
    async fn drain_events(
        scanner: FsScanner,
        resume_from: Option<SyncState>,
    ) -> Vec<Result<ScanEvent, SourceError>> {
        match scanner.scan(ScanMode::Enumerate, resume_from).await {
            Ok(stream) => stream.collect().await,
            Err(err) => vec![Err(err)],
        }
    }

    /// The same, with the bookkeeping dropped — what a parser would
    /// ever be handed.
    async fn drain(scanner: FsScanner) -> Vec<Result<RawItem, SourceError>> {
        drain_events(scanner, None)
            .await
            .into_iter()
            .filter_map(|r| match r {
                Ok(ScanEvent::Item(item)) => Some(Ok(item)),
                Ok(ScanEvent::Checkpoint(_)) => None,
                Err(err) => Some(Err(err)),
            })
            .collect()
    }

    /// The locators, in the order the walk yielded them.
    fn locators(events: &[Result<ScanEvent, SourceError>]) -> Vec<String> {
        events
            .iter()
            .filter_map(|r| match r {
                Ok(ScanEvent::Item(item)) => Some(item.locator.clone()),
                _ => None,
            })
            .collect()
    }

    /// The last checkpoint the scan emitted, which is what a runner
    /// would keep.
    fn last_checkpoint(events: &[Result<ScanEvent, SourceError>]) -> Option<SyncState> {
        events
            .iter()
            .filter_map(|r| match r {
                Ok(ScanEvent::Checkpoint(state)) => Some(state.clone()),
                _ => None,
            })
            .next_back()
    }

    /// A root that is not there is the caller's answer, and the scan
    /// never starts.
    #[tokio::test]
    async fn a_root_that_is_not_there_is_the_callers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = FsScanner::new(tmp.path().join("nope"))
            .scan(ScanMode::Enumerate, None)
            .await
            .err()
            .expect("no directory, no scan");
        assert!(matches!(err, SourceError::Config(_)), "{err}");
        assert_eq!(err.disposition(), Disposition::Failed);
    }

    /// The other half of the port's one rule, and the half
    /// `SqliteScanner` answers differently: a file this scanner cannot
    /// read costs that file, and the scan goes on to the next one.
    ///
    /// The locator is on the failure, so the report names the file that
    /// was skipped rather than saying that something was.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unreadable_file_costs_that_file_and_the_scan_goes_on() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().expect("tempdir");
        // Named so the walk reaches the unreadable one first.
        let blocked = tmp.path().join("1-blocked.txt");
        let readable = tmp.path().join("2-readable.txt");
        std::fs::write(&blocked, b"secret").expect("write");
        std::fs::write(&readable, b"open").expect("write");
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).expect("chmod");

        // Root can read anything, so the fixture cannot be built there;
        // the walk would return two items and say nothing about the
        // class.
        if std::fs::read(&blocked).is_ok() {
            eprintln!("skipped: this user can read a 0o000 file");
            return;
        }

        let out = drain(FsScanner::new(tmp.path())).await;
        let (items, failures): (Vec<_>, Vec<_>) = out.into_iter().partition(|r| r.is_ok());
        assert_eq!(items.len(), 1, "the readable file still arrives");
        assert_eq!(failures.len(), 1);
        let err = failures.into_iter().next().unwrap().unwrap_err();
        assert_eq!(
            err.disposition(),
            Disposition::RecordLost,
            "one file, not the run: {err}"
        );
        assert_eq!(
            err.locator(),
            Some(blocked.display().to_string().as_str()),
            "and the report names which file"
        );
    }

    /// Every file under the root, with the payload and the address the
    /// parser will be handed.
    #[tokio::test]
    async fn files_arrive_with_their_bytes_and_their_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(tmp.path().join("sub")).expect("mkdir");
        std::fs::write(tmp.path().join("a.txt"), b"one").expect("write");
        std::fs::write(tmp.path().join("sub/b.md"), b"two").expect("write");
        std::fs::write(tmp.path().join("sub/c.bin"), b"three").expect("write");

        let out = drain(FsScanner::new(tmp.path()).with_extensions(["txt", "md"])).await;
        let mut got: Vec<(String, Vec<u8>)> = out
            .into_iter()
            .map(|r| r.expect("no failures here"))
            .map(|item| (item.locator, item.payload))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                (
                    tmp.path().join("a.txt").display().to_string(),
                    b"one".to_vec()
                ),
                (
                    tmp.path().join("sub/b.md").display().to_string(),
                    b"two".to_vec()
                ),
            ],
            "the extension filter keeps `c.bin` out"
        );
    }

    /// What the whole resumption is for: a second scan handed the first
    /// one's checkpoint reads what follows it, and not the tree over
    /// again.
    ///
    /// The sorted walk is asserted here too, in the same test, because
    /// it is the same property: "after this path" only names a set at
    /// all if the order it refers to is the order the walk takes.
    #[tokio::test]
    async fn a_resumed_walk_yields_only_what_follows_the_checkpoint() {
        let tmp = tempfile::tempdir().expect("tempdir");
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(tmp.path().join(name), name.as_bytes()).expect("write");
        }
        let at = |name: &str| tmp.path().join(name).display().to_string();

        let first = drain_events(FsScanner::new(tmp.path()), None).await;
        assert_eq!(
            locators(&first),
            vec![at("a.txt"), at("b.txt"), at("c.txt")],
            "sorted, which is what lets a path stand for a position"
        );

        // The checkpoint that follows the first file — the scan got
        // that far and no further, which is the state a run cut short
        // after one file would have kept.
        let after_a = match &first[1] {
            Ok(ScanEvent::Checkpoint(state)) => state.clone(),
            other => panic!("a checkpoint follows each file, found {other:?}"),
        };

        let second = drain_events(FsScanner::new(tmp.path()), Some(after_a)).await;
        assert_eq!(
            locators(&second),
            vec![at("b.txt"), at("c.txt")],
            "the file already handled does not come back"
        );
        assert_eq!(
            last_checkpoint(&second).map(|state| state.offset),
            Some(serde_json::json!({ "after_path": at("c.txt") })),
            "and the resumed scan earns a point of its own"
        );
    }

    /// A state this scanner cannot honour is refused, because the
    /// alternative — shrugging and starting from the top — re-imports
    /// the tree while looking exactly like a resumption that worked.
    #[tokio::test]
    async fn a_state_this_scanner_cannot_use_is_refused_rather_than_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("a.txt"), b"one").expect("write");

        let refusals = [
            // Written by a scanner walking somewhere else.
            SyncState::new(
                "root=/somewhere/else",
                serde_json::json!({ "after_path": "/somewhere/else/a.txt" }),
            ),
            // This root, but an offset this version does not read.
            SyncState::new(
                format!("root={}", tmp.path().display()),
                serde_json::json!({ "cursor": 7 }),
            ),
        ];
        for state in refusals {
            let err = FsScanner::new(tmp.path())
                .scan(ScanMode::Enumerate, Some(state))
                .await
                .err()
                .expect("the scan does not start");
            assert!(
                matches!(err, SourceError::Config(_)),
                "how the scanner was built, not what the source did: {err}"
            );
        }
    }
}
