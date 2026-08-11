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

use super::{RawItem, ScanError, ScanFuture, ScanMode, SourceScanner};

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

    fn read_item(&self, path: PathBuf) -> Result<RawItem, ScanError> {
        let payload = std::fs::read(&path)
            .map_err(|e| ScanError::ItemReadFailed(format!("{}: {e}", path.display())))?;
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

    fn scan(&self, mode: ScanMode) -> ScanFuture<'_> {
        let root = self.root.clone();
        let this = self.clone();
        Box::pin(async move {
            if !root.exists() {
                return Err(ScanError::SourceUnavailable(format!(
                    "path does not exist: {}",
                    root.display()
                )));
            }

            // Enumerate the current tree into a channel so both modes
            // can share the same stream shape.
            let (tx, rx) = mpsc::channel::<Result<RawItem, ScanError>>(64);
            let enumerate = {
                let this = this.clone();
                let root = root.clone();
                let tx = tx.clone();
                async move {
                    // Walk the tree explicitly — never silently drop
                    // `walkdir::Error` (per-directory read failures on
                    // macOS especially can otherwise strand thousands
                    // of files with no user-visible signal).
                    for entry_res in WalkDir::new(&root) {
                        let entry = match entry_res {
                            Ok(e) => e,
                            Err(err) => {
                                let path = err
                                    .path()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_else(|| "<no-path>".into());
                                let _ = tx
                                    .send(Err(ScanError::ItemReadFailed(format!(
                                        "walkdir at {path}: {err}"
                                    ))))
                                    .await;
                                continue;
                            }
                        };
                        if !entry.file_type().is_file() {
                            continue;
                        }
                        if !this.accepts(entry.path()) {
                            continue;
                        }
                        let item = this.read_item(entry.path().to_path_buf());
                        if tx.send(item).await.is_err() {
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
                                let _ = tx
                                    .send(Err(ScanError::SourceUnavailable(format!(
                                        "watcher init failed: {err}"
                                    ))))
                                    .await;
                                return;
                            }
                        };
                        if let Err(err) = watcher.watch(&root_watch, RecursiveMode::Recursive) {
                            let _ = tx
                                .send(Err(ScanError::SourceUnavailable(format!(
                                    "watch failed: {err}"
                                ))))
                                .await;
                            return;
                        }
                        while let Some(res) = evt_rx.recv().await {
                            let event = match res {
                                Ok(ev) => ev,
                                Err(err) => {
                                    let _ = tx
                                        .send(Err(ScanError::ItemReadFailed(format!(
                                            "watcher error: {err}"
                                        ))))
                                        .await;
                                    continue;
                                }
                            };
                            for path in event.paths {
                                if !path.is_file() || !this_watch.accepts(&path) {
                                    continue;
                                }
                                let item = this_watch.read_item(path);
                                if tx.send(item).await.is_err() {
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
