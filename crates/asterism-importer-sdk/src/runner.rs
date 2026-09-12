//! Shared importer execution pipeline.
//!
//! CLI/config parsing stays in the outer binary. This module receives
//! resolved values plus a scanner/parser pair and owns the mechanical
//! scan → parse → batch → HTTP → progress loop.
//!
//! # Where a declared digest comes from
//!
//! [`AssetSpec::declared_content_hash`](crate::AssetSpec::declared_content_hash)
//! is filled in here rather than in any parser, because this is the one
//! place that can see both halves of the question at once: the scanner
//! says whether its payload is a whole artefact
//! ([`SourceScanner::payload_is_whole_artefact`]), and the spec says
//! whether the record still lives at the address the scanner read. Only
//! when both hold do the bytes in hand belong to the locator being
//! registered, and only then is a digest a true statement about the
//! file the server will later open.
//!
//! A parser could not decide this on its own: it is handed the payload
//! and hands back footprints, and whether those footprints kept the
//! item's address or split it into records inside the item is visible
//! only after the mapping — one Claude Code session file yields
//! messages addressed `<file>#<uuid>`, and one PNG yields itself.

use std::sync::Arc;

use anyhow::Context;
use asterism_contract::command::{AddAssetBatchCommand, AddAssetCommand};
use asterism_contract::digest;
use futures::stream::{FuturesUnordered, StreamExt};

use crate::{
    ApiClient, Progress, ScanEvent, ScanMode, SourceError, SourceParser, SourceScanner, SyncState,
    spec_to_command,
};

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub persona_id: String,
    pub server: String,
    pub batch_size: usize,
    pub upload_concurrency: usize,
    pub dry_run: bool,
    pub auto_organize_base_dir: Option<String>,
    /// Where to take up, or `None` to start at the beginning.
    ///
    /// Handed to the scanner unread: what it means belongs to whoever
    /// wrote it. A scanner that cannot use it refuses the scan, so a
    /// caller that asked to resume and could not is told — see
    /// [`SourceScanner::scan`].
    pub resume_from: Option<SyncState>,
}

impl ImportOptions {
    pub fn new(persona_id: impl Into<String>) -> Self {
        Self {
            persona_id: persona_id.into(),
            server: "http://127.0.0.1:8989".into(),
            batch_size: 50,
            upload_concurrency: 1,
            dry_run: false,
            auto_organize_base_dir: None,
            resume_from: None,
        }
    }
}

/// What one run did.
///
/// Returned for every run that *started*, including one a failure cut
/// short: the counts are what the run achieved and
/// [`ended_by`](Self::ended_by) is why it is not a success, so a caller
/// reads both from one place. Returning the failure *instead* would
/// throw the counts away at the moment somebody wants them.
///
/// [`run_import`] still returns `Err` for a run that could not start,
/// where there is nothing to count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    /// Records the server accepted.
    pub imported: u64,
    /// Records that did not land: one the source could not read, one
    /// no parser would take, one the server refused.
    ///
    /// Not the failure that cost the run — that is
    /// [`ended_by`](Self::ended_by). Counting it here would put "the
    /// source refused our credential" and "one file would not open"
    /// back into the one number the classification exists to tell
    /// apart, and a run that lost no records would report one.
    pub failed: u64,
    /// The failure that cost the run, if one did. `None` is a run that
    /// ended on the source's own terms — which is not the same as a run
    /// with no failures at all, since a lost record raises `failed` and
    /// leaves this `None`.
    pub ended_by: Option<SourceError>,
    /// Where a later run may take up, if this one earned the right to
    /// say.
    ///
    /// The last checkpoint the scanner emitted — **and only when
    /// nothing failed**. A checkpoint is the scanner's promise that it
    /// has handed over everything before it; storing one is a promise
    /// that everything before it *landed*, and this function cannot
    /// make the second promise about a run in which something did not.
    ///
    /// It could make it about part of one, by tracking which records
    /// sat between which checkpoints, and that is not free: batches are
    /// answered out of order, so the bookkeeping would be real. So
    /// a run with a single failed record hands back nothing, the next
    /// run re-reads from wherever it last resumed, and the cost of that
    /// is paid in reading rather than in a record nobody notices is
    /// missing.
    ///
    /// A scan cut short still earns one, which is the case worth
    /// having: a source that rate-limits halfway is exactly when a
    /// caller wants to take up rather than begin again.
    ///
    /// Handed back rather than kept: where a resumption point is
    /// stored is the transport's question and not this function's, and
    /// answering it here would settle it for every caller.
    pub resume_from: Option<SyncState>,
}

pub async fn run_import<S, P>(
    scanner: &S,
    parser: &P,
    mode: ScanMode,
    options: ImportOptions,
) -> anyhow::Result<ImportSummary>
where
    S: SourceScanner + ?Sized,
    P: SourceParser + ?Sized,
{
    let client = Arc::new(ApiClient::new(options.server.clone()));
    if !options.dry_run {
        client
            .health()
            .await
            .with_context(|| format!("cannot reach asterism-server at {}", options.server))?;
    }

    let progress = Progress::new();
    let payload_is_whole_artefact = scanner.payload_is_whole_artefact();
    let mut stream = scanner.scan(mode, options.resume_from.clone()).await?;
    let mut buffer: Vec<AddAssetCommand> = Vec::new();
    // The failure that cost the run, if the scanner sent one. Carried
    // out on the summary rather than thrown instead of it.
    let mut ended_by: Option<SourceError> = None;
    // The furthest point the scanner said it had got past. Whether this
    // run may hand it back is decided at the end, when what the server
    // did with the records in front of it is known.
    let mut last_checkpoint: Option<SyncState> = None;
    let batch_size = options.batch_size.max(1);
    let upload_concurrency = options.upload_concurrency.max(1);
    let mut in_flight = FuturesUnordered::new();

    while let Some(next) = stream.next().await {
        let event = match next {
            Ok(event) => event,
            // A failure never leaves this loop. It ends when the stream
            // does, which is the scanner's call and not this
            // function's: `SqliteScanner` stops after a row it could
            // not read and `FsScanner` takes the next file, each for a
            // reason about its own source that this function does not
            // have. A runner that jumped out on the class would have
            // overruled one of them.
            //
            // Reading on also means the ordinary tail below — flush
            // what is buffered, wait for what is in flight — runs
            // however the scan ended, rather than an early exit having
            // to remember to do it. It did not, the first time.
            //
            // A lost record is counted; a failure that cost the run is
            // carried whole on the summary. Counting that one too
            // would report a run that lost no records as having lost
            // one.
            Err(err) if err.is_record_lost() => {
                progress.record_err(err.locator().unwrap_or("<scan>"), &err.to_string());
                continue;
            }
            Err(err) => {
                // The last one wins. A scanner that keeps its side of
                // the bargain sends at most one, because it ends after
                // a failure it cannot continue past; one that sends
                // several is reported by the one it finished on.
                ended_by = Some(err);
                continue;
            }
        };
        let raw = match event {
            ScanEvent::Item(item) => item,
            // A checkpoint is not a record. It is counted nowhere, and
            // a scan of nothing but checkpoints imports nothing.
            ScanEvent::Checkpoint(state) => {
                last_checkpoint = Some(state);
                continue;
            }
        };
        let raw_locator = raw.locator.clone();
        // Computed before `parse` takes the payload, and only when the
        // scanner says the payload is the whole artefact. It is a CPU
        // pass over bytes already in memory — no second read — and it
        // is thrown away for every item whose parser addresses records
        // *inside* it, which is the price of not keeping a copy of the
        // payload alive across the parse.
        let declared_content_hash =
            payload_is_whole_artefact.then(|| digest::of_bytes(&raw.payload));
        let footprints = match parser.parse(raw) {
            Ok(footprints) => footprints,
            Err(err) => {
                progress.record_err(&raw_locator, &err.to_string());
                continue;
            }
        };

        for footprint in footprints {
            let mut spec = footprint.into_asset_spec();
            // The second half of the test. The digest describes the
            // bytes at `raw_locator`; a spec that moved to another
            // address — one record inside the item (`<file>#<uuid>`,
            // `<card>#field=name`), or a different file the parser
            // found named in this one (a Claude Code image marker) —
            // is not what was hashed, and attaching it there would
            // report a mismatch about a file that is fine.
            //
            // A parser that already set the field is left alone: it
            // read something this loop cannot see.
            if spec.declared_content_hash.is_none() && spec.locator == raw_locator {
                spec.declared_content_hash = declared_content_hash.clone();
            }
            let command = spec_to_command(spec, &options.persona_id);
            if options.dry_run {
                progress.record_ok(&format!("dry-run {}", command.locator));
                continue;
            }
            buffer.push(command);
            if buffer.len() >= batch_size {
                submit_batch(
                    &mut in_flight,
                    &client,
                    &progress,
                    &options.auto_organize_base_dir,
                    std::mem::take(&mut buffer),
                    upload_concurrency,
                )
                .await;
            }
        }

        // A watch stream may never end. Flush each changed source item
        // so a quiet source cannot leave a partial batch buffered forever.
        if mode == ScanMode::Watch && !buffer.is_empty() && !options.dry_run {
            submit_batch(
                &mut in_flight,
                &client,
                &progress,
                &options.auto_organize_base_dir,
                std::mem::take(&mut buffer),
                upload_concurrency,
            )
            .await;
        }
    }

    if !buffer.is_empty() {
        submit_batch(
            &mut in_flight,
            &client,
            &progress,
            &options.auto_organize_base_dir,
            std::mem::take(&mut buffer),
            upload_concurrency,
        )
        .await;
    }
    while let Some(result) = in_flight.next().await {
        if let Err(err) = result {
            progress.record_err("<batch-task>", &err.to_string());
        }
    }

    // Decided here rather than as each checkpoint arrived, because
    // this is the first line at which what the server did with every
    // record is known: batches are sent while the scan is still
    // running and answered out of order.
    let failed = progress.err_count();
    Ok(ImportSummary {
        imported: progress.ok_count(),
        failed,
        ended_by,
        resume_from: if failed == 0 { last_checkpoint } else { None },
    })
}

async fn submit_batch(
    in_flight: &mut FuturesUnordered<tokio::task::JoinHandle<()>>,
    client: &Arc<ApiClient>,
    progress: &Progress,
    auto_organize_base_dir: &Option<String>,
    items: Vec<AddAssetCommand>,
    upload_concurrency: usize,
) {
    while in_flight.len() >= upload_concurrency {
        if let Some(Err(err)) = in_flight.next().await {
            progress.record_err("<batch-task>", &err.to_string());
        }
    }
    let client = Arc::clone(client);
    let progress = progress.clone();
    let auto_organize_base_dir = auto_organize_base_dir.clone();
    in_flight.push(tokio::spawn(async move {
        flush_batch(&client, items, &progress, auto_organize_base_dir).await;
    }));
}

async fn flush_batch(
    client: &ApiClient,
    items: Vec<AddAssetCommand>,
    progress: &Progress,
    auto_organize_base_dir: Option<String>,
) {
    let count = items.len();
    match client
        .add_asset_batch(AddAssetBatchCommand {
            items: items.clone(),
            auto_organize_base_dir,
        })
        .await
    {
        // A duplicate is no longer a failure, so this reads two
        // outcomes rather than four. The `skip` / `trashed` counts that
        // used to sit here were the server's UNIQUE violation parsed
        // back out of its own message — a vocabulary of failures kept in
        // step by hand across two crates, and it went with the
        // constraint that produced it. A record arriving again is
        // answered by the server's lookup and comes back on the success
        // side, holding the id it already had.
        Ok(result) => {
            let mut ok_count = 0u64;
            for (index, item) in items.iter().enumerate() {
                let failed = result.failed.get(index).map(String::as_str).unwrap_or("");
                if failed.is_empty() {
                    progress.record_ok(&item.locator);
                    ok_count += 1;
                } else {
                    progress.record_err(&item.locator, failed);
                }
            }
            eprintln!(
                "batch flushed: {ok_count} registered / {} failed (of {count})",
                result.failure_count
            );
        }
        Err(err) => {
            for item in &items {
                progress.record_err(&item.locator, &err.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use futures::stream;

    use super::*;
    use crate::scanner::ScanFuture;
    use crate::{
        Footprint, FootprintSource, Note, ParseError, RawItem, ScanEvent, SourceError, SyncState,
    };

    struct OneItemScanner;

    impl SourceScanner for OneItemScanner {
        fn scan(&self, _mode: ScanMode, _resume_from: Option<SyncState>) -> ScanFuture<'_> {
            Box::pin(async {
                let item = RawItem {
                    source_kind: "test".into(),
                    locator: "/tmp/one.txt".into(),
                    payload: b"hello".to_vec(),
                    occurred_at: Some(Utc::now()),
                    extra: serde_json::json!({}),
                };
                Ok(Box::pin(stream::iter([Ok(ScanEvent::Item(item))]))
                    as crate::scanner::ItemStream)
            })
        }
    }

    struct NoteParser;

    impl SourceParser for NoteParser {
        fn parse(&self, raw: RawItem) -> Result<Vec<Footprint>, ParseError> {
            Ok(vec![Footprint::Note(Note {
                source: FootprintSource {
                    kind: raw.source_kind,
                    locator: raw.locator,
                    platform: None,
                    external_id: None,
                },
                occurred_at: raw.occurred_at.unwrap(),
                occurred_source: Default::default(),
                body: String::from_utf8(raw.payload).unwrap(),
                source_app: None,
                labels: vec![],
                bundle_id: None,
                extra: raw.extra,
            })])
        }
    }

    #[tokio::test]
    async fn dry_run_uses_shared_pipeline_without_contacting_server() {
        let mut options = ImportOptions::new("persona-id");
        options.dry_run = true;
        options.server = "http://127.0.0.1:1".into();
        let summary = run_import(&OneItemScanner, &NoteParser, ScanMode::Enumerate, options)
            .await
            .unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                imported: 1,
                failed: 0,
                ended_by: None,
                resume_from: None,
            }
        );
    }

    /// A scanner whose stream yields one failure and then an item, so a
    /// test can say whether the loop got past the failure.
    struct FailingScanner(std::sync::Mutex<Option<SourceError>>);

    impl FailingScanner {
        fn with(err: SourceError) -> Self {
            Self(std::sync::Mutex::new(Some(err)))
        }
    }

    impl SourceScanner for FailingScanner {
        fn scan(&self, _mode: ScanMode, _resume_from: Option<SyncState>) -> ScanFuture<'_> {
            let err = self.0.lock().expect("the fixture's error").take();
            Box::pin(async move {
                let item = RawItem {
                    source_kind: "test".into(),
                    locator: "/tmp/after.txt".into(),
                    payload: b"reached".to_vec(),
                    occurred_at: Some(Utc::now()),
                    extra: serde_json::json!({}),
                };
                Ok(Box::pin(stream::iter([
                    Err(err.expect("scan is called once")),
                    Ok(ScanEvent::Item(item)),
                ])) as crate::scanner::ItemStream)
            })
        }
    }

    /// A scanner that hands over `accepted` items and then fails, so a
    /// test can say what happened to the items in front of the failure.
    struct ThenFailsScanner {
        accepted: usize,
        err: std::sync::Mutex<Option<SourceError>>,
    }

    impl SourceScanner for ThenFailsScanner {
        fn scan(&self, _mode: ScanMode, _resume_from: Option<SyncState>) -> ScanFuture<'_> {
            let err = self.err.lock().expect("the fixture's error").take();
            let accepted = self.accepted;
            Box::pin(async move {
                let items = (0..accepted).map(|n| {
                    Ok(ScanEvent::Item(RawItem {
                        source_kind: "test".into(),
                        locator: format!("/tmp/{n}.txt"),
                        payload: b"held".to_vec(),
                        occurred_at: Some(Utc::now()),
                        extra: serde_json::json!({}),
                    }))
                });
                Ok(Box::pin(stream::iter(
                    items.chain([Err(err.expect("scan is called once"))]),
                )) as crate::scanner::ItemStream)
            })
        }
    }

    fn dry(server: &str) -> ImportOptions {
        let mut options = ImportOptions::new("persona-id");
        options.dry_run = true;
        options.server = server.into();
        options
    }

    /// A lost record is a line in the report and not the run's
    /// verdict. The fixture puts the failure *first*, so what this
    /// pins is that the loop went on to read the item behind it — and
    /// that nothing was recorded as having ended the run.
    #[tokio::test]
    async fn a_lost_record_is_not_the_run_s_verdict() {
        let scanner = FailingScanner::with(SourceError::item("/tmp/bad.txt", "permission denied"));
        let summary = run_import(
            &scanner,
            &NoteParser,
            ScanMode::Enumerate,
            dry("http://127.0.0.1:1"),
        )
        .await
        .expect("a lost record is not the end of the run");
        assert_eq!(
            summary,
            ImportSummary {
                imported: 1,
                failed: 1,
                ended_by: None,
                resume_from: None,
            },
            "reported, and the item behind it still lands"
        );
    }

    /// Every other class costs the run, and the summary says which one
    /// did — rather than the caller reading a count of failed records
    /// and guessing whether one of them was the source itself. The
    /// fixture puts the failure first and an item behind it, so the
    /// count also shows the loop did not stop reading.
    ///
    /// The run is still returned rather than thrown: whatever it
    /// managed is in the counts, and this is the moment somebody wants
    /// them.
    #[tokio::test]
    async fn any_other_class_costs_the_run_and_is_named_on_it() {
        for err in [
            SourceError::Config("no such directory".into()),
            SourceError::Source("malformed response".into()),
            SourceError::rate_limited("429"),
            SourceError::Transient("connection refused".into()),
        ] {
            let scanner = FailingScanner::with(err.clone());
            let summary = run_import(
                &scanner,
                &NoteParser,
                ScanMode::Enumerate,
                dry("http://127.0.0.1:1"),
            )
            .await
            .expect("a run that started is a run that reports");
            assert_eq!(
                summary.ended_by.as_ref(),
                Some(&err),
                "named on the summary"
            );
            assert_eq!(
                summary.imported, 1,
                "and what the source handed over before it is still counted"
            );
            assert_eq!(
                summary.failed, 0,
                "and it is not also counted as a record that did not land — \
                 no record failed here, and a report saying one did is the \
                 distinction this classification exists to draw, undone"
            );
        }
    }

    /// The scanner decides when the stream ends, not the classification.
    ///
    /// `SqliteScanner` sends a lost record and then stops, for the
    /// reason written beside the `break` that does it. Nothing here
    /// overrules that: the run ends where the stream ends, with the
    /// record counted and no verdict against the run.
    #[tokio::test]
    async fn a_scanner_that_stops_after_a_lost_record_is_not_overruled() {
        struct StopsAfterLoss;

        impl SourceScanner for StopsAfterLoss {
            fn scan(&self, _mode: ScanMode, _resume_from: Option<SyncState>) -> ScanFuture<'_> {
                Box::pin(async {
                    Ok(Box::pin(stream::iter([Err(SourceError::item(
                        "/db#7",
                        "row read failed",
                    ))])) as crate::scanner::ItemStream)
                })
            }
        }

        let summary = run_import(
            &StopsAfterLoss,
            &NoteParser,
            ScanMode::Enumerate,
            dry("http://127.0.0.1:1"),
        )
        .await
        .expect("the stream ending is not a failure");
        assert_eq!(
            summary,
            ImportSummary {
                imported: 0,
                failed: 1,
                ended_by: None,
                resume_from: None,
            },
            "one record lost, and the run itself is not condemned for it"
        );
    }

    /// What the source handed over before a run-ending failure is sent,
    /// not dropped.
    ///
    /// This is the one thing on this branch that needs a server: the
    /// records sit in the batch buffer until something flushes them, so
    /// "were they sent" can only be answered by something that receives
    /// them. The first shape of the early exit returned from inside the
    /// loop and skipped the flush entirely, and the test written for it
    /// ran under `dry_run`, where nothing is ever buffered — it asserted
    /// a message rather than a delivery, and would have passed with the
    /// bug in place.
    #[tokio::test(flavor = "multi_thread")]
    async fn records_accepted_before_a_run_ending_failure_are_still_sent() {
        let received = Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
        let port = spawn_counting_server(Arc::clone(&received)).await;

        let scanner = ThenFailsScanner {
            accepted: 3,
            err: std::sync::Mutex::new(Some(SourceError::Config("token rejected".into()))),
        };
        let mut options = ImportOptions::new("persona-id");
        options.server = format!("http://127.0.0.1:{port}");
        // Above the number accepted, so all three are still in the
        // buffer when the failure arrives — which is the case an early
        // return loses.
        options.batch_size = 50;

        let summary = run_import(&scanner, &NoteParser, ScanMode::Enumerate, options)
            .await
            .expect("a run that started is a run that reports");

        assert_eq!(
            *received.lock().expect("the server's log"),
            vec![3],
            "one batch, holding every record accepted before the failure"
        );
        assert_eq!(summary.imported, 3, "and the server's answer is counted");
        assert_eq!(
            summary.ended_by,
            Some(SourceError::Config("token rejected".into()))
        );
    }

    /// A scanner shaped like the bundled ones: a checkpoint behind each
    /// item, and a record of what it was handed to resume from.
    struct CheckpointingScanner {
        items: usize,
        /// Sent after the items, if the test asked for one.
        tail: std::sync::Mutex<Option<SourceError>>,
        handed: std::sync::Mutex<Option<SyncState>>,
    }

    impl CheckpointingScanner {
        fn new(items: usize) -> Self {
            Self {
                items,
                tail: std::sync::Mutex::new(None),
                handed: std::sync::Mutex::new(None),
            }
        }

        fn then(self, err: SourceError) -> Self {
            *self.tail.lock().expect("the fixture's error") = Some(err);
            self
        }

        /// The checkpoint this scanner sends behind item `n`.
        fn checkpoint(n: usize) -> SyncState {
            SyncState::new("test", serde_json::json!({ "after": n }))
        }
    }

    impl SourceScanner for CheckpointingScanner {
        fn scan(&self, _mode: ScanMode, resume_from: Option<SyncState>) -> ScanFuture<'_> {
            *self.handed.lock().expect("the fixture's record") = resume_from;
            let items = self.items;
            let tail = self.tail.lock().expect("the fixture's error").take();
            Box::pin(async move {
                let events = (0..items).flat_map(|n| {
                    [
                        Ok(ScanEvent::Item(RawItem {
                            source_kind: "test".into(),
                            locator: format!("/tmp/{n}.txt"),
                            payload: b"held".to_vec(),
                            occurred_at: Some(Utc::now()),
                            extra: serde_json::json!({}),
                        })),
                        Ok(ScanEvent::Checkpoint(Self::checkpoint(n))),
                    ]
                });
                Ok(Box::pin(stream::iter(events.chain(tail.map(Err))))
                    as crate::scanner::ItemStream)
            })
        }
    }

    /// A checkpoint is the scanner's bookkeeping and not a record: the
    /// counts do not move for one, and the last one is what a later run
    /// may take up from.
    #[tokio::test]
    async fn a_checkpoint_is_not_a_record_and_the_last_one_comes_back() {
        let scanner = CheckpointingScanner::new(2);
        let summary = run_import(
            &scanner,
            &NoteParser,
            ScanMode::Enumerate,
            dry("http://127.0.0.1:1"),
        )
        .await
        .expect("a scan of items and checkpoints is an ordinary run");
        assert_eq!(
            summary,
            ImportSummary {
                imported: 2,
                failed: 0,
                ended_by: None,
                resume_from: Some(CheckpointingScanner::checkpoint(1)),
            },
            "two records and two checkpoints, and only the records counted"
        );
    }

    /// A scan of nothing but checkpoints imports nothing.
    ///
    /// Asserted apart from the mixed case above, where a checkpoint
    /// counted as a record would hide inside the items' own count.
    #[tokio::test]
    async fn a_scan_of_checkpoints_alone_imports_nothing() {
        struct CheckpointsOnly;

        impl SourceScanner for CheckpointsOnly {
            fn scan(&self, _mode: ScanMode, _resume_from: Option<SyncState>) -> ScanFuture<'_> {
                Box::pin(async {
                    Ok(Box::pin(stream::iter([
                        Ok(ScanEvent::Checkpoint(CheckpointingScanner::checkpoint(0))),
                        Ok(ScanEvent::Checkpoint(CheckpointingScanner::checkpoint(1))),
                    ])) as crate::scanner::ItemStream)
                })
            }
        }

        let summary = run_import(
            &CheckpointsOnly,
            &NoteParser,
            ScanMode::Enumerate,
            dry("http://127.0.0.1:1"),
        )
        .await
        .expect("a scan that hands over no records is not a failure");
        assert_eq!(
            summary,
            ImportSummary {
                imported: 0,
                failed: 0,
                ended_by: None,
                resume_from: Some(CheckpointingScanner::checkpoint(1)),
            },
            "nothing counted, and the last point still earned"
        );
    }

    /// A run that lost a record hands back nothing, though it saw a
    /// checkpoint before the loss.
    ///
    /// Keeping one would promise that everything in front of it landed,
    /// and this run cannot make that promise about the part of itself
    /// that failed. The next run re-reads instead, which costs reading
    /// rather than a record nobody notices is missing.
    #[tokio::test]
    async fn a_run_that_lost_a_record_earns_no_resumption_point() {
        let scanner = CheckpointingScanner::new(1)
            .then(SourceError::item("/tmp/bad.txt", "permission denied"));
        let summary = run_import(
            &scanner,
            &NoteParser,
            ScanMode::Enumerate,
            dry("http://127.0.0.1:1"),
        )
        .await
        .expect("a lost record is not the end of the run");
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(
            summary.resume_from, None,
            "the checkpoint was reached, but not earned"
        );
    }

    /// What the caller asked to resume from reaches the scanner as
    /// given. Nothing here reads it: what a state means belongs to
    /// whoever wrote it.
    #[tokio::test]
    async fn the_resumption_point_reaches_the_scanner_unread() {
        let scanner = CheckpointingScanner::new(1);
        let asked = SyncState::new("test", serde_json::json!({ "after": 41 }));
        let mut options = dry("http://127.0.0.1:1");
        options.resume_from = Some(asked.clone());

        run_import(&scanner, &NoteParser, ScanMode::Enumerate, options)
            .await
            .expect("a resumed run is an ordinary run");
        assert_eq!(
            *scanner.handed.lock().expect("the fixture's record"),
            Some(asked),
            "handed over whole, neither read nor rewritten"
        );
    }

    /// A loopback server that answers the two calls an import makes and
    /// records how many records each batch carried.
    ///
    /// Hand-rolled rather than reached for: this crate has no HTTP test
    /// dependency, and what the test needs is a socket that says 200 and
    /// counts. Returns the port it bound.
    async fn spawn_counting_server(received: Arc<std::sync::Mutex<Vec<usize>>>) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let received = Arc::clone(&received);
                tokio::spawn(async move {
                    // Read until the body is whole rather than once: a
                    // single `read` returns whatever one segment
                    // carried, and a batch split across two would be
                    // counted as a batch of nothing — a test failing
                    // for a reason that is not the code's.
                    let mut request = String::new();
                    let mut buf = vec![0u8; 8 * 1024];
                    loop {
                        let Ok(n) = socket.read(&mut buf).await else {
                            return;
                        };
                        if n == 0 {
                            break;
                        }
                        request.push_str(&String::from_utf8_lossy(&buf[..n]));
                        let Some(head_end) = request.find("\r\n\r\n") else {
                            continue;
                        };
                        let declared = request
                            .split("\r\n")
                            .find_map(|line| {
                                line.strip_prefix("content-length: ")
                                    .or_else(|| line.strip_prefix("Content-Length: "))
                            })
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if request.len() - (head_end + 4) >= declared {
                            break;
                        }
                    }
                    let body = if request.contains("/asterism/assets/add-batch") {
                        let items = request.matches("\"locator\"").count();
                        received.lock().expect("the server's log").push(items);
                        let ids: Vec<String> = (0..items).map(|n| format!("\"id-{n}\"")).collect();
                        format!(
                            "{{\"succeeded\":[{}],\"failed\":[],\"success_count\":{items},\"failure_count\":0}}",
                            ids.join(",")
                        )
                    } else {
                        "{}".to_string()
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                });
            }
        });
        port
    }
}
