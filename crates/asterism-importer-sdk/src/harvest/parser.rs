//! [`SourceParser`] impl that decodes `asterism_agent_harvest` JSON
//! dumps and emits [`Footprint::ChatMessage`] × N.
//!
//! A message is addressed `<dump>#conversation=<id>/message=<id>`,
//! both ids as the dump states them. A message with no `id` of its
//! own is dropped, not numbered by its place in the array — see
//! [`crate::parser`] for the rule and
//! [`crate::parser::RecordAddresses`] for the count that goes with it.

use chrono::Utc;
use serde_json::{Value, json};

use crate::bundle::session_id_for;
use crate::parser::{ParseError, RecordAddresses, SourceParser};
use crate::scanner::RawItem;
use crate::{ChatMessage, ChatRole, Footprint, FootprintSource};

use super::envelope::{HARVEST_SPEC, HarvestEnvelope, HarvestMessage};

/// SourceParser for the `asterism_agent_harvest` canonical schema.
///
/// One dump file yields one `ChatMessage` per message across every
/// conversation in the envelope. Character metadata (`characters[]`)
/// is preserved in `extra.character` on the corresponding messages
/// via the character_id ↔ id join, but no dedicated character
/// footprints are emitted in v1 (see the module rustdoc).
pub struct HarvestSourceParser {
    /// Optional platform override for `FootprintSource.platform`.
    /// When `None`, the parser uses the envelope's `service` slug so
    /// the label always reflects the origin.
    platform: Option<String>,
}

impl HarvestSourceParser {
    /// Build a parser with defaults (platform = `envelope.service`).
    pub fn new() -> Self {
        Self { platform: None }
    }

    /// Override the platform string with a caller-supplied literal.
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }
}

impl Default for HarvestSourceParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceParser for HarvestSourceParser {
    fn parse(&self, item: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let text = std::str::from_utf8(&item.payload).map_err(|e| ParseError::Malformed {
            locator: item.locator.clone(),
            message: format!("not utf-8: {e}"),
        })?;
        let env: HarvestEnvelope =
            serde_json::from_str(text).map_err(|e| ParseError::Malformed {
                locator: item.locator.clone(),
                message: format!("agent-harvest deserialise: {e}"),
            })?;
        if env.spec != HARVEST_SPEC {
            return Err(ParseError::Malformed {
                locator: item.locator,
                message: format!("wrong spec: expected {HARVEST_SPEC}, got {}", env.spec),
            });
        }

        // Index characters by id so we can enrich each message with
        // the resolved character name / description in `extra`.
        let char_index: std::collections::HashMap<&str, &super::envelope::HarvestCharacter> =
            env.characters.iter().map(|c| (c.id.as_str(), c)).collect();

        let platform = self.platform.clone().unwrap_or_else(|| env.service.clone());
        let raw_occurred_at = item.occurred_at;

        let mut out = Vec::new();
        let mut addresses = RecordAddresses::in_container(&item.locator);
        for conv in &env.conversations {
            // Derived external key — the server resolves it to a
            // `Session.id` via find-or-create so a re-import of the
            // same harvest dump lands on the same Session row.
            let external_session_key = session_id_for(&format!("{}#{}", item.locator, conv.id));
            let character = conv
                .character_id
                .as_deref()
                .and_then(|id| char_index.get(id).copied());
            for (idx, msg) in conv.messages.iter().enumerate() {
                // No id, no address, no asset — the message's position
                // in the array is the array's property, not the
                // message's. See the module rustdoc.
                let Some(msg_id) = addresses.declared(msg.id.as_deref()) else {
                    continue;
                };
                let occurred_at = msg
                    .timestamp
                    .or(conv.started_at)
                    .or(env.harvested_at)
                    .or(raw_occurred_at)
                    .unwrap_or_else(Utc::now);
                let locator = format!(
                    "{}#conversation={}/message={}",
                    item.locator, conv.id, msg_id
                );
                let source = FootprintSource {
                    kind: item.source_kind.clone(),
                    locator,
                    platform: Some(platform.clone()),
                    // The harvested message's own id is already inside
                    // the locator above, which is where the pipeline
                    // reads it. Restating it as an external key would
                    // be the same string in two roles.
                    external_id: None,
                };
                let role = parse_role(&msg.role);
                let mut labels = vec![
                    "agent-harvest".into(),
                    env.service.clone(),
                    role.as_slug().to_string(),
                ];
                if let Some(t) = &conv.title {
                    labels.push(format!("chat:{}", trunc_label(t)));
                }
                let extra = build_extra(&env, conv, msg, character);
                out.push(Footprint::ChatMessage(ChatMessage {
                    source,
                    occurred_at,
                    external_session_key: external_session_key.clone(),
                    role,
                    body: msg.body.clone(),
                    thread_position: Some(idx as u64),
                    parent_message_id: msg.parent_id.clone(),
                    labels,
                    extra,
                }));
            }
        }
        addresses.report();
        Ok(out)
    }
}

fn parse_role(slug: &str) -> ChatRole {
    match slug {
        "user" => ChatRole::User,
        "assistant" => ChatRole::Assistant,
        "system" => ChatRole::System,
        "tool" => ChatRole::Tool,
        other => ChatRole::Other(other.to_string()),
    }
}

fn trunc_label(s: &str) -> String {
    let one = s.replace('\n', " ");
    if one.chars().count() > 40 {
        one.chars().take(40).collect()
    } else {
        one
    }
}

fn build_extra(
    env: &HarvestEnvelope,
    conv: &super::envelope::HarvestConversation,
    msg: &HarvestMessage,
    character: Option<&super::envelope::HarvestCharacter>,
) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("service".into(), Value::String(env.service.clone()));
    out.insert(
        "spec_version".into(),
        Value::String(env.spec_version.clone()),
    );
    if let Some(uid) = &env.service_user_id {
        out.insert("service_user_id".into(), Value::String(uid.clone()));
    }
    if !env.extra.is_null() {
        out.insert("envelope_extra".into(), env.extra.clone());
    }
    if !conv.extra.is_null() {
        out.insert("conversation_extra".into(), conv.extra.clone());
    }
    if let Some(title) = &conv.title {
        out.insert("conversation_title".into(), Value::String(title.clone()));
    }
    if !msg.extra.is_null() {
        out.insert("message_extra".into(), msg.extra.clone());
    }
    if let Some(c) = character {
        out.insert(
            "character".into(),
            json!({
                "id": c.id,
                "name": c.name,
                "description": c.description,
            }),
        );
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::super::envelope::HARVEST_SPEC_VERSION;
    use super::*;
    use serde_json::json;

    fn item_from(body: serde_json::Value, locator: &str) -> RawItem {
        RawItem {
            source_kind: "agent-harvest".into(),
            locator: locator.into(),
            payload: serde_json::to_vec(&body).unwrap(),
            occurred_at: None,
            extra: json!({}),
        }
    }

    #[test]
    fn parses_example_fixture_into_chat_messages() {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../../schema/agent_harvest.example.json")).unwrap();
        let item = item_from(raw, "/tmp/dump.json");
        let out = HarvestSourceParser::new().parse(item).unwrap();
        assert_eq!(out.len(), 2, "example has 2 messages");
        let Footprint::ChatMessage(m0) = &out[0] else {
            panic!()
        };
        assert_eq!(m0.role, ChatRole::User);
        assert_eq!(m0.body, "hello");
        assert_eq!(m0.thread_position, Some(0));
        let Footprint::ChatMessage(m1) = &out[1] else {
            panic!()
        };
        assert_eq!(m1.role, ChatRole::Assistant);
        assert_eq!(m1.parent_message_id.as_deref(), Some("m1"));
    }

    #[test]
    fn wrong_spec_returns_error() {
        let raw = json!({
            "spec": "not_agent_harvest",
            "spec_version": "1.0",
            "service": "x",
            "conversations": []
        });
        let item = item_from(raw, "/tmp/wrong.json");
        let err = HarvestSourceParser::new().parse(item).unwrap_err();
        assert!(format!("{err}").contains("wrong spec"));
    }

    /// This test used to assert the opposite: that a message with no
    /// `id` was addressed `message=m0` by its index. That address
    /// belonged to whichever message happened to be first at scan
    /// time, so it moved whenever the dump gained a message ahead of
    /// it. A message the dump does not identify is now dropped.
    #[test]
    fn a_message_with_no_id_is_dropped_not_numbered() {
        let raw = json!({
            "spec": HARVEST_SPEC,
            "spec_version": HARVEST_SPEC_VERSION,
            "service": "character.ai",
            "conversations": [{
                "id": "c1",
                "messages": [
                    {"role": "user", "body": "a"},
                    {"role": "assistant", "id": "x1", "body": "b"}
                ]
            }]
        });
        let item = item_from(raw, "/tmp/anon.json");
        let out = HarvestSourceParser::new().parse(item).unwrap();

        assert_eq!(out.len(), 1, "only the identified message becomes an asset");
        let Footprint::ChatMessage(m) = &out[0] else {
            panic!()
        };
        assert_eq!(
            m.source.locator, "/tmp/anon.json#conversation=c1/message=x1",
            "and it is addressed by the id the dump gave it"
        );
    }

    fn address_of(messages: serde_json::Value, body: &str) -> Option<String> {
        let raw = json!({
            "spec": HARVEST_SPEC,
            "spec_version": HARVEST_SPEC_VERSION,
            "service": "character.ai",
            "conversations": [{ "id": "c1", "messages": messages }]
        });
        HarvestSourceParser::new()
            .parse(item_from(raw, "/tmp/anon.json"))
            .expect("parse ok")
            .iter()
            .find_map(|f| match f {
                Footprint::ChatMessage(m) if m.body == body => Some(m.source.locator.clone()),
                _ => None,
            })
    }

    /// The invariant the fix is for: a message's address does not move
    /// because the dump gained a message ahead of it.
    ///
    /// The assertions pin the address, not just the equality between
    /// the two scans — an implementation that stopped addressing
    /// messages at all would compare `None` to `None` and pass.
    #[test]
    fn record_address_survives_an_insert_ahead_of_it() {
        let before = json!([
            {"role": "user", "id": "x1", "body": "first"},
            {"role": "assistant", "id": "x2", "body": "second"}
        ]);
        let after = json!([
            {"role": "user", "id": "x0", "body": "zero"},
            {"role": "user", "id": "x1", "body": "first"},
            {"role": "assistant", "id": "x2", "body": "second"}
        ]);

        assert_eq!(
            address_of(before.clone(), "first").as_deref(),
            Some("/tmp/anon.json#conversation=c1/message=x1"),
            "the address is the id the message itself carries"
        );
        assert_eq!(
            address_of(after.clone(), "first"),
            address_of(before.clone(), "first"),
            "the same record, addressed twice across an insert ahead of it"
        );
        assert_eq!(
            address_of(after, "second"),
            address_of(before, "second"),
            "the same record, addressed twice across an insert ahead of it"
        );
    }

    #[test]
    fn external_session_key_shared_within_conversation_split_between_conversations() {
        let raw = json!({
            "spec": HARVEST_SPEC,
            "spec_version": HARVEST_SPEC_VERSION,
            "service": "kindroid",
            "conversations": [
                {"id": "c1", "messages": [{"role": "user", "id": "x1", "body": "a"}, {"role": "assistant", "id": "x2", "body": "b"}]},
                {"id": "c2", "messages": [{"role": "user", "id": "x3", "body": "c"}]}
            ]
        });
        let item = item_from(raw, "/tmp/multi.json");
        let out = HarvestSourceParser::new().parse(item).unwrap();
        let keys: Vec<String> = out
            .iter()
            .filter_map(|f| {
                if let Footprint::ChatMessage(m) = f {
                    Some(m.external_session_key.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            keys[0], keys[1],
            "same conversation → same external_session_key"
        );
        assert_ne!(
            keys[0], keys[2],
            "different conversation → different external_session_key"
        );
    }

    #[test]
    fn role_slug_falls_through_to_other() {
        let raw = json!({
            "spec": HARVEST_SPEC,
            "spec_version": HARVEST_SPEC_VERSION,
            "service": "grok",
            "conversations": [{
                "id": "c1",
                "messages": [{"role": "custom_bot", "id": "x1", "body": "beep"}]
            }]
        });
        let item = item_from(raw, "/tmp/roles.json");
        let out = HarvestSourceParser::new().parse(item).unwrap();
        let Footprint::ChatMessage(m) = &out[0] else {
            panic!()
        };
        assert_eq!(m.role, ChatRole::Other("custom_bot".into()));
    }

    #[test]
    fn character_metadata_enriches_message_extra() {
        let raw = json!({
            "spec": HARVEST_SPEC,
            "spec_version": HARVEST_SPEC_VERSION,
            "service": "character.ai",
            "characters": [
                {"id": "seraphina", "name": "Seraphina", "description": "gentle"}
            ],
            "conversations": [{
                "id": "c1",
                "character_id": "seraphina",
                "messages": [{"role": "assistant", "id": "x1", "body": "welcome"}]
            }]
        });
        let item = item_from(raw, "/tmp/char.json");
        let out = HarvestSourceParser::new().parse(item).unwrap();
        let Footprint::ChatMessage(m) = &out[0] else {
            panic!()
        };
        let ch = m
            .extra
            .get("character")
            .and_then(|v| v.as_object())
            .unwrap();
        assert_eq!(ch.get("name").and_then(|v| v.as_str()), Some("Seraphina"));
    }
}
