//! [`CharacterCardParser`] trait + canonical V2 slot logic.
//!
//! The trait has six per-slot hooks whose defaults cover the V2 spec.
//! Derivatives override individual hooks — see [`crate::card::v3`] for
//! the reference example.
//!
//! The V2 slot logic is also exposed as free functions in
//! [`v2_default`] so derivatives can chain (e.g. V3
//! [`crate::card::v3::V3Parser`] calls [`v2_default::creator_notes`]
//! and appends multilingual variants).

use crate::Footprint;

use super::envelope::{CardContext, CardEnvelope};

/// Extension trait for character-card decomposition.
///
/// Impl this for each spec / vendor variant. The default `parse`
/// implementation runs the six slot hooks in a fixed order; override
/// individual hooks to tweak per-slot behaviour, or override `parse`
/// itself for a radically different layout (rare — the V2 six-slot
/// shape covers V3 and every third-party derivative surveyed in
/// [`crate::catalogue`]).
///
/// The registered [`crate::card::CardParserRegistry`] pre-loads
/// [`crate::card::V2Parser`] and [`crate::card::V3Parser`]. Third-party
/// crates add their own parsers via [`crate::card::CardParserRegistry::register`].
pub trait CharacterCardParser: Send + Sync {
    /// The `spec` string this parser handles
    /// (e.g. `"chara_card_v2"`, `"chara_card_v3"`, or a vendor slug).
    /// [`crate::card::CardParserRegistry`] dispatches by exact match.
    fn spec(&self) -> &'static str;

    /// Decompose an envelope into per-slot footprints.
    ///
    /// The default composition runs
    /// [`parse_text_slots`](Self::parse_text_slots),
    /// [`parse_greetings`](Self::parse_greetings),
    /// [`parse_mes_example`](Self::parse_mes_example),
    /// [`parse_creator_notes`](Self::parse_creator_notes),
    /// [`parse_book`](Self::parse_book),
    /// [`parse_extras`](Self::parse_extras) in that order and
    /// concatenates the results.
    fn parse(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        let mut out = Vec::new();
        out.extend(self.parse_text_slots(env, ctx));
        out.extend(self.parse_greetings(env, ctx));
        out.extend(self.parse_mes_example(env, ctx));
        out.extend(self.parse_creator_notes(env, ctx));
        out.extend(self.parse_book(env, ctx));
        out.extend(self.parse_extras(env, ctx));
        out
    }

    /// Emit one `Note` per non-empty text slot
    /// (name / description / personality / scenario / system_prompt /
    /// post_history_instructions). Default = [`v2_default::text_slots`].
    fn parse_text_slots(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        v2_default::text_slots(env, ctx)
    }

    /// Emit one `ChatMessage` for `first_mes` plus one per
    /// `alternate_greetings[i]`. Default = [`v2_default::greetings`].
    fn parse_greetings(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        v2_default::greetings(env, ctx)
    }

    /// Emit one `Doc(Plain)` for `mes_example` when present. Default =
    /// [`v2_default::mes_example`]. A derivative that recognises the
    /// `<START>` delimiter can override this to fan out into many
    /// [`crate::ChatMessage`]s.
    fn parse_mes_example(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        v2_default::mes_example(env, ctx)
    }

    /// Emit one `Doc(Markdown)` for `creator_notes` when present.
    /// Default = [`v2_default::creator_notes`]. V3 overrides this to
    /// also fan out `creator_notes_multilingual{lang}`.
    fn parse_creator_notes(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        v2_default::creator_notes(env, ctx)
    }

    /// Emit one `Note` per `character_book.entries[]` element. Handles
    /// both the V2 embedded array shape and the standalone
    /// dict-of-uid shape. Default = [`v2_default::book`].
    fn parse_book(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        v2_default::book(env, ctx)
    }

    /// Vendor-specific extension hook.
    ///
    /// Default = no-op. V3 overrides this to emit `assets[]` as
    /// `Image` × N and `group_only_greetings[]` as `ChatMessage` × N;
    /// a chub-specific parser could override it to promote
    /// `data.extensions.chub.expressions[]` similarly.
    fn parse_extras(&self, _env: &CardEnvelope, _ctx: &CardContext<'_>) -> Vec<Footprint> {
        Vec::new()
    }
}

/// Canonical V2 slot logic, exposed as free functions so derivatives
/// can chain (call the V2 version, then append their own outputs)
/// rather than re-implement.
///
/// Every function is deterministic given the same envelope + context
/// and produces zero footprints when the corresponding slot is
/// absent, empty, or the wrong shape (so importers can call them
/// unconditionally without pre-checking).
pub mod v2_default {
    use serde_json::{Value, json};

    use crate::{ChatMessage, ChatRole, Doc, DocFormat, Footprint, Note};

    use super::{CardContext, CardEnvelope};

    /// The six V2 text-slot names, in the order [`text_slots`] emits
    /// them. Exposed for derivatives that want to enumerate the same
    /// slots without depending on the exact list.
    pub const TEXT_SLOT_NAMES: &[&str] = &[
        "name",
        "description",
        "personality",
        "scenario",
        "system_prompt",
        "post_history_instructions",
    ];

    /// Extract the extensions bag (`data.extensions`) as a JSON value
    /// so every emitted footprint's `extra` can carry the same
    /// verbatim copy — the canonical pass-through convention documented
    /// in [`crate::catalogue`].
    pub fn extensions_bag(env: &CardEnvelope) -> Value {
        env.data.get("extensions").cloned().unwrap_or(Value::Null)
    }

    /// Card-level tag labels (`data.tags[]`) as owned strings. Empty
    /// when the slot is absent or not an array.
    pub fn card_tags(env: &CardEnvelope) -> Vec<String> {
        env.data
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// V2 default for [`super::CharacterCardParser::parse_text_slots`].
    ///
    /// Emits one [`Note`] per non-empty text slot, with
    /// labels `["character-card", <slot>, <spec>, …tags]` and the
    /// vendor extensions bag mirrored into `extra.extensions`.
    pub fn text_slots(env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        let mut out = Vec::new();
        let bag = extensions_bag(env);
        let tags = card_tags(env);
        for slot in TEXT_SLOT_NAMES {
            let Some(text) = env.data.get(*slot).and_then(|v| v.as_str()) else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }
            let source = ctx.footprint_source(&format!("field={slot}"));
            let extra = json!({
                "card_slot": slot,
                "card_spec": env.spec,
                "card_spec_version": env.spec_version,
                "extensions": bag,
            });
            let mut labels = vec![
                "character-card".into(),
                (*slot).to_string(),
                env.spec.clone(),
            ];
            labels.extend(tags.iter().cloned());
            out.push(Footprint::Note(Note {
                source,
                occurred_at: ctx.occurred_at,
                body: text.to_string(),
                source_app: None,
                labels,
                bundle_id: Some(ctx.session_id.to_string()),
                extra,
            }));
        }
        out
    }

    /// V2 default for [`super::CharacterCardParser::parse_greetings`].
    ///
    /// Emits `first_mes` at `thread_position=0` and each
    /// `alternate_greetings[i]` at `thread_position=i+1`, all with
    /// `role = Assistant`.
    pub fn greetings(env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        let mut out = Vec::new();
        let bag = extensions_bag(env);
        let tags = card_tags(env);

        if let Some(text) = env.data.get("first_mes").and_then(|v| v.as_str())
            && !text.trim().is_empty()
        {
            let source = ctx.footprint_source("greeting=0");
            let mut labels = vec!["character-card".into(), "greeting".into(), env.spec.clone()];
            labels.extend(tags.iter().cloned());
            out.push(Footprint::ChatMessage(ChatMessage {
                source,
                occurred_at: ctx.occurred_at,
                external_session_key: ctx.session_id.to_string(),
                role: ChatRole::Assistant,
                body: text.to_string(),
                thread_position: Some(0),
                parent_message_id: None,
                labels,
                extra: json!({
                    "greeting_kind": "first",
                    "card_spec": env.spec,
                    "extensions": bag,
                }),
            }));
        }

        if let Some(arr) = env
            .data
            .get("alternate_greetings")
            .and_then(|v| v.as_array())
        {
            for (i, text_v) in arr.iter().enumerate() {
                let Some(text) = text_v.as_str() else {
                    continue;
                };
                if text.trim().is_empty() {
                    continue;
                }
                let pos = (i as u64) + 1;
                let source = ctx.footprint_source(&format!("greeting={pos}"));
                let mut labels = vec![
                    "character-card".into(),
                    "greeting-alt".into(),
                    env.spec.clone(),
                ];
                labels.extend(tags.iter().cloned());
                out.push(Footprint::ChatMessage(ChatMessage {
                    source,
                    occurred_at: ctx.occurred_at,
                    external_session_key: ctx.session_id.to_string(),
                    role: ChatRole::Assistant,
                    body: text.to_string(),
                    thread_position: Some(pos),
                    parent_message_id: None,
                    labels,
                    extra: json!({
                        "greeting_kind": "alternate",
                        "greeting_index": i,
                        "card_spec": env.spec,
                        "extensions": bag,
                    }),
                }));
            }
        }
        out
    }

    /// V2 default for
    /// [`super::CharacterCardParser::parse_mes_example`]. Emits one
    /// [`Doc`] with `format = Plain` when the slot is
    /// present.
    pub fn mes_example(env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        let Some(text) = env.data.get("mes_example").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        if text.trim().is_empty() {
            return Vec::new();
        }
        let tags = card_tags(env);
        let mut labels = vec![
            "character-card".into(),
            "mes-example".into(),
            env.spec.clone(),
        ];
        labels.extend(tags);
        vec![Footprint::Doc(Doc {
            source: ctx.footprint_source("field=mes_example"),
            occurred_at: ctx.occurred_at,
            title: Some("mes_example".into()),
            excerpt: text.to_string(),
            format: DocFormat::Plain,
            bundle_id: Some(ctx.session_id.to_string()),
            file_size_bytes: None,
            word_count: None,
            labels,
            extra: json!({
                "card_slot": "mes_example",
                "card_spec": env.spec,
                "extensions": extensions_bag(env),
            }),
        })]
    }

    /// V2 default for
    /// [`super::CharacterCardParser::parse_creator_notes`]. Emits one
    /// [`Doc`] with `format = Markdown` when the slot is
    /// present.
    pub fn creator_notes(env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        let Some(text) = env.data.get("creator_notes").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        if text.trim().is_empty() {
            return Vec::new();
        }
        let tags = card_tags(env);
        let mut labels = vec![
            "character-card".into(),
            "creator-notes".into(),
            env.spec.clone(),
        ];
        labels.extend(tags);
        vec![Footprint::Doc(Doc {
            source: ctx.footprint_source("field=creator_notes"),
            occurred_at: ctx.occurred_at,
            title: Some("creator_notes".into()),
            excerpt: text.to_string(),
            format: DocFormat::Markdown,
            bundle_id: Some(ctx.session_id.to_string()),
            file_size_bytes: None,
            word_count: None,
            labels,
            extra: json!({
                "card_slot": "creator_notes",
                "card_spec": env.spec,
                "extensions": extensions_bag(env),
            }),
        })]
    }

    /// V2 default for [`super::CharacterCardParser::parse_book`].
    ///
    /// Handles both the V2 embedded array shape
    /// (`character_book.entries: [{…}]`) and the standalone
    /// dict-of-uid shape (`character_book.entries: {"<uid>": {…}}`).
    /// Each entry becomes a [`Note`] whose body is the
    /// entry `content`, with `key[]` (or `keys[]`) merged into labels
    /// and the raw entry preserved in `extra.raw` so downstream can
    /// still reach the position / order / decorator fields verbatim.
    pub fn book(env: &CardEnvelope, ctx: &CardContext<'_>) -> Vec<Footprint> {
        let Some(book) = env.data.get("character_book").and_then(|v| v.as_object()) else {
            return Vec::new();
        };
        let Some(entries) = book.get("entries") else {
            return Vec::new();
        };

        let iter: Vec<(String, Value)> = if let Some(arr) = entries.as_array() {
            arr.iter()
                .enumerate()
                .map(|(i, v)| (i.to_string(), v.clone()))
                .collect()
        } else if let Some(obj) = entries.as_object() {
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for (fallback_id, entry_v) in iter {
            let Some(entry) = entry_v.as_object() else {
                continue;
            };
            let Some(content) = entry.get("content").and_then(|v| v.as_str()) else {
                continue;
            };
            if content.trim().is_empty() {
                continue;
            }
            // Prefer explicit uid, then id, then the enumeration key /
            // array index. The suffix stays stable across re-imports
            // as long as the source keeps the same field.
            let uid = entry
                .get("uid")
                .and_then(|v| v.as_str().map(String::from))
                .or_else(|| entry.get("id").and_then(|v| v.as_str().map(String::from)))
                .unwrap_or(fallback_id);
            let keys: Vec<String> = entry
                .get("keys")
                .or_else(|| entry.get("key"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let mut labels = vec!["character-card".into(), "lorebook".into(), env.spec.clone()];
            labels.extend(keys);

            out.push(Footprint::Note(Note {
                source: ctx.footprint_source(&format!("book_entry={uid}")),
                occurred_at: ctx.occurred_at,
                body: content.to_string(),
                source_app: None,
                labels,
                bundle_id: Some(ctx.session_id.to_string()),
                extra: json!({
                    "card_slot": "character_book",
                    "entry_uid": uid,
                    "card_spec": env.spec,
                    "raw": entry_v,
                }),
            }));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatRole, DocFormat, Footprint};
    use chrono::Utc;
    use serde_json::json;

    fn ctx() -> CardContext<'static> {
        CardContext {
            source_kind: "chara",
            locator: "/tmp/alice.png",
            session_id: "sid-42",
            occurred_at: Utc::now(),
            platform: Some("SillyTavern"),
        }
    }

    fn env_from(v: serde_json::Value) -> CardEnvelope {
        CardEnvelope::from_json(v).expect("envelope parses")
    }

    #[test]
    fn text_slots_emit_one_note_per_present_slot() {
        let env = env_from(json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Alice",
                "description": "an example",
                "personality": "curious",
                "scenario": "",         // empty → skipped
                "tags": ["fantasy", "solo"]
            }
        }));
        let out = v2_default::text_slots(&env, &ctx());
        assert_eq!(out.len(), 3, "3 non-empty slots yield 3 Notes");
        for f in &out {
            let Footprint::Note(n) = f else {
                panic!("not a Note")
            };
            assert!(n.labels.contains(&"character-card".into()));
            assert!(n.labels.contains(&"fantasy".into()));
            assert_eq!(n.bundle_id.as_deref(), Some("sid-42"));
        }
    }

    #[test]
    fn greetings_emit_first_and_alternates_with_incrementing_positions() {
        let env = env_from(json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "first_mes": "hello",
                "alternate_greetings": ["hi", "yo", ""],
            }
        }));
        let out = v2_default::greetings(&env, &ctx());
        assert_eq!(out.len(), 3, "1 first + 2 non-empty alts");
        let positions: Vec<u64> = out
            .iter()
            .map(|f| {
                let Footprint::ChatMessage(m) = f else {
                    panic!()
                };
                assert_eq!(m.role, ChatRole::Assistant);
                m.thread_position.unwrap()
            })
            .collect();
        assert_eq!(positions, vec![0, 1, 2]);
    }

    #[test]
    fn mes_example_emits_doc_plain_when_present() {
        let env = env_from(json!({
            "spec": "chara_card_v2",
            "data": { "mes_example": "user: hi\nchar: hi" }
        }));
        let out = v2_default::mes_example(&env, &ctx());
        assert_eq!(out.len(), 1);
        let Footprint::Doc(d) = &out[0] else {
            panic!("not a Doc")
        };
        assert_eq!(d.format, DocFormat::Plain);
    }

    #[test]
    fn creator_notes_emits_doc_markdown_when_present() {
        let env = env_from(json!({
            "spec": "chara_card_v2",
            "data": { "creator_notes": "## About\nSome markdown" }
        }));
        let out = v2_default::creator_notes(&env, &ctx());
        assert_eq!(out.len(), 1);
        let Footprint::Doc(d) = &out[0] else {
            panic!("not a Doc")
        };
        assert_eq!(d.format, DocFormat::Markdown);
    }

    #[test]
    fn book_handles_v2_array_shape() {
        let env = env_from(json!({
            "spec": "chara_card_v2",
            "data": {
                "character_book": {
                    "entries": [
                        {"keys": ["fire","flame"], "content": "burns hot", "id": "e1"},
                        {"keys": ["water"], "content": "wet", "uid": "e2"},
                        {"keys": [], "content": ""}
                    ]
                }
            }
        }));
        let out = v2_default::book(&env, &ctx());
        assert_eq!(out.len(), 2, "empty content skipped");
        let Footprint::Note(n0) = &out[0] else {
            panic!()
        };
        assert!(n0.labels.iter().any(|l| l == "fire"));
        assert!(n0.source.locator.ends_with("#book_entry=e1"));
        let Footprint::Note(n1) = &out[1] else {
            panic!()
        };
        assert!(n1.source.locator.ends_with("#book_entry=e2"));
    }

    #[test]
    fn book_handles_standalone_dict_shape() {
        let env = env_from(json!({
            "spec": "chara_card_v2",
            "data": {
                "character_book": {
                    "entries": {
                        "u-1": {"content": "foo", "keys": ["k"]},
                        "u-2": {"content": "bar"}
                    }
                }
            }
        }));
        let out = v2_default::book(&env, &ctx());
        assert_eq!(out.len(), 2);
        for f in &out {
            let Footprint::Note(n) = f else { panic!() };
            assert!(
                n.source.locator.ends_with("#book_entry=u-1")
                    || n.source.locator.ends_with("#book_entry=u-2")
            );
        }
    }

    #[test]
    fn extensions_bag_passthrough_appears_on_notes() {
        let env = env_from(json!({
            "spec": "chara_card_v2",
            "data": {
                "name": "Alice",
                "extensions": { "chub": { "alt_expressions": ["happy","sad"] } }
            }
        }));
        let out = v2_default::text_slots(&env, &ctx());
        let Footprint::Note(n) = &out[0] else {
            panic!()
        };
        let ext = n
            .extra
            .get("extensions")
            .and_then(|v| v.as_object())
            .unwrap();
        assert!(ext.contains_key("chub"));
    }
}
