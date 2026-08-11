//! Claude Code JSONL → `Footprint::ChatMessage` (+ `Footprint::Image`)
//! parser.
//!
//! Each `*.jsonl` file under `~/.claude/projects/<slug>/` is one Claude
//! Code session. We emit one `Footprint::ChatMessage` per `user` /
//! `assistant` line with extractable text content. Non-text records
//! (mode, file-history-snapshot, tool_use / tool_result blocks with no
//! text) are skipped.
//!
//! Images that entered the conversation (asset-model v4 P4): Claude
//! Code renders a pasted / referenced image into the message text as a
//! `[Image: source: <absolute path>]` marker — pasted screenshots get
//! a durable copy under `~/.claude/image-cache/<session>/`, file
//! references keep their original path. Each unique marker path emits
//! one `Footprint::Image` carrying the **same `external_session_key`**
//! as the messages, so the image lands as a member of the same Session
//! composite (membership is modality-agnostic). The base64 content
//! blocks in the same lines are ignored — they have no durable locator
//! of their own and the marker already points at the on-disk truth.
//!
//! The `FootprintSource.locator` is `<file-path>#<line-uuid>` so
//! re-imports collapse to no-ops via the server-side
//! `asset.source_kind + source_locator` unique constraint. Image
//! footprints use the marker path itself as the locator — the same
//! constraint therefore assigns an image pasted into several sessions
//! to the first session imported (one asset per original file).
//!
//! A line that carries no `uuid` is **not** given one made from its
//! line number: it has no address, so no `Footprint::ChatMessage` is
//! emitted for it and the drop is counted
//! ([`asterism_importer_sdk::RecordAddresses`], which states the rule
//! and reports the count). An image marker on such a line still
//! becomes a `Footprint::Image` — its locator is the marker path, an
//! address the source really does state, and dropping the whole line
//! would take the picture down with the missing id.

use std::collections::HashSet;
use std::path::PathBuf;

use asterism_importer_sdk::{
    ChatMessage, ChatRole, Footprint, FootprintSource, Image, ParseError, RawItem, RecordAddresses,
    SourceParser,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

/// Parser for Claude Code session JSONL logs.
pub struct CcSessionParser;

impl SourceParser for CcSessionParser {
    fn parse(&self, item: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let path = PathBuf::from(&item.locator);
        // Claude Code stores one session per JSONL file, keyed by the
        // filename stem (session UUID). We hand that stem to the
        // server as the `external_session_key`; the server resolves
        // it to a `Session.id` via
        // `SessionService::find_or_create_by_external_key` so
        // re-imports of the same file converge on the same Session.
        let external_session_key = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| item.locator.clone());
        let project_slug = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let text = std::str::from_utf8(&item.payload).map_err(|e| ParseError::Malformed {
            locator: item.locator.clone(),
            message: format!("not utf-8: {e}"),
        })?;

        let mut footprints = Vec::new();
        // One Image footprint per unique marker path per session file —
        // the same screenshot referenced by several messages must not
        // produce duplicate specs (the server-side UNIQUE would reject
        // the second anyway; deduping here keeps the run log clean).
        let mut seen_images: HashSet<String> = HashSet::new();
        let mut addresses = RecordAddresses::in_container(&item.locator);
        for (idx, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let obj: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let role = match obj.get("type").and_then(|v| v.as_str()) {
                Some("user") => ChatRole::User,
                Some("assistant") => ChatRole::Assistant,
                _ => continue,
            };

            let body = obj
                .get("message")
                .and_then(|m| m.as_object())
                .and_then(extract_message_text);
            let Some(body) = body else { continue };
            if body.trim().is_empty() {
                continue;
            }

            // This line's own id, or `None` when it stated none — in
            // which case it yields no message, only whatever images it
            // pointed at. See the module rustdoc.
            let uuid = addresses.declared(obj.get("uuid").and_then(|v| v.as_str()));

            let occurred_at = obj
                .get("timestamp")
                .and_then(|v| v.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .or(item.occurred_at)
                .unwrap_or_else(Utc::now);

            let parent_message_id = obj
                .get("parentUuid")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let cwd = obj.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
            let git_branch = obj
                .get("gitBranch")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let is_meta = obj.get("isMeta").and_then(|v| v.as_bool()).unwrap_or(false);

            let extra = json!({
                "cwd": cwd,
                "git_branch": git_branch,
                "is_meta": is_meta,
                "line_index": idx,
                "project_slug": project_slug,
            });

            let mut labels = Vec::new();
            if !project_slug.is_empty() {
                labels.push(project_slug.clone());
            }
            labels.push("cc".into());

            // Images that entered this message (v4 P4): one Image
            // footprint per unique `[Image: source: …]` marker path,
            // bound to the same Session container as the message.
            for image_path in extract_image_markers(&body) {
                if !seen_images.insert(image_path.clone()) {
                    continue;
                }
                let alt = PathBuf::from(&image_path)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string());
                footprints.push(Footprint::Image(Image {
                    source: FootprintSource {
                        kind: item.source_kind.clone(),
                        locator: image_path,
                        platform: Some("Claude Code".to_string()),
                        external_id: None,
                    },
                    occurred_at,
                    external_session_key: Some(external_session_key.clone()),
                    alt,
                    dims: None,
                    file_size_bytes: None,
                    labels: labels.clone(),
                    bundle_id: None,
                    extra: image_extra(uuid, idx, &project_slug),
                    // An image inside a conversation arrived with the
                    // conversation, not from a generator round trip.
                    derived_from: None,
                    album_meta: Default::default(),
                }));
            }

            let Some(uuid) = uuid else { continue };
            footprints.push(Footprint::ChatMessage(ChatMessage {
                source: FootprintSource {
                    kind: item.source_kind.clone(),
                    locator: format!("{}#{uuid}", item.locator),
                    platform: Some("Claude Code".to_string()),
                    external_id: None,
                },
                occurred_at,
                external_session_key: external_session_key.clone(),
                role,
                body,
                thread_position: Some(idx as u64),
                parent_message_id,
                labels,
                extra,
            }));
        }

        addresses.report();
        Ok(footprints)
    }
}

/// `extra` for the footprint of an image marker.
///
/// `message_uuid` appears only when the line stated one. An absent id
/// is left out rather than filled in, so that "this image came from a
/// line with no id" stays distinguishable from "this image came from
/// the line whose id is `L3`" — which is the confusion this whole fix
/// is about.
fn image_extra(message_uuid: Option<&str>, line_index: usize, project_slug: &str) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(uuid) = message_uuid {
        out.insert("message_uuid".into(), Value::String(uuid.to_string()));
    }
    out.insert("line_index".into(), json!(line_index));
    out.insert(
        "project_slug".into(),
        Value::String(project_slug.to_string()),
    );
    Value::Object(out)
}

/// Extracts the paths of `[Image: source: <path>]` markers — the text
/// form Claude Code renders for a pasted or referenced image. The path
/// is everything up to the closing `]` (paths with spaces are common:
/// macOS screenshots); relative or empty candidates are skipped, since
/// the marker contract is an absolute path to the on-disk original.
fn extract_image_markers(body: &str) -> Vec<String> {
    const MARKER: &str = "[Image: source: ";
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find(MARKER) {
        let tail = &rest[start + MARKER.len()..];
        let Some(end) = tail.find(']') else { break };
        let path = tail[..end].trim();
        if path.starts_with('/') {
            out.push(path.to_string());
        }
        rest = &tail[end + 1..];
    }
    out
}

/// Extract user-facing text from a `message` block.
///
/// Handles both the string form (`content: "hello"`) and the block
/// form (`content: [{ type: "text", text: "hello" }, ...]`).
/// Concatenates all text blocks with a space; ignores tool_use /
/// tool_result / thinking / image blocks (which carry no user-visible
/// prose).
fn extract_message_text(msg: &serde_json::Map<String, Value>) -> Option<String> {
    let content = msg.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    if let Some(items) = content.as_array() {
        let mut collected = String::new();
        for c in items {
            if let Some(obj) = c.as_object()
                && obj.get("type").and_then(|v| v.as_str()) == Some("text")
                && let Some(text) = obj.get("text").and_then(|v| v.as_str())
            {
                if !collected.is_empty() {
                    collected.push(' ');
                }
                collected.push_str(text);
            }
        }
        if !collected.is_empty() {
            return Some(collected);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_markers_extract_absolute_paths_with_spaces() {
        let body = "before [Image: source: /Users/x/Desktop/スクリーンショット 2026-07-27 0.14.04.png] \
                    after [Image: source: relative.png] \
                    [Image: source: /a/b.png]";
        assert_eq!(
            extract_image_markers(body),
            vec![
                "/Users/x/Desktop/スクリーンショット 2026-07-27 0.14.04.png".to_string(),
                "/a/b.png".to_string(),
            ],
            "absolute paths (spaces included) extract; relative ones skip"
        );
    }

    /// The v4 P4 contract: an image that entered the conversation
    /// becomes one `Footprint::Image` bound to the **same**
    /// `external_session_key` as the messages, deduped per session
    /// file, with the marker path as its own locator.
    #[test]
    fn conversation_image_joins_the_same_session() {
        let lines = concat!(
            r#"{"type":"user","uuid":"u1","timestamp":"2026-07-27T00:14:05Z","message":{"content":[{"type":"text","text":"look [Image: source: /tmp/shot 1.png]"}]}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-07-27T00:14:06Z","message":{"content":[{"type":"text","text":"seen [Image: source: /tmp/shot 1.png]"}]}}"#,
        );
        let item = RawItem {
            source_kind: "cc".into(),
            locator: "/logs/0198aaaa-session.jsonl".into(),
            payload: lines.as_bytes().to_vec(),
            occurred_at: None,
            extra: serde_json::Value::Null,
        };

        let fps = CcSessionParser.parse(item).unwrap();
        let images: Vec<&Image> = fps
            .iter()
            .filter_map(|f| match f {
                Footprint::Image(i) => Some(i),
                _ => None,
            })
            .collect();
        let messages: Vec<&ChatMessage> = fps
            .iter()
            .filter_map(|f| match f {
                Footprint::ChatMessage(m) => Some(m),
                _ => None,
            })
            .collect();

        assert_eq!(messages.len(), 2);
        assert_eq!(images.len(), 1, "the repeated marker dedupes to one image");
        let image = images[0];
        assert_eq!(image.source.locator, "/tmp/shot 1.png");
        assert_eq!(
            image.external_session_key.as_deref(),
            Some("0198aaaa-session"),
            "the image binds to the same Session container as the messages"
        );
        assert_eq!(
            messages[0].external_session_key, "0198aaaa-session",
            "sanity: message key matches"
        );
        assert_eq!(image.alt.as_deref(), Some("shot 1.png"));
    }

    /// A container is still a container.
    ///
    /// `#` left image locators because a PNG is one record and its text
    /// is that record's metadata. A
    /// `.jsonl` session log is the case the delimiter was always for:
    /// many records in one file, each independently addressable, each
    /// its own row. Without this fixture, "we removed the `#`" and "we
    /// broke record addressing" look the same from the outside.
    #[test]
    fn a_jsonl_container_is_still_one_row_per_record_addressed_by_fragment() {
        let lines = concat!(
            r#"{"type":"user","uuid":"u1","timestamp":"2026-07-27T00:14:05Z","message":{"content":"first"}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-07-27T00:14:06Z","message":{"content":"second"}}"#,
            "\n",
            r#"{"type":"user","uuid":"u2","timestamp":"2026-07-27T00:14:07Z","message":{"content":"third"}}"#,
        );
        let container = "/logs/0198aaaa-session.jsonl";
        let fps = CcSessionParser
            .parse(RawItem {
                source_kind: "cc".into(),
                locator: container.into(),
                payload: lines.as_bytes().to_vec(),
                occurred_at: None,
                extra: serde_json::Value::Null,
            })
            .expect("parse ok");

        // Three records in, three footprints out — the count is the
        // point, so a container collapsed to one row would fail here
        // rather than pass quietly as "one file, one asset".
        assert_eq!(fps.len(), 3, "one row per record, not one per file");
        let locators: Vec<String> = fps
            .into_iter()
            .map(|f| f.into_asset_spec().locator)
            .collect();
        assert_eq!(
            locators,
            vec![
                format!("{container}#u1"),
                format!("{container}#a1"),
                format!("{container}#u2"),
            ],
            "each record keeps its own address inside the container"
        );
    }

    fn address_of(payload: &str, body: &str) -> Option<String> {
        CcSessionParser
            .parse(RawItem {
                source_kind: "cc".into(),
                locator: "/logs/0198aaaa-session.jsonl".into(),
                payload: payload.as_bytes().to_vec(),
                occurred_at: None,
                extra: serde_json::Value::Null,
            })
            .expect("parse ok")
            .iter()
            .find_map(|f| match f {
                Footprint::ChatMessage(m) if m.body == body => Some(m.source.locator.clone()),
                _ => None,
            })
    }

    /// The invariant the fix is for: a record's address does not move
    /// because something was inserted ahead of it in the container.
    ///
    /// The assertions pin the address, not just the equality between
    /// the two scans — with position-derived addressing gone, an
    /// implementation that stopped addressing records at all would
    /// compare `None` to `None` and pass.
    #[test]
    fn record_address_survives_an_insert_ahead_of_it() {
        let before = concat!(
            r#"{"type":"user","uuid":"u1","message":{"content":"first"}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","message":{"content":"second"}}"#,
        );
        let after = concat!(
            r#"{"type":"user","uuid":"u0","message":{"content":"zero"}}"#,
            "\n",
            r#"{"type":"user","uuid":"u1","message":{"content":"first"}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a1","message":{"content":"second"}}"#,
        );

        assert_eq!(
            address_of(before, "first").as_deref(),
            Some("/logs/0198aaaa-session.jsonl#u1"),
            "the address is the id the record itself carries"
        );
        assert_eq!(
            address_of(after, "first"),
            address_of(before, "first"),
            "the same record, addressed twice across an insert ahead of it"
        );
        assert_eq!(
            address_of(after, "second"),
            address_of(before, "second"),
            "the same record, addressed twice across an insert ahead of it"
        );
    }

    /// A line with no `uuid` has no address, so it produces no
    /// message. The image it pointed at is a different record with an
    /// address of its own (the marker path) and survives the drop.
    #[test]
    fn a_line_without_a_uuid_drops_its_message_and_keeps_its_image() {
        let lines = concat!(
            r#"{"type":"user","uuid":"u1","message":{"content":"first"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"anonymous [Image: source: /tmp/shot.png]"}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"u2","message":{"content":"third [Image: source: /tmp/other.png]"}}"#,
        );
        let container = "/logs/0198aaaa-session.jsonl";
        let fps = CcSessionParser
            .parse(RawItem {
                source_kind: "cc".into(),
                locator: container.into(),
                payload: lines.as_bytes().to_vec(),
                occurred_at: None,
                extra: serde_json::Value::Null,
            })
            .expect("parse ok");

        let messages: Vec<&ChatMessage> = fps
            .iter()
            .filter_map(|f| match f {
                Footprint::ChatMessage(m) => Some(m),
                _ => None,
            })
            .collect();
        let images: Vec<&Image> = fps
            .iter()
            .filter_map(|f| match f {
                Footprint::Image(i) => Some(i),
                _ => None,
            })
            .collect();

        assert_eq!(
            messages
                .iter()
                .map(|m| m.source.locator.clone())
                .collect::<Vec<_>>(),
            vec![format!("{container}#u1"), format!("{container}#u2")],
            "the id-less line contributes no message, and does not shift \
             the addresses of the lines around it"
        );
        assert_eq!(
            images
                .iter()
                .map(|i| i.source.locator.clone())
                .collect::<Vec<_>>(),
            vec!["/tmp/shot.png".to_string(), "/tmp/other.png".to_string()],
            "both images survive — a marker path is an address the \
             source really states"
        );
        assert_eq!(
            images[0].extra.get("message_uuid"),
            None,
            "no id to report, so no key — not a fabricated one"
        );
        assert_eq!(
            images[1].extra.get("message_uuid").and_then(|v| v.as_str()),
            Some("u2"),
            "and when the line does state an id, it is carried through"
        );
    }
}
