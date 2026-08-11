//! Character Card V3 parser (see [`crate::catalogue`] section 2).
//!
//! Extends the V2 default composition with three V3-only slot groups:
//!
//! - `creator_notes_multilingual{lang}` → one extra
//!   [`Doc`] (Markdown) per language (appended to the V2
//!   creator_notes doc via
//!   [`super::parser::v2_default::creator_notes`]).
//! - `group_only_greetings[]` → one [`ChatMessage`]
//!   per entry, at high thread positions so they sort after normal
//!   alternates without colliding.
//! - `assets[]` → one [`Image`] per entry, keyed by
//!   `(type, name)` so the grid can group emotions / backgrounds under
//!   the card's shared `session_id`.

use serde_json::json;

use crate::{ChatMessage, ChatRole, Doc, DocFormat, Footprint, Image};

use super::envelope::{CardContext, CardEnvelope};
use super::parser::{CharacterCardParser, v2_default};

/// Thread-position offset used for V3 `group_only_greetings` so they
/// sort strictly after V2 `first_mes` (position 0) and
/// `alternate_greetings[i]` (positions 1..=N). 1000 leaves plenty of
/// headroom while still ordering deterministically.
const GROUP_ONLY_GREETING_OFFSET: u64 = 1000;

/// V3 character-card parser. Inherits V2 slot logic and adds the V3
/// deltas via [`parse_creator_notes`](CharacterCardParser::parse_creator_notes)
/// and [`parse_extras`](CharacterCardParser::parse_extras).
pub struct V3Parser;

impl CharacterCardParser for V3Parser {
    fn spec(&self) -> &'static str {
        "chara_card_v3"
    }

    fn parse_creator_notes(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        // Start from V2 creator_notes (single Markdown doc when present).
        let mut out = v2_default::creator_notes(env, ctx);

        // Append one Doc per language in creator_notes_multilingual.
        let Some(m) = env
            .data
            .get("creator_notes_multilingual")
            .and_then(|v| v.as_object())
        else {
            return out;
        };
        let tags = v2_default::card_tags(env);
        for (lang, text_v) in m {
            let Some(text) = text_v.as_str() else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }
            let mut labels = vec![
                "character-card".into(),
                "creator-notes".into(),
                format!("lang:{lang}"),
                env.spec.clone(),
            ];
            labels.extend(tags.iter().cloned());
            out.push(Footprint::Doc(Doc {
                source: ctx.footprint_source(&format!("field=creator_notes_ml={lang}")),
                occurred_at: ctx.occurred_at,
                title: Some(format!("creator_notes:{lang}")),
                excerpt: text.to_string(),
                format: DocFormat::Markdown,
                bundle_id: Some(ctx.session_id.to_string()),
                file_size_bytes: None,
                word_count: None,
                labels,
                extra: json!({
                    "card_slot": "creator_notes_multilingual",
                    "lang": lang,
                    "card_spec": env.spec,
                    "extensions": v2_default::extensions_bag(env),
                }),
            }));
        }
        out
    }

    fn parse_extras(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        let mut out = Vec::new();
        let bag = v2_default::extensions_bag(env);
        let tags = v2_default::card_tags(env);

        // group_only_greetings[i] → ChatMessage at position 1000+i.
        if let Some(arr) = env
            .data
            .get("group_only_greetings")
            .and_then(|v| v.as_array())
        {
            for (i, text_v) in arr.iter().enumerate() {
                let Some(text) = text_v.as_str() else {
                    continue;
                };
                if text.trim().is_empty() {
                    continue;
                }
                let mut labels = vec![
                    "character-card".into(),
                    "greeting".into(),
                    "group-only".into(),
                    env.spec.clone(),
                ];
                labels.extend(tags.iter().cloned());
                out.push(Footprint::ChatMessage(ChatMessage {
                    source: ctx.footprint_source(&format!("group_only_greeting={i}")),
                    occurred_at: ctx.occurred_at,
                    external_session_key: ctx.session_id.to_string(),
                    role: ChatRole::Assistant,
                    body: text.to_string(),
                    thread_position: Some(GROUP_ONLY_GREETING_OFFSET + i as u64),
                    parent_message_id: None,
                    labels,
                    extra: json!({
                        "greeting_kind": "group_only",
                        "greeting_index": i,
                        "card_spec": env.spec,
                        "extensions": bag,
                    }),
                }));
            }
        }

        // assets[i] → Image (icon / background / emotion / user_icon / x_*).
        if let Some(arr) = env.data.get("assets").and_then(|v| v.as_array()) {
            for (i, asset) in arr.iter().enumerate() {
                let Some(obj) = asset.as_object() else {
                    continue;
                };
                let atype = obj
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("main");
                let uri = obj.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                let ext = obj.get("ext").and_then(|v| v.as_str()).unwrap_or("");

                let mut labels = vec![
                    "character-card".into(),
                    format!("asset:{atype}"),
                    env.spec.clone(),
                ];
                if atype == "emotion" {
                    labels.push(format!("emotion:{name}"));
                }
                labels.extend(tags.iter().cloned());

                out.push(Footprint::Image(Image {
                    source: ctx.footprint_source(&format!("asset={atype}/{name}[{i}]")),
                    occurred_at: ctx.occurred_at,
                    external_session_key: None,
                    alt: Some(format!("{atype}:{name}")),
                    dims: None,
                    file_size_bytes: None,
                    labels,
                    bundle_id: Some(ctx.session_id.to_string()),
                    extra: json!({
                        "asset_type": atype,
                        "asset_name": name,
                        "asset_uri": uri,
                        "asset_ext": ext,
                        "asset_index": i,
                        "card_spec": env.spec,
                        "extensions": bag,
                    }),
                    // A card's inline asset references a URI inside
                    // the card, not a file that made a round trip.
                    derived_from: None,
                    // Nothing in a card mints an identifier for the
                    // inline asset; the card's own `spec` / extension
                    // bag is source data and rides in `extra` above.
                    album_meta: Default::default(),
                }));
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn ctx() -> CardContext<'static> {
        CardContext {
            source_kind: "chara",
            locator: "/tmp/alice.png",
            session_id: "sid-v3",
            occurred_at: Utc::now(),
            platform: Some("SillyTavern"),
        }
    }

    fn env_from(v: serde_json::Value) -> CardEnvelope {
        CardEnvelope::from_json(v).expect("envelope parses")
    }

    #[test]
    fn v3_creator_notes_ml_adds_one_doc_per_language() {
        let env = env_from(json!({
            "spec": "chara_card_v3",
            "spec_version": "3.0",
            "data": {
                "creator_notes": "base note",
                "creator_notes_multilingual": {
                    "en": "hi",
                    "ja": "こんにちは",
                    "de": ""
                }
            }
        }));
        let out = V3Parser.parse_creator_notes(&env, &ctx());
        // 1 (V2 creator_notes) + 2 (en, ja; de empty skipped) = 3.
        assert_eq!(out.len(), 3);
        let langs: Vec<String> = out
            .iter()
            .filter_map(|f| {
                let Footprint::Doc(d) = f else { return None };
                d.extra
                    .get("lang")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();
        assert!(langs.contains(&"en".to_string()));
        assert!(langs.contains(&"ja".to_string()));
    }

    #[test]
    fn v3_group_only_greetings_use_offset_positions() {
        let env = env_from(json!({
            "spec": "chara_card_v3",
            "data": { "group_only_greetings": ["a", "b", "c"] }
        }));
        let out = V3Parser.parse_extras(&env, &ctx());
        let positions: Vec<u64> = out
            .iter()
            .filter_map(|f| {
                let Footprint::ChatMessage(m) = f else {
                    return None;
                };
                m.thread_position
            })
            .collect();
        assert_eq!(
            positions,
            vec![
                GROUP_ONLY_GREETING_OFFSET,
                GROUP_ONLY_GREETING_OFFSET + 1,
                GROUP_ONLY_GREETING_OFFSET + 2
            ]
        );
    }

    #[test]
    fn v3_assets_emit_image_per_entry() {
        let env = env_from(json!({
            "spec": "chara_card_v3",
            "data": {
                "assets": [
                    {"type": "icon",       "name": "main",  "uri": "embeded://icon.png", "ext": "png"},
                    {"type": "emotion",    "name": "happy", "uri": "embeded://happy.png","ext": "png"},
                    {"type": "background", "name": "main",  "uri": "embeded://bg.png",   "ext": "png"}
                ]
            }
        }));
        let out = V3Parser.parse_extras(&env, &ctx());
        let imgs: Vec<&Image> = out
            .iter()
            .filter_map(|f| {
                if let Footprint::Image(i) = f {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(imgs.len(), 3);
        assert!(
            imgs.iter()
                .any(|i| i.labels.contains(&"emotion:happy".into()))
        );
        assert!(
            imgs.iter()
                .any(|i| i.source.locator.ends_with("#asset=icon/main[0]"))
        );
    }

    #[test]
    fn v3_parse_full_composes_v2_defaults_and_v3_extras() {
        let env = env_from(json!({
            "spec": "chara_card_v3",
            "spec_version": "3.0",
            "data": {
                "name": "Alice",
                "first_mes": "hi",
                "group_only_greetings": ["party start"],
                "assets": [{"type":"icon","name":"main","uri":"e://x","ext":"png"}]
            }
        }));
        let out = V3Parser.parse(&env, &ctx());
        // 1 name Note + 1 first_mes ChatMessage + 1 group_only ChatMessage +
        // 1 icon Image = 4.
        assert_eq!(out.len(), 4);
    }
}
