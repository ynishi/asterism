//! End-to-end: a remote source, resumed from a cursor the server kept.
//!
//! #295 proved a resumption point survives the round trip through the
//! router and the table, and proved it with `FsScanner` — whose offset
//! is a filesystem path the scanner compares. This asks the same
//! question of the offset shape that arrived with `HttpScanner`: an
//! opaque token that nothing on either side can compare, order, or do
//! anything with except hand back to the source that issued it.
//!
//! The two halves have to agree without either understanding the value.
//! That is the port's opacity rule at full stretch, and it is the reason
//! this file exists rather than a note saying the fs case covers it.
//!
//! What is driven here: a scripted HTTP source over loopback, the real
//! `HttpScanner`, `run_import_with` over the real `HttpSyncStore`, the
//! actual router, the actual `import_state` table. Nobody types a
//! cursor at any stage.
//!
//! Its own test binary because `init_core` opens a Tantivy index (one
//! core per test binary, as with the sibling e2e files).

use std::sync::{Arc, Mutex};

use asterism_contract::command::RegisterPersonaCommand;
use asterism_importer_sdk::scanner::http::{Cursor, RecordMap};
use asterism_importer_sdk::{
    ApiClient, Footprint, FootprintSource, HttpScanner, HttpSyncStore, ImportOptions, Note,
    OccurredSource, ParseError, RawItem, ScanMode, SourceParser, SourceScanner, StateKey,
    SyncStore, resolve_occurrence, run_import_with,
};
use asterism_server::core_init::{CoreCtx, CoreMode, LogEmitter, init_core_with};
use asterism_server::state::ServerCtx;

/// A caller that states nothing, which records nothing.
fn unattributed() -> asterism_core::domain::attribution::AttributionContext {
    asterism_core::domain::attribution::AttributionContext::asserted(None, None)
        .expect("stating no author and no operator is always valid")
}

async fn boot(tmp: &std::path::Path) -> (CoreCtx, u16) {
    let core = init_core_with(
        &tmp.join("asterism.db"),
        Arc::new(LogEmitter),
        CoreMode::ReadOnly,
        Some(&tmp.join("tantivy")),
    )
    .await
    .expect("init_core");
    let router = asterism_server::http::router(ServerCtx::from_core(&core));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (core, port)
}

async fn register(core: &CoreCtx, pack_id: &str) -> String {
    core.persona_service
        .register(
            RegisterPersonaCommand {
                name: "E2E".into(),
                pack_id: Some(pack_id.into()),
            },
            &unattributed(),
        )
        .await
        .expect("register persona")
        .id
}

/// One note per record, the record's own JSON as the body — the same
/// shape the `http` subcommand uses.
struct RecordParser;

impl SourceParser for RecordParser {
    fn parse(&self, raw: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let body = String::from_utf8(raw.payload).expect("the fixture writes JSON");
        let (occurred_at, occurred_source) =
            resolve_occurrence(raw.occurred_at, OccurredSource::Record, None);
        Ok(vec![Footprint::Note(Note {
            source: FootprintSource {
                kind: raw.source_kind,
                locator: raw.locator,
                platform: None,
                external_id: None,
            },
            occurred_at,
            occurred_source,
            body,
            source_app: None,
            labels: vec![],
            bundle_id: None,
            extra: raw.extra,
        })])
    }
}

/// A loopback source that serves pages and remembers what it was asked
/// for.
///
/// Separate from the scanner's own fixture on purpose: that one lives in
/// the crate under test and could drift into agreeing with it by
/// construction. This one is written from the outside, against the wire
/// format and nothing else.
async fn spawn_source(pages: Vec<(Vec<&'static str>, Option<&'static str>)>) -> (u16, Requests) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    let asked = Requests::default();
    let log = asked.clone();
    let bodies: Vec<String> = pages
        .into_iter()
        .map(|(ids, next)| {
            let items: Vec<String> = ids
                .iter()
                .map(|id| format!(r#"{{"id":"{id}","said":"record {id}"}}"#))
                .collect();
            let next = match next {
                Some(token) => format!(r#""{token}""#),
                None => "null".into(),
            };
            format!(
                r#"{{"data":{{"items":[{}]}},"paging":{{"next":{next}}}}}"#,
                items.join(",")
            )
        })
        .collect();

    tokio::spawn(async move {
        let mut served = 0usize;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
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
                if request.contains("\r\n\r\n") {
                    break;
                }
            }
            let target = request.split_whitespace().nth(1).unwrap_or("/").to_string();
            let cursor = target.split_once('?').and_then(|(_, query)| {
                query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("cursor=").map(str::to_string))
            });
            log.0.lock().expect("the source's log").push(cursor);

            // Past the script the source says so rather than repeating,
            // so a scanner that will not stop paging fails instead of
            // hanging a test nobody is watching.
            let response = match bodies.get(served) {
                Some(body) => format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ),
                None => "HTTP/1.1 500 X\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into(),
            };
            served += 1;
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });
    (port, asked)
}

/// Every cursor the scanner asked with, in order.
#[derive(Clone, Default)]
struct Requests(Arc<Mutex<Vec<Option<String>>>>);

impl Requests {
    fn cursors(&self) -> Vec<Option<String>> {
        self.0.lock().expect("the source's log").clone()
    }
}

fn options(persona: &str, port: u16) -> ImportOptions {
    let mut options = ImportOptions::new(persona);
    options.server = format!("http://127.0.0.1:{port}");
    options
}

fn reading(source: u16) -> HttpScanner {
    HttpScanner::new(
        format!("http://127.0.0.1:{source}/records"),
        RecordMap::new(["data", "items"], "id"),
        Cursor::new(["paging", "next"], "cursor"),
    )
}

/// **A cursor kept by the server takes the next run to where the last
/// one stopped**, and the pages behind it are never requested.
///
/// The value nothing in the chain can read: the scanner writes a token
/// the source issued, the client sends it as text, the service stores
/// it as text, the table holds it as text, and the scanner hands it
/// back to the source. The one thing that understands it is the only
/// thing that ever sees it as more than bytes.
#[tokio::test(flavor = "multi_thread")]
async fn a_cursor_the_server_kept_takes_the_next_run_where_it_left_off() {
    let (source, asked) = spawn_source(vec![
        (vec!["1", "2"], Some("page2")),
        (vec!["3", "4"], Some("page3")),
        (vec!["5", "6"], None),
    ])
    .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-http-import").await;
    let scanner = reading(source);
    let store = HttpSyncStore::new(ApiClient::new(format!("http://127.0.0.1:{port}")));

    let first = run_import_with(
        &scanner,
        &RecordParser,
        ScanMode::Enumerate,
        options(&persona, port),
        Some(&store),
    )
    .await
    .expect("the first run");
    assert_eq!((first.imported, first.failed), (6, 0), "three pages of two");
    assert_eq!(
        asked.cursors(),
        vec![None, Some("page2".into()), Some("page3".into())],
        "and it followed the source's own tokens to the end"
    );

    // What the server kept is the token behind the *last page that
    // offered one* — the final page had none, so the position is
    // before it. That is the honest answer and not an off-by-one: a
    // scanner cannot invent a token to mean "done".
    let key = StateKey::new(
        &persona,
        SourceScanner::partition(&scanner).expect("a URL has a partition"),
    );
    let kept = store
        .read(&key)
        .await
        .expect("reading it back")
        .expect("the point the run stored");
    assert_eq!(
        kept.offset,
        serde_json::json!({ "after_cursor": "page3" }),
        "the token the source issued, not one anybody here made up"
    );

    // A second run over a source with nothing new asks once, from
    // where the first stopped.
    let (source2, asked2) = spawn_source(vec![(vec!["5", "6"], None)]).await;
    let scanner2 = reading(source2);
    // Same partition shape, different port, so the key is the first
    // run's only if the URL matches — which it does not. Point it at
    // the stored token by hand to keep the question about the token
    // rather than about the key.
    let resumed = run_import_with(
        &scanner2,
        &RecordParser,
        ScanMode::Enumerate,
        {
            let mut o = options(&persona, port);
            o.resume = asterism_importer_sdk::Resume::From(asterism_importer_sdk::SyncState::new(
                SourceScanner::partition(&scanner2).expect("a URL has a partition"),
                kept.offset.clone(),
            ));
            o
        },
        Some(&store),
    )
    .await
    .expect("the resumed run");
    assert_eq!(resumed.failed, 0);
    assert_eq!(
        asked2.cursors(),
        vec![Some("page3".into())],
        "one request, carrying the token the server had been keeping"
    );
}

/// A run that reached a rate limit keeps the position it had earned up
/// to that point, and says when the source is expected back.
///
/// The case the whole classification was built for, and the first one
/// in this workspace driven by a source that really answered 429. Both
/// halves matter: the records before the limit are not re-read next
/// time, and the operator is told a time rather than a failure.
#[tokio::test(flavor = "multi_thread")]
async fn a_rate_limited_run_keeps_what_it_reached_and_says_when_to_return() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // A source that serves one page and then refuses.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let source = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let mut served = 0usize;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 8 * 1024];
            let mut request = String::new();
            loop {
                let Ok(n) = socket.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    break;
                }
                request.push_str(&String::from_utf8_lossy(&buf[..n]));
                if request.contains("\r\n\r\n") {
                    break;
                }
            }
            let response = if served == 0 {
                let body =
                    r#"{"data":{"items":[{"id":"1","said":"a"}]},"paging":{"next":"page2"}}"#;
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 429 X\r\nRetry-After: 120\r\nContent-Length: 2\r\n\
                 Connection: close\r\n\r\n{}"
                    .into()
            };
            served += 1;
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });

    let tmp = tempfile::tempdir().expect("tempdir");
    let (core, port) = boot(tmp.path()).await;
    let persona = register(&core, "e2e-http-rate-limited").await;
    let scanner = reading(source);
    let store = HttpSyncStore::new(ApiClient::new(format!("http://127.0.0.1:{port}")));

    let summary = run_import_with(
        &scanner,
        &RecordParser,
        ScanMode::Enumerate,
        options(&persona, port),
        Some(&store),
    )
    .await
    .expect("a run that started is a run that reports");

    assert_eq!(
        summary.imported, 1,
        "the page before the limit still landed"
    );
    let ended_by = summary.ended_by.expect("the limit cost the run");
    assert_eq!(
        ended_by.disposition(),
        asterism_importer_sdk::Disposition::FailedRetryable {
            after: Some(std::time::Duration::from_secs(120))
        },
        "and the wait the source stated reaches a caller: {ended_by}"
    );

    let key = StateKey::new(
        &persona,
        SourceScanner::partition(&scanner).expect("a URL has a partition"),
    );
    assert_eq!(
        store
            .read(&key)
            .await
            .expect("a read")
            .expect("a run cut short still earns a position")
            .offset,
        serde_json::json!({ "after_cursor": "page2" }),
        "so coming back later starts at the page the limit interrupted"
    );
}
