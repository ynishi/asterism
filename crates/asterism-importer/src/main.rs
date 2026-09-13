//! Unified CLI for all built-in Asterism import adapters.
//!
//! One subcommand per source — Claude Code sessions, character cards,
//! agent-harvest envelopes, arbitrary SQLite queries, persona-journal
//! entries, tapes, image / video / audio files, and written documents.
//! Every subcommand
//! runs the same importer-SDK pipeline: walk the source, parse it into
//! typed footprints, and push them in batches to a running
//! `asterism-server` over HTTP (`--server`, default local). All imports
//! are persona-scoped (`--persona-id`) and support `--dry-run`, which
//! validates and reports without writing anything.

use std::path::PathBuf;

use anyhow::{Context, bail};
use asterism_importer_audio::AudioParser;
use asterism_importer_cc::CcSessionParser;
use asterism_importer_image::ImageParser;
use asterism_importer_persona_journal::PersonaJournalParser;
use asterism_importer_sdk::card::CharaSourceParser;
use asterism_importer_sdk::harvest::{HarvestSourceParser, schema_example_json};
use asterism_importer_sdk::scanner::http::{Cursor, RecordMap};
use asterism_importer_sdk::scanner::sqlite::ColumnMap;
use asterism_importer_sdk::{
    ApiClient, ChatMessage, ChatRole, Doc, DocFormat, Footprint, FootprintSource, FsScanner,
    HttpScanner, HttpSyncStore, ImportOptions, ImportReport, ImportSummary, Note, OccurredSource,
    ParseError, RawItem, ReportedFailure, Resume, ScanMode, SourceError, SourceParser,
    SqliteScanner, SyncState, resolve_occurrence, run_import_with,
};
use asterism_importer_tape::TapeParser;
use asterism_importer_text::TextParser;
use asterism_importer_video::VideoParser;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::Value;

const JOURNAL_QUERY: &str = r#"
SELECT
  e.id                                                              AS id,
  e.kind                                                            AS kind,
  e.seq_in_kind                                                     AS seq_in_kind,
  e.created_at                                                      AS created_at,
  e.updated_at                                                      AS updated_at,
  v.body                                                            AS body,
  (SELECT GROUP_CONCAT(tag, ',') FROM tags WHERE entry_id = e.id)   AS tags
FROM entries e
JOIN versions v
  ON v.entry_id = e.id
 AND v.version  = e.current_version
ORDER BY e.created_at ASC
"#;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "gif", "tiff", "tif", "bmp",
];
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "webm", "m4v", "mkv", "avi"];
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "wav", "flac", "ogg", "opus", "oga", "aac", "aiff",
];
/// The spellings a person writes a document in, out of the four
/// `guess_mime` names `text/plain`.
///
/// Deliberately not `.markdown`, and the reason is #259's first rung:
/// the map has to name every extension a scanner accepts, or a file
/// imports with no format at all. That makes the map's four the ceiling
/// and not the list. `.jsonl` and `.db` are the two left out — a file
/// records are read out of, with importers of their own, rather than a
/// file that is one document.
///
/// `.txt` is also the tape route's extension, and that overlap is
/// deliberate rather than settled here: a terminal transcript and a
/// note are different things wearing one suffix, and which one a
/// directory holds is what picking the subcommand says.
const TEXT_EXTENSIONS: &[&str] = &["md", "txt"];

#[derive(Debug, Parser)]
#[command(
    name = "asterism-import",
    version,
    about = "Import external footprints into Asterism"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Import Claude Code JSONL sessions.
    Cc(CcArgs),
    /// Import character-card PNG, JSON or `.charx` files.
    Chara(CharaArgs),
    /// Import canonical agent-harvest JSON envelopes.
    Harvest(HarvestArgs),
    /// Import rows from an arbitrary SQLite query.
    Sqlite(Box<SqliteArgs>),
    /// Import records from a paginated JSON HTTP endpoint.
    Http(Box<HttpArgs>),
    /// Import persona-journal EventLog entries.
    #[command(alias = "persona-journal")]
    Journal(JournalArgs),
    /// Import Persona Tape terminal-session transcripts.
    Tape(TapeArgs),
    /// Import image files and their metadata.
    Image(ImageArgs),
    /// Import video files and their metadata.
    Video(MediaArgs),
    /// Import audio files and their metadata.
    Audio(MediaArgs),
    /// Import Markdown or plain-text files, one document per file.
    Text(TextArgs),
}

#[derive(Debug, Clone, Args)]
struct CommonArgs {
    /// Persona id every emitted footprint belongs to.
    #[arg(long)]
    persona_id: String,
    /// Asterism HTTP API base URL.
    #[arg(long, default_value = "http://127.0.0.1:8989")]
    server: String,
    /// Number of assets sent per add-batch request.
    #[arg(long, default_value_t = 50)]
    batch_size: usize,
    /// Validate and report without contacting Asterism at all.
    ///
    /// Nothing is written, and nothing is asked either — including
    /// where the last run stopped. So a dry run walks the whole source
    /// rather than the part an ordinary run would, which is the price
    /// of a rehearsal that needs nothing running.
    #[arg(long)]
    dry_run: bool,
    /// Materialise the source directory hierarchy after each batch.
    #[arg(long)]
    auto_organize_base_dir: Option<String>,
    /// Take up from this point instead of the stored one.
    ///
    /// Rarely needed: a run keeps its own position and the next one
    /// finds it. This is for putting the position somewhere a run would
    /// not have — back before a batch that needs importing again, say.
    /// What a person types beats what a machine stored, and the run
    /// stores the point it then earns.
    ///
    /// What the JSON means belongs to the scanner that wrote it, and a
    /// scanner handed one it cannot use refuses the scan rather than
    /// quietly starting over.
    #[arg(long, value_parser = parse_sync_state, conflicts_with = "no_resume")]
    resume_from: Option<SyncState>,
    /// Read the whole source, and leave the stored position alone.
    ///
    /// For re-reading a source after a parser changed, or checking what
    /// a run would do. It does not forget where the import was: the
    /// next ordinary run takes up from the same place it would have.
    #[arg(long)]
    no_resume: bool,
    /// Write this run's outcome to a file, as JSON.
    ///
    /// For whatever started this process. The progress lines above are
    /// for a person and are free to change wording; this is a contract
    /// between two binaries, and a supervisor scraping "done — ok=3
    /// err=0" would break the day somebody improved that sentence — and
    /// break silently, reporting zero.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Send this header with the credential in `$ASTERISM_IMPORT_SECRET`.
    ///
    /// `--header-secret Authorization` sends `Authorization: <value of
    /// that variable>`. For a supervisor that resolved the credential
    /// and must not put it on a command line: an argument vector is
    /// readable by every other process on the machine, and this way
    /// what they can read is the name of a header.
    ///
    /// A person running this by hand wants `--header` instead.
    #[arg(long)]
    header_secret: Option<String>,
}

impl CommonArgs {
    fn options(&self) -> ImportOptions {
        ImportOptions {
            persona_id: self.persona_id.clone(),
            server: self.server.clone(),
            batch_size: self.batch_size.max(1),
            upload_concurrency: 1,
            dry_run: self.dry_run,
            auto_organize_base_dir: self.auto_organize_base_dir.clone(),
            resume: resume_policy(self.resume_from.clone(), self.no_resume),
        }
    }
}

/// The three answers the two flags spell.
///
/// Clap refuses both at once, so the order here is not a precedence
/// anybody can trip over — it is a total function over the states that
/// can arrive.
fn resume_policy(resume_from: Option<SyncState>, no_resume: bool) -> Resume {
    match (resume_from, no_resume) {
        (_, true) => Resume::No,
        (Some(state), false) => Resume::From(state),
        (None, false) => Resume::Stored,
    }
}

/// The variable a supervisor puts this run's credential in.
///
/// Named to match `asterism_infra::import_launcher::CHILD_SECRET_VAR`,
/// which is the only thing that sets it. A person running this by hand
/// uses `--header` and never sees it.
const SECRET_VAR: &str = "ASTERISM_IMPORT_SECRET";

/// Writes what the run did, for whatever started it.
///
/// Carries the **class** of the failure that ended a run and not only
/// its message: the class is what a caller acts on, and it is the part
/// a scheduler needs in order to know whether running again is worth
/// anything. A supervisor given a message alone would have to parse
/// English to find out it had been rate limited.
fn write_report(path: &std::path::Path, summary: &ImportSummary) -> anyhow::Result<()> {
    let ended_by = summary.ended_by.as_ref().map(|err| ReportedFailure {
        class: match err {
            SourceError::Config(_) => "config",
            SourceError::Transient(_) => "transient",
            SourceError::RateLimited { .. } => "rate_limited",
            SourceError::Source(_) => "source",
            SourceError::Item { .. } => "item",
        }
        .into(),
        message: err.to_string(),
        retry_after_secs: match err {
            SourceError::RateLimited { retry_after, .. } => retry_after.map(|wait| wait.as_secs()),
            _ => None,
        },
    });
    let report = ImportReport {
        imported: summary.imported,
        failed: summary.failed,
        ended_by,
    };
    std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

/// Reads a resumption point typed on the command line.
///
/// Read as a whole and not field by field: the partition is half of
/// what makes a state answerable, and a caller who could supply an
/// offset alone would be able to point one source's position at
/// another's.
fn parse_sync_state(raw: &str) -> Result<SyncState, String> {
    serde_json::from_str(raw).map_err(|err| format!("not a resumption point: {err}"))
}

#[derive(Debug, Args)]
struct CcArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Claude Code projects directory. Defaults to ~/.claude/projects.
    #[arg(long)]
    dir: Option<PathBuf>,
    /// Continue watching for changed session files.
    #[arg(long)]
    watch: bool,
}

#[derive(Debug, Args)]
struct CharaArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[arg(long)]
    dir: PathBuf,
    #[arg(long, default_value = "chara")]
    source_kind: String,
    #[arg(long)]
    platform: Option<String>,
    #[arg(long)]
    watch: bool,
}

#[derive(Debug, Args)]
struct HarvestArgs {
    /// Print the canonical harvest schema and exit.
    #[arg(long, exclusive = true)]
    print_schema: bool,
    #[arg(long, required_unless_present = "print_schema")]
    persona_id: Option<String>,
    #[arg(long, required_unless_present = "print_schema")]
    dir: Option<PathBuf>,
    #[arg(long, default_value = "http://127.0.0.1:8989")]
    server: String,
    #[arg(long, default_value_t = 50)]
    batch_size: usize,
    #[arg(long, default_value = "agent-harvest")]
    source_kind: String,
    #[arg(long)]
    platform: Option<String>,
    #[arg(long)]
    watch: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    auto_organize_base_dir: Option<String>,
    /// Take up from this point instead of the stored one. See
    /// `CommonArgs`.
    #[arg(long, value_parser = parse_sync_state, conflicts_with = "no_resume")]
    resume_from: Option<SyncState>,
    /// Read the whole source, and leave the stored position alone. See
    /// `CommonArgs`.
    #[arg(long)]
    no_resume: bool,
    /// Write this run's outcome to a file, as JSON. See `CommonArgs`.
    #[arg(long)]
    report: Option<PathBuf>,
}

/// The generic HTTP source: a URL, where the records are in the
/// answer, and how the next page is asked for.
///
/// Every flag here is something only the person pointing it at a
/// service can know. There is no per-service knowledge in the binary
/// and no list of supported APIs — a service is a set of these values,
/// which is what makes a thirtieth adapter cost a shell script rather
/// than a release.
#[derive(Debug, Args)]
struct HttpArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// First request's URL, query string included.
    ///
    /// Whole, because parameters choose what comes back — a `since=` or
    /// a `type=` makes it a different set of records, and the stored
    /// position is filed under the URL for exactly that reason.
    ///
    /// **Not where a credential goes.** The whole URL, query string
    /// included, is written into the stored position and into every
    /// failure message; `--header` is neither. A token put in a query
    /// parameter is a token in the database and in the logs.
    #[arg(long)]
    url: String,
    /// Header sent with every request, as `Name: value`. Repeatable.
    ///
    /// Where a credential goes. Nothing here knows an auth scheme, so
    /// whatever the service wants is spelled out by whoever knows it.
    #[arg(long = "header", value_parser = parse_header)]
    headers: Vec<(String, String)>,
    /// Dotted path to the array of records in the response
    /// (`data.items`). Omit when the body is itself the array.
    #[arg(long, default_value = "")]
    items_path: String,
    /// Field on each record holding its id.
    ///
    /// It becomes half of the locator, which is what the server reads
    /// for idempotency — so it has to be stable for the life of the
    /// record. A field the service calls `position` or `index` is not.
    #[arg(long, default_value = "id")]
    id_field: String,
    /// Field holding the record's occurrence time: RFC 3339 text, or an
    /// integer read as unix epoch milliseconds.
    #[arg(long)]
    ts_field: Option<String>,
    /// Dotted path to the next-page token in the response
    /// (`paging.next`).
    #[arg(long)]
    cursor_path: String,
    /// Query parameter the next-page token is sent back as.
    #[arg(long, default_value = "cursor")]
    cursor_param: String,
    /// Slug written to every footprint's source kind.
    ///
    /// Name the service here. It leads the stored position's partition,
    /// and the server's identity for a record is this slug together
    /// with the locator — so two HTTP imports in one persona stay
    /// apart, and the slug has to stay the same across releases of this
    /// importer or every record it already landed becomes a new one.
    #[arg(long, default_value = "http")]
    source_kind: String,
}

/// Splits `Name: value`, which is how a header is written everywhere
/// else a person writes one.
fn parse_header(raw: &str) -> Result<(String, String), String> {
    match raw.split_once(':') {
        Some((name, value)) if !name.trim().is_empty() => {
            Ok((name.trim().to_string(), value.trim().to_string()))
        }
        _ => Err(format!("not a header: {raw:?}, expected `Name: value`")),
    }
}

/// Splits a dotted path, treating the empty string as no path at all.
///
/// `""` is "the body is itself the thing", which is a real shape and
/// not an omission — so it cannot be spelled as one empty segment.
fn dotted(path: &str) -> Vec<String> {
    if path.is_empty() {
        return Vec::new();
    }
    path.split('.').map(str::to_string).collect()
}

#[derive(Debug, Args)]
struct TapeArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[arg(long)]
    dir: PathBuf,
    #[arg(long, default_value = "persona-tape")]
    source_kind: String,
    #[arg(long, default_value = "Claude Code Tape")]
    platform: String,
}

#[derive(Debug, Args)]
struct JournalArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[arg(long)]
    persona_name: String,
    #[arg(long)]
    persona_journal_root: Option<PathBuf>,
    #[arg(long)]
    db_path: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ImageArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Image directory. Defaults to ~/Pictures.
    #[arg(long)]
    dir: Option<PathBuf>,
    #[arg(long, value_delimiter = ',')]
    extensions: Vec<String>,
    #[arg(long)]
    platform: Option<String>,
    #[arg(long, default_value_t = 4)]
    upload_concurrency: usize,
    /// Continue watching for new or changed image files.
    #[arg(long)]
    watch: bool,
}

#[derive(Debug, Args)]
struct MediaArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[arg(long)]
    dir: PathBuf,
    #[arg(long)]
    platform: Option<String>,
    #[arg(long)]
    source_kind: Option<String>,
    /// Continue watching for new or changed media files. Video and
    /// audio round-trip through generators the same way images do, so
    /// their return trips deserve the same waiting-room.
    #[arg(long)]
    watch: bool,
}

/// No `watch`, and the reason is not that re-running would do instead.
/// It would not: `AssetService::add` returns the row already held for a
/// `(persona, source_kind, locator)` and stops there, above the index
/// rebuild and the hash, so a second arrival of an edited document
/// changes nothing. What this route does is bring a document in; that
/// it does not keep one current is a limitation rather than a schedule.
///
/// Watching would not close that either — it would call `add` more
/// often against the same guard. What would is a route that means "this
/// file changed", which is a different verb from this one.
#[derive(Debug, Clone, Args)]
struct TextArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[arg(long)]
    dir: PathBuf,
    #[arg(long)]
    platform: Option<String>,
    #[arg(long)]
    source_kind: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SqliteKind {
    Note,
    ChatMessage,
    Doc,
}

#[derive(Debug, Args)]
struct SqliteArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[arg(long)]
    db_path: PathBuf,
    #[arg(long)]
    query: String,
    #[arg(long, default_value = "id")]
    id_column: String,
    #[arg(long, default_value = "body")]
    body_column: String,
    #[arg(long)]
    ts_column: Option<String>,
    #[arg(long, value_enum, default_value_t = SqliteKind::Note)]
    kind: SqliteKind,
    #[arg(long, required_if_eq("kind", "chat-message"))]
    session_column: Option<String>,
    #[arg(long)]
    role_column: Option<String>,
    #[arg(long)]
    title_column: Option<String>,
    #[arg(long)]
    doc_format: Option<String>,
    #[arg(long)]
    label: Vec<String>,
    #[arg(long)]
    platform: Option<String>,
    #[arg(long)]
    source_app: Option<String>,
    #[arg(long, default_value = "sqlite")]
    source_kind: String,
    /// Declare that `--query` returns rows in ascending order of
    /// `--id-column`, which is what makes the scan resumable.
    ///
    /// Only you can say it: the query is yours. Without it the run
    /// prints no resumption point and refuses `--resume-from`, rather
    /// than resuming inside an order nobody promised and skipping rows
    /// it never read.
    #[arg(long)]
    ordered_by_id: bool,
}

struct SqliteRowParser<'a> {
    args: &'a SqliteArgs,
}

impl SourceParser for SqliteRowParser<'_> {
    fn parse(&self, raw: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let body = std::str::from_utf8(&raw.payload)
            .map_err(|err| ParseError::Malformed {
                locator: raw.locator.clone(),
                message: format!("row body is not UTF-8: {err}"),
            })?
            .to_owned();
        // The scanner lifted the row's own timestamp column, which is a
        // stamp the record states — rung 1, not the container's mtime.
        let (occurred_at, occurred_source) =
            resolve_occurrence(raw.occurred_at, OccurredSource::Record, None);
        let source = FootprintSource {
            kind: raw.source_kind,
            locator: raw.locator,
            platform: self.args.platform.clone(),
            external_id: None,
        };
        let extra = raw.extra;
        let mut labels = vec!["sqlite".to_string()];
        labels.extend(self.args.label.iter().cloned());
        let footprint = match self.args.kind {
            SqliteKind::Note => Footprint::Note(Note {
                source,
                occurred_at,
                occurred_source,
                body,
                source_app: self.args.source_app.clone(),
                labels,
                bundle_id: None,
                extra,
            }),
            SqliteKind::ChatMessage => {
                let column = self.args.session_column.as_deref().ok_or_else(|| {
                    ParseError::Other(anyhow::anyhow!(
                        "--session-column is required for chat-message"
                    ))
                })?;
                let external_session_key = string_from_extra(&extra, column).ok_or_else(|| {
                    ParseError::Other(anyhow::anyhow!("session column {column:?} missing or null"))
                })?;
                let role = self
                    .args
                    .role_column
                    .as_deref()
                    .and_then(|name| string_from_extra(&extra, name))
                    .map(chat_role)
                    .unwrap_or(ChatRole::User);
                Footprint::ChatMessage(ChatMessage {
                    source,
                    occurred_at,
                    occurred_source,
                    external_session_key,
                    role,
                    body,
                    thread_position: None,
                    parent_message_id: None,
                    labels,
                    extra,
                })
            }
            SqliteKind::Doc => Footprint::Doc(Doc {
                source,
                occurred_at,
                occurred_source,
                title: self
                    .args
                    .title_column
                    .as_deref()
                    .and_then(|name| string_from_extra(&extra, name)),
                excerpt: body,
                format: self
                    .args
                    .doc_format
                    .as_deref()
                    .map(doc_format)
                    .unwrap_or(DocFormat::Plain),
                bundle_id: None,
                file_size_bytes: None,
                word_count: None,
                labels,
                extra,
            }),
        };
        Ok(vec![footprint])
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Cc(args) => {
            let root = args
                .dir
                .unwrap_or_else(|| home_dir().join(".claude/projects"));
            let scanner = FsScanner::new(root).with_extensions(["jsonl"]);
            run(
                "cc",
                &scanner,
                &CcSessionParser,
                args.watch,
                args.common.options(),
                args.common.report.clone(),
            )
            .await
        }
        Command::Chara(args) => {
            let scanner = FsScanner::new(args.dir)
                .with_extensions(["png", "json", "charx"])
                .with_source_kind(args.source_kind);
            let mut parser = CharaSourceParser::new();
            if let Some(platform) = args.platform {
                parser = parser.with_platform(platform);
            }
            run(
                "chara",
                &scanner,
                &parser,
                args.watch,
                args.common.options(),
                args.common.report.clone(),
            )
            .await
        }
        Command::Harvest(args) => run_harvest(args).await,
        Command::Sqlite(args) => run_sqlite(*args).await,
        Command::Journal(args) => {
            let db_path = args.db_path.unwrap_or_else(|| {
                args.persona_journal_root
                    .unwrap_or_else(|| home_dir().join(".persona-journal"))
                    .join(&args.persona_name)
                    .join("_journal.db")
            });
            let columns = ColumnMap::new("id", "body").with_timestamp("created_at");
            // No `ordered_by_id`, and not by oversight: `JOURNAL_QUERY`
            // orders by `created_at`, which is not the order a
            // resumption after a row id would take up inside. So this
            // importer prints no resumption point and refuses
            // `--resume-from`, which is the truthful pair.
            let scanner = SqliteScanner::new(db_path, JOURNAL_QUERY, columns)
                .with_source_kind("persona-journal");
            let parser = PersonaJournalParser {
                persona_name: args.persona_name,
            };
            run(
                "journal",
                &scanner,
                &parser,
                false,
                args.common.options(),
                args.common.report.clone(),
            )
            .await
        }
        Command::Http(args) => {
            let mut records = RecordMap::new(dotted(&args.items_path), &args.id_field);
            if let Some(field) = &args.ts_field {
                records = records.with_timestamp(field);
            }
            let mut scanner = HttpScanner::new(
                &args.url,
                records,
                Cursor::new(dotted(&args.cursor_path), &args.cursor_param),
            )
            .with_source_kind(&args.source_kind);
            for (name, value) in &args.headers {
                scanner = scanner.with_header(name, value);
            }
            // The supervisor's route for a credential: it resolved the
            // variable the definition named and put the value here,
            // where an argument vector cannot leak it.
            if let Some(header) = &args.common.header_secret {
                let value = std::env::var(SECRET_VAR).with_context(|| {
                    format!(
                        "--header-secret {header} needs the credential in ${SECRET_VAR}, \
                         and it is not set"
                    )
                })?;
                scanner = scanner.with_header(header, value);
            }
            let parser = HttpRecordParser {
                source_kind: args.source_kind.clone(),
            };
            run(
                "http",
                &scanner,
                &parser,
                false,
                args.common.options(),
                args.common.report.clone(),
            )
            .await
        }
        Command::Tape(args) => {
            let scanner = FsScanner::new(args.dir)
                .with_extensions(["txt"])
                .with_source_kind(args.source_kind);
            let parser = TapeParser::new(Some(args.platform));
            run(
                "tape",
                &scanner,
                &parser,
                false,
                args.common.options(),
                args.common.report.clone(),
            )
            .await
        }
        Command::Image(args) => {
            let root = args.dir.unwrap_or_else(|| home_dir().join("Pictures"));
            let extensions = if args.extensions.is_empty() {
                IMAGE_EXTENSIONS.iter().map(ToString::to_string).collect()
            } else {
                args.extensions
            };
            let scanner = FsScanner::new(root).with_extensions(extensions);
            let parser = ImageParser::new(args.platform);
            let mut options = args.common.options();
            options.upload_concurrency = args.upload_concurrency.max(1);
            run(
                "image",
                &scanner,
                &parser,
                args.watch,
                options,
                args.common.report.clone(),
            )
            .await
        }
        Command::Video(args) => {
            let scanner = FsScanner::new(args.dir)
                .with_extensions(VIDEO_EXTENSIONS.iter().copied())
                .with_source_kind(args.source_kind.unwrap_or_else(|| "video".into()));
            let parser = VideoParser::new(args.platform);
            run(
                "video",
                &scanner,
                &parser,
                args.watch,
                args.common.options(),
                args.common.report.clone(),
            )
            .await
        }
        Command::Audio(args) => {
            let scanner = FsScanner::new(args.dir)
                .with_extensions(AUDIO_EXTENSIONS.iter().copied())
                .with_source_kind(args.source_kind.unwrap_or_else(|| "audio".into()));
            let parser = AudioParser::new(args.platform);
            run(
                "audio",
                &scanner,
                &parser,
                args.watch,
                args.common.options(),
                args.common.report.clone(),
            )
            .await
        }
        Command::Text(args) => {
            let scanner = FsScanner::new(args.dir)
                .with_extensions(TEXT_EXTENSIONS.iter().copied())
                .with_source_kind(args.source_kind.unwrap_or_else(|| "text".into()));
            let parser = TextParser::new(args.platform);
            run(
                "text",
                &scanner,
                &parser,
                false,
                args.common.options(),
                args.common.report.clone(),
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run<S, P>(
    name: &str,
    scanner: &S,
    parser: &P,
    watch: bool,
    options: ImportOptions,
    report: Option<PathBuf>,
) -> anyhow::Result<()>
where
    S: asterism_importer_sdk::SourceScanner + ?Sized,
    P: SourceParser + ?Sized,
{
    let mode = if watch {
        ScanMode::Watch
    } else {
        ScanMode::Enumerate
    };
    // The store is the same server the records go to, over the same
    // client. That is the whole of the transport decision: an adapter
    // runs itself, pushes what it read, and keeps its position in the
    // place it was already talking to — so nothing has to start it, and
    // nothing has to read its output, for the import to be incremental.
    //
    // Withheld under `--dry-run`, which is a promise about the whole
    // process and not only about writes: the run skips the health
    // probe, and a rehearsal that then failed on an unreachable store
    // would be a rehearsal that needed the thing it was standing in
    // for. The cost is stated where the flag is: a dry run walks the
    // source from the beginning, because asking where the last one
    // stopped is the contact it is not making.
    let dry_run = options.dry_run;
    let report_path = report.clone();
    let store = (!dry_run).then(|| HttpSyncStore::new(ApiClient::new(options.server.clone())));
    let kept = !dry_run && !matches!(options.resume, Resume::No);
    let summary = run_import_with(
        scanner,
        parser,
        mode,
        options,
        store
            .as_ref()
            .map(|s| s as &dyn asterism_importer_sdk::SyncStore),
    )
    .await?;
    // Written before anything below can end this function early, so a
    // run that failed still reports what it managed. A supervisor left
    // with no report has to say the importer died, which is a different
    // and less useful thing than "it was rate limited after 40
    // records".
    if let Some(path) = &report_path {
        write_report(path, &summary)
            .with_context(|| format!("writing the run's report to {}", path.display()))?;
    }
    // Printed for every run that started, including one a failure cut
    // short: the counts are what it managed, and they are worth having
    // either way.
    eprintln!(
        "\nasterism-import {name}: done — ok={} err={}",
        summary.imported, summary.failed
    );
    // Two lines for two situations, and the difference is whether the
    // point this run earned went anywhere.
    //
    // When it was stored, saying so is enough: the next run finds it
    // and nobody has to carry anything. Printing the JSON every time
    // would be a line of noise per run for a value nobody copies.
    //
    // When it was not — a dry run, or `--no-resume` — this line is the
    // only place it exists, so it is printed in full. Printed before
    // the failure below, so a run cut short still leaves it where an
    // operator will look; that is the case it is most wanted in.
    if let Some(state) = &summary.resume_from {
        if kept {
            eprintln!(
                "asterism-import {name}: position kept at {}",
                state.partition
            );
        } else {
            eprintln!(
                "asterism-import {name}: not stored ({}); to take up here, \
                 pass --resume-from '{}'",
                if dry_run { "dry run" } else { "--no-resume" },
                serde_json::to_string(state).expect("a state this crate built serialises")
            );
        }
    }
    // Checked before the count, so it is the failure the exit names:
    // "the source refused the credential" tells an operator what to do
    // next and "3 failed item(s)" does not. The counts are printed
    // above either way, because they are what the run managed.
    if let Some(err) = summary.ended_by {
        bail!("{name} import stopped: {err}");
    }
    if summary.failed > 0 {
        bail!(
            "{name} import completed with {} failed item(s)",
            summary.failed
        );
    }
    Ok(())
}

async fn run_harvest(args: HarvestArgs) -> anyhow::Result<()> {
    if args.print_schema {
        print!("{}", schema_example_json());
        return Ok(());
    }
    let persona_id = args.persona_id.context("--persona-id is required")?;
    let dir = args.dir.context("--dir is required")?;
    let scanner = FsScanner::new(dir)
        .with_extensions(["json"])
        .with_source_kind(args.source_kind);
    let mut parser = HarvestSourceParser::new();
    if let Some(platform) = args.platform {
        parser = parser.with_platform(platform);
    }
    let options = ImportOptions {
        persona_id,
        server: args.server,
        batch_size: args.batch_size.max(1),
        upload_concurrency: 1,
        dry_run: args.dry_run,
        auto_organize_base_dir: args.auto_organize_base_dir,
        resume: resume_policy(args.resume_from, args.no_resume),
    };
    run(
        "harvest",
        &scanner,
        &parser,
        args.watch,
        options,
        args.report.clone(),
    )
    .await
}

async fn run_sqlite(args: SqliteArgs) -> anyhow::Result<()> {
    let mut columns = ColumnMap::new(&args.id_column, &args.body_column);
    if let Some(timestamp) = args.ts_column.clone() {
        columns = columns.with_timestamp(timestamp);
    }
    let mut scanner = SqliteScanner::new(&args.db_path, &args.query, columns)
        .with_source_kind(args.source_kind.clone());
    if args.ordered_by_id {
        scanner = scanner.ordered_by_id();
    }
    let parser = SqliteRowParser { args: &args };
    run(
        "sqlite",
        &scanner,
        &parser,
        false,
        args.common.options(),
        args.common.report.clone(),
    )
    .await
}

/// One note per record, carrying the record's own JSON as its body.
///
/// The least this importer can do and still be an importer: a generic
/// HTTP source has no modality to infer and no field this binary could
/// read as a title, so the record goes in whole and the reading of it
/// belongs to whatever looks at it later. A service worth a typed
/// parser gets one; this is what every other service gets for free.
struct HttpRecordParser {
    source_kind: String,
}

impl SourceParser for HttpRecordParser {
    fn parse(&self, raw: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let body = String::from_utf8(raw.payload).map_err(|e| ParseError::Malformed {
            locator: raw.locator.clone(),
            message: format!("record is not UTF-8: {e}"),
        })?;
        // Rung 1, not rung 2: the stamp came out of a field of the
        // record itself, the way `SqliteScanner`'s comes out of a
        // column. `RawItem::occurred_at` being the scanner's doing does
        // not make it the *container's* time — a JSON record's
        // `created_at` is the record stating when it happened, and
        // recording it as an mtime would put a rung it did not come
        // from on the row.
        let (occurred_at, occurred_source) =
            resolve_occurrence(raw.occurred_at, OccurredSource::Record, None);
        Ok(vec![Footprint::Note(Note {
            source: FootprintSource {
                kind: self.source_kind.clone(),
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

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn string_from_extra(extra: &Value, key: &str) -> Option<String> {
    match extra.as_object()?.get(key)? {
        Value::String(value) => Some(value.clone()),
        Value::Null => None,
        value => Some(value.to_string()),
    }
}

fn chat_role(value: String) -> ChatRole {
    match value.to_ascii_lowercase().as_str() {
        "user" => ChatRole::User,
        "assistant" | "ai" | "bot" => ChatRole::Assistant,
        "system" => ChatRole::System,
        "tool" | "tool_use" | "tool_result" => ChatRole::Tool,
        _ => ChatRole::Other(value),
    }
}

fn doc_format(value: &str) -> DocFormat {
    match value {
        "markdown" | "md" => DocFormat::Markdown,
        "pdf" => DocFormat::Pdf,
        "html" => DocFormat::Html,
        "plain" | "text" | "txt" => DocFormat::Plain,
        value if value.starts_with("code:") => DocFormat::Code(value[5..].to_string()),
        value => DocFormat::Other(value.to_string()),
    }
}
