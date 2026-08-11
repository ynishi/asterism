//! Persona Tape parser — one terminal transcript file becomes one document.

use std::path::Path;

use asterism_importer_sdk::{Footprint, FootprintSource, ParseError, RawItem, SourceParser, Tape};
use chrono::Utc;

pub struct TapeParser {
    platform: Option<String>,
}

impl TapeParser {
    pub fn new(platform: Option<String>) -> Self {
        Self { platform }
    }
}

impl SourceParser for TapeParser {
    fn parse(&self, item: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let text = std::str::from_utf8(&item.payload).map_err(|err| ParseError::Malformed {
            locator: item.locator.clone(),
            message: format!("not utf-8: {err}"),
        })?;
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let path = Path::new(&item.locator);
        let stem = path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| item.locator.clone());
        let title = text
            .lines()
            .map(str::trim)
            .find_map(|line| line.strip_prefix("# ").map(str::to_owned))
            .unwrap_or_else(|| stem.clone());
        let excerpt = text
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with('❯'))
            .or_else(|| text.lines().map(str::trim).find(|line| !line.is_empty()))
            .unwrap_or(&title)
            .to_owned();
        let file_size_bytes = item
            .extra
            .get("file_size_bytes")
            .and_then(serde_json::Value::as_u64)
            .or(Some(item.payload.len() as u64));

        Ok(vec![Footprint::Tape(Tape {
            source: FootprintSource {
                kind: item.source_kind,
                locator: item.locator,
                platform: self.platform.clone(),
                external_id: None,
            },
            occurred_at: item.occurred_at.unwrap_or_else(Utc::now),
            title: Some(title),
            excerpt,
            // Tape is a non-Dialog modality — the file stem drives
            // constellation-edge grouping through `bundle_id`, not
            // through `Session`.
            bundle_id: Some(stem.clone()),
            file_size_bytes,
            labels: vec!["tape".into()],
            extra: serde_json::json!({ "tape_id": stem }),
        })])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_importer_sdk::RawItem;

    #[test]
    fn maps_one_tape_to_one_searchable_terminal_document() {
        let raw = RawItem {
            source_kind: "persona-tape".into(),
            locator: "/tmp/rin-202607-00080.txt".into(),
            payload: "Claude Code\n\n❯ hello\n\n⏺ hi".as_bytes().to_vec(),
            occurred_at: None,
            extra: serde_json::json!({ "file_size_bytes": 31 }),
        };

        let output = TapeParser::new(Some("Claude Code Tape".into()))
            .parse(raw)
            .unwrap();
        assert_eq!(output.len(), 1);
        let spec = output.into_iter().next().unwrap().into_asset_spec();
        assert_eq!(spec.source_kind, "persona-tape");
        assert_eq!(spec.modality.as_deref(), Some("tape"));
        // P3 (session-model): the file stem drives constellation-edge
        // grouping through `bundle_id` — Session is reserved for
        // dialogue-modality assets and `session_id` stays `None`.
        assert_eq!(spec.bundle_id.as_deref(), Some("rin-202607-00080"));
        assert_eq!(spec.session_id, None);
        assert_eq!(spec.external_session_key, None);
        assert_eq!(spec.register_note.as_deref(), Some("rin-202607-00080"));
        assert_eq!(spec.cover_hint.as_deref(), Some("❯ hello"));
        assert!(spec.labels.contains(&"tape".to_string()));
    }
}
