//! Typed schema for `asterism_agent_harvest` v1 JSON dumps.
//!
//! Every field is either strictly required or has a serde default so a
//! minimal well-formed dump only needs `spec`, `spec_version`,
//! `service`, and at least one `conversations[].messages[]` entry.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Canonical `spec` string every dump must carry. The parser rejects
/// payloads whose `spec` differs, so future revisions (v2, v3) can
/// coexist without silent mis-interpretation.
pub const HARVEST_SPEC: &str = "asterism_agent_harvest";

/// Current `spec_version`. Bumped on breaking schema changes.
pub const HARVEST_SPEC_VERSION: &str = "1.0";

/// Top-level envelope for one harvested batch (one file = one dump).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestEnvelope {
    /// Must equal [`HARVEST_SPEC`]; the parser gates on this.
    pub spec: String,
    /// Human-readable version string ([`HARVEST_SPEC_VERSION`] as of
    /// this SDK release).
    pub spec_version: String,
    /// Origin service slug (`"character.ai"`, `"kindroid"`,
    /// `"replika"`, `"grok"`, …). Flows into every emitted
    /// footprint's `labels` and `extra.service`.
    pub service: String,
    /// External user id at the origin service (opaque string). Optional.
    #[serde(default)]
    pub service_user_id: Option<String>,
    /// Wall-clock time the harvest ran. Used as a fallback occurred-at
    /// when neither the message nor the conversation carries one.
    #[serde(default)]
    pub harvested_at: Option<DateTime<Utc>>,
    /// Optional character metadata. Not emitted as footprints in v1
    /// (see the module rustdoc); reserved for v2 promotion.
    #[serde(default)]
    pub characters: Vec<HarvestCharacter>,
    /// The conversations captured in this batch.
    #[serde(default)]
    pub conversations: Vec<HarvestConversation>,
    /// Envelope-level extension bag. Merged verbatim into every
    /// emitted footprint's `extra.envelope_extra` for downstream
    /// query.
    #[serde(default)]
    pub extra: Value,
}

/// Optional character metadata (persona-side info the harvester
/// picked up alongside the conversations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestCharacter {
    /// Stable id at the origin service. Used as `character_id`
    /// foreign key from [`HarvestConversation`].
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional long-form character description.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional avatar. Data URI (`data:image/png;base64,…`) or HTTP
    /// URL — the SDK does not fetch remotes, so a v2 emitter should
    /// prefer inline data URIs when the intent is to persist the
    /// avatar.
    #[serde(default)]
    pub avatar_uri: Option<String>,
    /// Service-specific extension bag.
    #[serde(default)]
    pub extra: Value,
}

/// One conversation. All messages share the conversation's `id` as
/// their session_id so `edge_rebuild` clusters them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestConversation {
    /// Stable conversation id at the origin service. Becomes the
    /// `session_id` on every emitted footprint.
    pub id: String,
    /// Optional foreign key to [`HarvestCharacter::id`].
    #[serde(default)]
    pub character_id: Option<String>,
    /// Human-readable title (sidebar name, first-message excerpt, …).
    #[serde(default)]
    pub title: Option<String>,
    /// Conversation start time. Used as fallback for messages without
    /// their own `timestamp`.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// Messages in the conversation, in author order.
    pub messages: Vec<HarvestMessage>,
    /// Conversation-level extension bag.
    #[serde(default)]
    pub extra: Value,
}

/// One message inside a conversation. Maps to
/// [`crate::ChatMessage`] verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarvestMessage {
    /// Origin-service message id, and the message's whole claim to an
    /// address: the parser emits `#conversation=<id>/message=<id>`
    /// from it. **A message that omits this is dropped** — its
    /// position in the array is the array's property, not the
    /// message's, and addressing by it moves every message behind an
    /// insert onto its neighbour's row (see [`crate::parser`]). A
    /// converter written against this schema should carry the origin
    /// service's own id through, or synthesise a stable one from the
    /// message's content; the drop is counted and reported per
    /// container, not silent, but it is still a drop.
    #[serde(default)]
    pub id: Option<String>,
    /// Author role slug — `"user"` / `"assistant"` / `"system"` /
    /// `"tool"` / any other string (falls through to
    /// [`crate::ChatRole::Other`]).
    pub role: String,
    /// Message text.
    pub body: String,
    /// Send time. Falls back to
    /// `conversation.started_at → envelope.harvested_at →
    /// RawItem.occurred_at → Utc::now()`.
    #[serde(default)]
    pub timestamp: Option<DateTime<Utc>>,
    /// Parent message id when the conversation is tree-shaped
    /// (ChatGPT-style branching). Linear chats leave this `null`.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Message-level extension bag (model slug, token count,
    /// thumbs-up/down, …).
    #[serde(default)]
    pub extra: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_envelope_roundtrips() {
        let raw = r#"{
            "spec": "asterism_agent_harvest",
            "spec_version": "1.0",
            "service": "character.ai",
            "conversations": [
                {"id": "c1", "messages": [{"role": "user", "body": "hi"}]}
            ]
        }"#;
        let env: HarvestEnvelope = serde_json::from_str(raw).unwrap();
        assert_eq!(env.spec, HARVEST_SPEC);
        assert_eq!(env.service, "character.ai");
        assert_eq!(env.conversations.len(), 1);
        assert_eq!(env.conversations[0].messages.len(), 1);
        assert_eq!(env.conversations[0].messages[0].role, "user");
    }

    #[test]
    fn example_fixture_parses() {
        let raw = include_str!("../../schema/agent_harvest.example.json");
        let env: HarvestEnvelope = serde_json::from_str(raw).expect("example JSON parses");
        assert_eq!(env.spec, HARVEST_SPEC);
        assert_eq!(env.spec_version, HARVEST_SPEC_VERSION);
        assert_eq!(env.conversations[0].messages.len(), 2);
    }
}
