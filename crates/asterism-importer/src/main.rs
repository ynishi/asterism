//! Unified CLI for all built-in Asterism import adapters.
//!
//! One subcommand per source — Claude Code sessions, character cards,
//! agent-harvest envelopes, arbitrary SQLite queries, persona-journal
//! entries, tapes, and image / video / audio files. Every subcommand
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
use asterism_importer_sdk::scanner::sqlite::ColumnMap;
use asterism_importer_sdk::{
    ChatMessage, ChatRole, Doc, DocFormat, Footprint, FootprintSource, FsScanner, ImportOptions,
    Note, ParseError, RawItem, ScanMode, SourceParser, SqliteScanner, run_import,
};
use asterism_importer_tape::TapeParser;
use asterism_importer_video::VideoParser;
use chrono::Utc;
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
    /// Import character-card PNG or JSON files.
    Chara(CharaArgs),
    /// Import canonical agent-harvest JSON envelopes.
    Harvest(HarvestArgs),
    /// Import rows from an arbitrary SQLite query.
    Sqlite(Box<SqliteArgs>),
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
    /// Validate and report without writing to Asterism.
    #[arg(long)]
    dry_run: bool,
    /// Materialise the source directory hierarchy after each batch.
    #[arg(long)]
    auto_organize_base_dir: Option<String>,
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
        }
    }
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
        let occurred_at = raw.occurred_at.unwrap_or_else(Utc::now);
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
            )
            .await
        }
        Command::Chara(args) => {
            let scanner = FsScanner::new(args.dir)
                .with_extensions(["png", "json"])
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
            let scanner = SqliteScanner::new(db_path, JOURNAL_QUERY, columns)
                .with_source_kind("persona-journal");
            let parser = PersonaJournalParser {
                persona_name: args.persona_name,
            };
            run("journal", &scanner, &parser, false, args.common.options()).await
        }
        Command::Tape(args) => {
            let scanner = FsScanner::new(args.dir)
                .with_extensions(["txt"])
                .with_source_kind(args.source_kind);
            let parser = TapeParser::new(Some(args.platform));
            run("tape", &scanner, &parser, false, args.common.options()).await
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
            run("image", &scanner, &parser, args.watch, options).await
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
            )
            .await
        }
    }
}

async fn run<S, P>(
    name: &str,
    scanner: &S,
    parser: &P,
    watch: bool,
    options: ImportOptions,
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
    let summary = run_import(scanner, parser, mode, options).await?;
    eprintln!(
        "\nasterism-import {name}: done — ok={} err={}",
        summary.imported, summary.failed
    );
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
    };
    run("harvest", &scanner, &parser, args.watch, options).await
}

async fn run_sqlite(args: SqliteArgs) -> anyhow::Result<()> {
    let mut columns = ColumnMap::new(&args.id_column, &args.body_column);
    if let Some(timestamp) = args.ts_column.clone() {
        columns = columns.with_timestamp(timestamp);
    }
    let scanner = SqliteScanner::new(&args.db_path, &args.query, columns)
        .with_source_kind(args.source_kind.clone());
    let parser = SqliteRowParser { args: &args };
    run("sqlite", &scanner, &parser, false, args.common.options()).await
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
