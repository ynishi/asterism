//! Filesystem adapter for `SourceTextReader` — resolves asset
//! locators back to the full text of the original artefact.
//!
//! The DB stores only the 200-char cover snippet; the source of
//! truth stays on disk (Asterism never writes back to a locator).
//! Four locator forms are handled, chosen by the extension of the
//! container path:
//!
//! - `<path>` — plain text file; the whole content, capped.
//! - `<path>.jsonl#<uuid>` — one record inside a JSON-lines
//!   container (Claude Code session logs): the line whose `uuid`
//!   field equals the fragment. The message body is extracted from
//!   the common chat-log shapes (`message.content` as a string, or
//!   as an array of `{type: "text", text}` blocks joined by blank
//!   lines).
//! - `<path>.db#<uuid>` — one row in a persona-journal SQLite
//!   EventLog dump (`entries` + `versions` schema). The current
//!   version's `body` column is returned. Opened read-only so it
//!   never collides with the journal-mcp writer.
//! - `<path>.json#conversation=<conv_id>/message=<msg_id>` — one
//!   nested message inside a single-envelope harvest JSON
//!   (`asterism_agent_harvest` v1). The envelope is parsed once,
//!   then the walk finds `conversations[].id == conv_id` and
//!   `messages[].id == msg_id`.
//!
//! Batching: locators are grouped by container path first, so a
//! 200-message session / db / envelope costs exactly one open.
//! Extraction failures degrade to `None` per locator — the caller
//! falls back to the stored cover instead of failing the whole
//! batch.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};

use asterism_core::domain::repository::{SourceTextReader, TextLocator};
use asterism_core::domain::source_locator::SourceLocator;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::OpenFlags;

/// Upper bound for a whole plain-text file read (chars are counted
/// after UTF-8 decode; this is a display path, not an archival one).
const PLAIN_FILE_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Upper bound for one extracted message body, in characters.
const RECORD_MAX_CHARS: usize = 200_000;

/// Filesystem-backed reader. Stateless; safe to share.
#[derive(Clone, Default)]
pub struct FsSourceTextReader;

impl FsSourceTextReader {
    /// Constructs the reader.
    pub fn new() -> Self {
        Self
    }
}

// `split_locator(&str) -> (&str, Option<&str>)` used to live here. It is
// gone: the locator arrives already taken apart, so a `Record` hands
// over its container and its address as two values instead of this
// function guessing which half of a string is which. It also closes a
// hole of its own — it handed its container half straight to
// `File::open` with no absoluteness test, so `pics/a.jsonl#uuid`
// resolved against whatever directory the process happened to be in.
// A rootless container cannot become a `ContainerRecord` at all.

/// Truncates on a char boundary (mirrors the importer-side helper).
fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((byte_index, _)) => text[..byte_index].to_string(),
        None => text.to_string(),
    }
}

/// Pulls the human-readable body out of one chat-log JSON record.
/// Understands `message.content` as a plain string or as an array of
/// content blocks (text blocks joined; tool-use blocks skipped), and
/// falls back to a top-level `content` / `text` field.
fn extract_body(value: &serde_json::Value) -> Option<String> {
    let content = value
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| value.get("content"))
        .or_else(|| value.get("text"))?;
    match content {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(blocks) => {
            let texts: Vec<&str> = blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        b.get("text").and_then(|t| t.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n\n"))
            }
        }
        _ => None,
    }
}

/// Scans a JSONL container once and returns the bodies of every
/// requested fragment (uuid). Records that fail to parse or carry no
/// text are simply absent from the result map.
fn scan_container(path: &str, fragments: &[&str]) -> HashMap<String, String> {
    let mut found = HashMap::new();
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return found,
    };
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        // Cheap pre-filter before the JSON parse: the uuid literal
        // must appear somewhere in the line.
        let Some(frag) = fragments
            .iter()
            .find(|f| !found.contains_key(**f) && line.contains(*f as &str))
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        // Confirm the match is the record's own uuid, not an
        // incidental substring inside some body text. Bind to a
        // `&str` explicitly — rkyv adds Option PartialEq impls that
        // otherwise leave `Some(frag)` (a `&&String`) with an
        // ambiguous type at the equality site.
        let uuid = value.get("uuid").and_then(|u| u.as_str());
        let frag_str: &str = frag;
        if uuid != Some(frag_str) {
            continue;
        }
        if let Some(body) = extract_body(&value) {
            found.insert((*frag).to_string(), truncate_chars(&body, RECORD_MAX_CHARS));
        }
        if found.len() == fragments.len() {
            break;
        }
    }
    found
}

/// Reads a whole plain-text file (size-capped, lossy UTF-8).
fn read_plain(path: &str) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > PLAIN_FILE_MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Route hint derived from a container path — chooses which
/// per-container reader to use.
enum ContainerShape {
    Jsonl,
    PersonaJournalDb,
    HarvestEnvelope,
}

/// Classifies a container by the extension of its path. The default
/// (`.jsonl` / unknown) is JSONL because that is the original
/// Claude Code session shape the codebase started with.
fn shape_of(path: &str) -> ContainerShape {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".db") {
        ContainerShape::PersonaJournalDb
    } else if lower.ends_with(".json") {
        ContainerShape::HarvestEnvelope
    } else {
        ContainerShape::Jsonl
    }
}

/// Opens a persona-journal `.db` container read-only and returns the
/// body text for every requested entry uuid. Missing rows are simply
/// absent from the returned map (caller falls back to `None`).
///
/// Schema (persona-journal EventLog):
/// - `entries(id, current_version, ...)` — id = uuid text.
/// - `versions(entry_id, version, body, ...)` — one row per revision.
///
/// The join fetches `versions.body` for `entries.current_version`.
/// `SQLITE_OPEN_READ_ONLY` avoids collision with journal-mcp's
/// writer (WAL mode); `busy_timeout` shields against snapshot
/// contention on ~1 s scale.
fn scan_persona_journal_db(path: &str, fragments: &[&str]) -> HashMap<String, String> {
    let mut found = HashMap::new();
    let conn = match rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => return found,
    };
    let _ = conn.busy_timeout(std::time::Duration::from_millis(1_500));
    // One prepared statement, executed per uuid — the request set
    // for a single container is typically small (per-persona
    // batches from `IndexRebuild`), so the round-trip cost is
    // negligible compared with the extra SQL machinery a bulk
    // `IN (?, ?, ?, ...)` would need to build for a dynamic list.
    let mut stmt = match conn.prepare(
        "SELECT v.body \
         FROM entries e \
         JOIN versions v ON v.entry_id = e.id AND v.version = e.current_version \
         WHERE e.id = ? LIMIT 1",
    ) {
        Ok(s) => s,
        Err(_) => return found,
    };
    for frag in fragments {
        let Ok(body) = stmt.query_row([*frag], |row| row.get::<_, String>(0)) else {
            continue;
        };
        found.insert((*frag).to_string(), truncate_chars(&body, RECORD_MAX_CHARS));
    }
    found
}

/// Parses a fragment of the shape
/// `conversation=<conv>/message=<msg>` into its two parts. Returns
/// `None` when the fragment does not match the expected form.
fn parse_harvest_fragment(fragment: &str) -> Option<(&str, &str)> {
    let rest = fragment.strip_prefix("conversation=")?;
    // The message id is emitted after the *last* `/message=` marker
    // so a conv_id containing `/` (harvest lets any string sit
    // there) is captured whole.
    let idx = rest.rfind("/message=")?;
    let conv = &rest[..idx];
    let msg = &rest[idx + "/message=".len()..];
    if conv.is_empty() || msg.is_empty() {
        return None;
    }
    Some((conv, msg))
}

/// Opens a single-envelope harvest JSON container once and returns
/// the body text for every requested `conversation=/message=`
/// fragment. Failures on individual fragments (malformed shape,
/// conv or msg missing) simply skip that entry.
fn scan_harvest_envelope(path: &str, fragments: &[&str]) -> HashMap<String, String> {
    let mut found = HashMap::new();
    let bytes = match std::fs::read(path) {
        Ok(b) if (b.len() as u64) <= PLAIN_FILE_MAX_BYTES => b,
        _ => return found,
    };
    let envelope: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return found,
    };
    let conversations = match envelope.get("conversations").and_then(|c| c.as_array()) {
        Some(arr) => arr,
        None => return found,
    };
    for frag in fragments {
        let Some((conv_id, msg_id)) = parse_harvest_fragment(frag) else {
            continue;
        };
        // Find the target conversation by id.
        let Some(conv) = conversations
            .iter()
            .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(conv_id))
        else {
            continue;
        };
        let Some(messages) = conv.get("messages").and_then(|m| m.as_array()) else {
            continue;
        };
        let Some(msg) = messages
            .iter()
            .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(msg_id))
        else {
            continue;
        };
        let body = match msg.get("body").and_then(|b| b.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        found.insert((*frag).to_string(), truncate_chars(&body, RECORD_MAX_CHARS));
    }
    found
}

#[async_trait]
impl SourceTextReader for FsSourceTextReader {
    async fn read_batch(
        &self,
        locators: &[TextLocator],
    ) -> Result<Vec<Option<String>>, DomainError> {
        // Every element arrived through `TextLocator::new`, so the
        // "is this text?" question is already answered and the plain
        // read below cannot be handed a picture.
        let locators: Vec<SourceLocator> = locators.iter().map(|l| l.locator().clone()).collect();
        // Blocking file IO off the async runtime.
        tokio::task::spawn_blocking(move || {
            // Group record lookups by container path so each container
            // is scanned exactly once. The variant is what says a
            // lookup belongs here — no string is split to find out, and
            // the container is a `LocalPath`, so nothing rootless
            // reaches `File::open`.
            let mut by_container: HashMap<&str, Vec<&str>> = HashMap::new();
            for locator in &locators {
                if let SourceLocator::Record(record) = locator {
                    by_container
                        .entry(record.container().as_str())
                        .or_default()
                        .push(record.record().as_str());
                }
            }
            let mut container_hits: HashMap<String, HashMap<String, String>> = HashMap::new();
            for (path, frags) in &by_container {
                let hits = match shape_of(path) {
                    ContainerShape::Jsonl => scan_container(path, frags),
                    ContainerShape::PersonaJournalDb => scan_persona_journal_db(path, frags),
                    ContainerShape::HarvestEnvelope => scan_harvest_envelope(path, frags),
                };
                container_hits.insert((*path).to_string(), hits);
            }
            locators
                .iter()
                .map(|locator| match locator {
                    SourceLocator::Record(record) => container_hits
                        .get(record.container().as_str())
                        .and_then(|hits| hits.get(record.record().as_str()))
                        .cloned(),
                    SourceLocator::File(path) => read_plain(path.as_str()),
                    // No bytes on this disk to read. The reader used to
                    // hand these to `File::open` anyway and get `None`
                    // from the failure; now it declines to ask.
                    SourceLocator::Remote(_) | SourceLocator::Logical(_) => None,
                })
                .collect()
        })
        .await
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("source text read join error: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every fixture below is a text file this test just wrote, so the
    /// format check the constructor normally performs has nothing to
    /// consult — the declared form is the honest one here.
    fn loc(raw: impl AsRef<str>) -> TextLocator {
        TextLocator::of_known_text(SourceLocator::from_wire(raw.as_ref()).expect("locator"))
    }

    #[tokio::test]
    async fn reads_plain_files_and_jsonl_fragments() {
        let dir = std::env::temp_dir().join(format!("asterism-src-text-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("note.md");
        std::fs::write(&plain, "full note body").unwrap();
        let jsonl = dir.join("session.jsonl");
        std::fs::write(
            &jsonl,
            concat!(
                "{\"uuid\":\"m1\",\"message\":{\"content\":\"hello world\"}}\n",
                "{\"uuid\":\"m2\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"part a\"},{\"type\":\"tool_use\"},{\"type\":\"text\",\"text\":\"part b\"}]}}\n",
            ),
        )
        .unwrap();

        let reader = FsSourceTextReader::new();
        let out = reader
            .read_batch(&[
                loc(plain.to_string_lossy()),
                loc(format!("{}#m2", jsonl.to_string_lossy())),
                loc(format!("{}#m1", jsonl.to_string_lossy())),
                loc(format!("{}#missing", jsonl.to_string_lossy())),
                loc(dir.join("absent.md").to_string_lossy()),
            ])
            .await
            .unwrap();

        assert_eq!(out[0].as_deref(), Some("full note body"));
        assert_eq!(out[1].as_deref(), Some("part a\n\npart b"));
        assert_eq!(out[2].as_deref(), Some("hello world"));
        assert_eq!(out[3], None);
        assert_eq!(out[4], None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The fixture that matters for this reader: a record whose **container path
    /// carries a `#` of its own**, and a rootless container beside it.
    ///
    /// The container is found by the split being `rsplit`, which the
    /// deleted helper also did — so that half alone would pass either
    /// way. The rootless case is what the helper answered wrongly: it
    /// handed its container half straight to `File::open` with no
    /// absoluteness test, so `a.jsonl#m1` resolved against whatever
    /// directory the process happened to be in. A rootless container
    /// cannot become a `ContainerRecord` at all, so the honest answer is
    /// `None` — and the same file *is* readable through its absolute
    /// spelling in the same test, which is what stops the `None` from
    /// being "the fixture was unreadable".
    #[tokio::test]
    async fn a_container_path_carrying_a_hash_resolves_and_a_rootless_one_does_not() {
        let dir = std::env::temp_dir().join(format!("asterism-src-hash-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let hashed = dir.join("a#b.jsonl");
        std::fs::write(
            &hashed,
            "{\"uuid\":\"m1\",\"message\":{\"content\":\"inside a hashed container\"}}\n",
        )
        .unwrap();

        let reader = FsSourceTextReader::new();
        let out = reader
            .read_batch(&[
                loc(format!("{}#m1", hashed.to_string_lossy())),
                // Same container, same record, spelled without a root.
                loc("a#b.jsonl#m1"),
            ])
            .await
            .unwrap();

        assert_eq!(out[0].as_deref(), Some("inside a hashed container"));
        assert_eq!(
            out[1], None,
            "a rootless container is a name, and must not resolve against the process CWD"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn reads_persona_journal_db_fragments() {
        let dir = std::env::temp_dir().join(format!("asterism-pj-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("_journal.db");
        // Build a minimal persona-journal schema and one entry with
        // two versions; the reader should return `versions.body` for
        // `entries.current_version` (= 2).
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE entries (\
                 id TEXT PRIMARY KEY, current_version INTEGER NOT NULL\
             );\
             CREATE TABLE versions (\
                 entry_id TEXT NOT NULL, version INTEGER NOT NULL, body TEXT NOT NULL,\
                 PRIMARY KEY(entry_id, version)\
             );\
             INSERT INTO entries VALUES ('e1', 2);\
             INSERT INTO versions VALUES ('e1', 1, 'old body');\
             INSERT INTO versions VALUES ('e1', 2, 'current body');",
        )
        .unwrap();
        drop(conn);

        let reader = FsSourceTextReader::new();
        let out = reader
            .read_batch(&[
                loc(format!("{}#e1", db_path.to_string_lossy())),
                loc(format!("{}#absent", db_path.to_string_lossy())),
            ])
            .await
            .unwrap();
        assert_eq!(out[0].as_deref(), Some("current body"));
        assert_eq!(out[1], None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn reads_harvest_envelope_fragments() {
        let dir = std::env::temp_dir().join(format!("asterism-hv-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let json_path = dir.join("envelope.json");
        let envelope = serde_json::json!({
            "spec": "asterism_agent_harvest",
            "conversations": [
                {
                    "id": "conv-a",
                    "messages": [
                        {"id": "m1", "body": "first message body"},
                        {"id": "m2", "body": "second message body"},
                    ]
                },
                {
                    "id": "conv-b",
                    "messages": [
                        {"id": "x1", "body": "other conversation"},
                    ]
                }
            ]
        });
        std::fs::write(&json_path, serde_json::to_string(&envelope).unwrap()).unwrap();

        let reader = FsSourceTextReader::new();
        let out = reader
            .read_batch(&[
                loc(format!(
                    "{}#conversation=conv-a/message=m2",
                    json_path.to_string_lossy()
                )),
                loc(format!(
                    "{}#conversation=conv-b/message=x1",
                    json_path.to_string_lossy()
                )),
                loc(format!(
                    "{}#conversation=conv-a/message=missing",
                    json_path.to_string_lossy()
                )),
                loc(format!(
                    "{}#malformed-fragment",
                    json_path.to_string_lossy()
                )),
            ])
            .await
            .unwrap();
        assert_eq!(out[0].as_deref(), Some("second message body"));
        assert_eq!(out[1].as_deref(), Some("other conversation"));
        assert_eq!(out[2], None);
        assert_eq!(out[3], None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
