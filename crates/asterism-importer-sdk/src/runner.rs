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
    ApiClient, Progress, ScanMode, SourceError, SourceParser, SourceScanner, spec_to_command,
};

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub persona_id: String,
    pub server: String,
    pub batch_size: usize,
    pub upload_concurrency: usize,
    pub dry_run: bool,
    pub auto_organize_base_dir: Option<String>,
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
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportSummary {
    pub imported: u64,
    pub failed: u64,
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
    let mut stream = scanner.scan(mode).await?;
    let mut buffer: Vec<AddAssetCommand> = Vec::new();
    // Set when a failure ends the scan early. Returned after the tail
    // below has flushed and drained, never instead of it.
    let mut ended_by: Option<SourceError> = None;
    let batch_size = options.batch_size.max(1);
    let upload_concurrency = options.upload_concurrency.max(1);
    let mut in_flight = FuturesUnordered::new();

    while let Some(next) = stream.next().await {
        let raw = match next {
            Ok(item) => item,
            // This loop used to have one arm: record the message, take
            // the next item, whatever the failure was. Stopping
            // therefore had to be done by the scanner — `SqliteScanner`
            // returned from its reader thread after a source-level
            // failure so the stream would end — which put the decision
            // in each adapter rather than in the port, and left a
            // scanner that did not stop indistinguishable from one with
            // nothing more to say.
            //
            // Now the classification decides, in one place.
            //
            // `Retry` ends the run here too, and that is this runner's
            // limitation rather than the class's meaning: there is no
            // backoff loop to hand it to yet. A caller that grows one
            // reads `disposition` and waits instead — which is why the
            // wait rides on the error rather than being recomputed from
            // its text.
            Err(err) if err.is_item_local() => {
                progress.record_err(err.locator().unwrap_or("<scan>"), &err.to_string());
                continue;
            }
            // Breaks rather than returns. Everything already scanned,
            // parsed and mapped is in `buffer`, and up to
            // `batch_size - 1` of it has not been sent; the tasks in
            // `in_flight` are still running against a client this
            // function owns. Returning here would drop the first and
            // detach the second — work the source had already handed
            // over, thrown away because something *after* it failed.
            // The tail below sends what is held and waits for what is
            // in the air, and the failure is returned once it has.
            Err(err) => {
                progress.record_err(err.locator().unwrap_or("<scan>"), &err.to_string());
                ended_by = Some(err);
                break;
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

    if let Some(err) = ended_by {
        return Err(anyhow::Error::new(err).context(format!(
            "scan ended early; the {} record(s) already accepted were sent first",
            progress.ok_count()
        )));
    }

    Ok(ImportSummary {
        imported: progress.ok_count(),
        failed: progress.err_count(),
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
    use crate::{Footprint, FootprintSource, Note, ParseError, RawItem, SourceError};

    struct OneItemScanner;

    impl SourceScanner for OneItemScanner {
        fn scan(&self, _mode: ScanMode) -> ScanFuture<'_> {
            Box::pin(async {
                let item = RawItem {
                    source_kind: "test".into(),
                    locator: "/tmp/one.txt".into(),
                    payload: b"hello".to_vec(),
                    occurred_at: Some(Utc::now()),
                    extra: serde_json::json!({}),
                };
                Ok(Box::pin(stream::iter([Ok(item)])) as crate::scanner::ItemStream)
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
                failed: 0
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
        fn scan(&self, _mode: ScanMode) -> ScanFuture<'_> {
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
                    Ok(item),
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
        fn scan(&self, _mode: ScanMode) -> ScanFuture<'_> {
            let err = self.err.lock().expect("the fixture's error").take();
            let accepted = self.accepted;
            Box::pin(async move {
                let items = (0..accepted).map(|n| {
                    Ok(RawItem {
                        source_kind: "test".into(),
                        locator: format!("/tmp/{n}.txt"),
                        payload: b"held".to_vec(),
                        occurred_at: Some(Utc::now()),
                        extra: serde_json::json!({}),
                    })
                });
                Ok(Box::pin(stream::iter(
                    items.chain([Err(err.expect("scan is called once"))]),
                )) as crate::scanner::ItemStream)
            })
        }
    }

    /// What the source handed over before it failed is not thrown away
    /// because of the failure.
    ///
    /// The first shape of the early exit returned from the middle of
    /// the loop, which left the buffered commands — up to
    /// `batch_size - 1` of them, already scanned, parsed and mapped —
    /// unsent, and the tasks already in flight detached from the client
    /// this function owns. The run still ends; it flushes and drains
    /// first, and says how much it sent on the way out.
    #[tokio::test]
    async fn work_accepted_before_the_failure_is_not_dropped_by_it() {
        let scanner = ThenFailsScanner {
            accepted: 3,
            err: std::sync::Mutex::new(Some(SourceError::Config("token rejected".into()))),
        };
        let reported = run_import(
            &scanner,
            &NoteParser,
            ScanMode::Enumerate,
            // `batch_size` above the number accepted, so every one of
            // them is still in the buffer when the failure arrives —
            // which is the case the early return lost.
            ImportOptions {
                batch_size: 50,
                ..dry("http://127.0.0.1:1")
            },
        )
        .await
        .expect_err("the run ends")
        .chain()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join(": ");
        assert!(
            reported.contains("3 record(s) already accepted were sent first"),
            "the run accounts for what it did with them: {reported}"
        );
        assert!(
            reported.contains("token rejected"),
            "and still names what ended it: {reported}"
        );
    }

    fn dry(server: &str) -> ImportOptions {
        let mut options = ImportOptions::new("persona-id");
        options.dry_run = true;
        options.server = server.into();
        options
    }

    /// One unreadable item is a line in the report, and the item behind
    /// it still arrives.
    #[tokio::test]
    async fn an_item_local_failure_does_not_end_the_scan() {
        let scanner = FailingScanner::with(SourceError::item("/tmp/bad.txt", "permission denied"));
        let summary = run_import(
            &scanner,
            &NoteParser,
            ScanMode::Enumerate,
            dry("http://127.0.0.1:1"),
        )
        .await
        .expect("an item-local failure is not the end of the run");
        assert_eq!(
            summary,
            ImportSummary {
                imported: 1,
                failed: 1
            },
            "the failure is reported and the item behind it still lands"
        );
    }

    /// Every class but the item-local one ends this run, and the run
    /// says which failure did it rather than folding a source-level
    /// refusal into a count of failed items.
    ///
    /// `Transient` and `RateLimited` are in the list because *this*
    /// runner ends on them, not because the classes mean that: they are
    /// retried by a caller with a backoff loop, and this one has none
    /// yet. When it grows one, this test splits.
    #[tokio::test]
    async fn every_failure_but_an_item_local_one_ends_this_run() {
        for err in [
            SourceError::Config("no such directory".into()),
            SourceError::Source("malformed response".into()),
            SourceError::rate_limited("429"),
            SourceError::Transient("connection refused".into()),
        ] {
            let expected = err.to_string();
            let scanner = FailingScanner::with(err);
            let outcome = run_import(
                &scanner,
                &NoteParser,
                ScanMode::Enumerate,
                dry("http://127.0.0.1:1"),
            )
            .await;
            let reported = outcome
                .expect_err("the run ends")
                .chain()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(": ");
            assert!(
                reported.contains(&expected),
                "the run says which failure ended it: {reported}"
            );
        }
    }
}
