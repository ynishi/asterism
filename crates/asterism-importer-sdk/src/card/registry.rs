//! Registry that dispatches a [`CardEnvelope`] to the right
//! [`CharacterCardParser`] by its `spec` string.
//!
//! Pre-loaded defaults, first-registration-wins on collision, an
//! optional look-up hook, and a `dispatch` that returns `None` when no
//! parser claims the spec (so the caller can decide between skipping
//! and falling back to raw V2 defaults).

use crate::Footprint;

use super::envelope::{CardContext, CardEnvelope};
use super::parser::CharacterCardParser;
use super::v2::V2Parser;
use super::v3::V3Parser;

/// Routes envelopes to their parser by exact `spec()` match.
///
/// Third-party derivatives (chub / RisuAI / AgnAI / KoboldAI) register
/// themselves here at importer start-up; the pre-loaded defaults
/// cover [`V2Parser`] and [`V3Parser`].
pub struct CardParserRegistry {
    parsers: Vec<Box<dyn CharacterCardParser>>,
}

impl CardParserRegistry {
    /// Returns an empty registry. Prefer [`Self::with_defaults`]
    /// unless the caller wants full control (e.g. testing).
    pub fn empty() -> Self {
        Self {
            parsers: Vec::new(),
        }
    }

    /// Returns a registry pre-loaded with [`V2Parser`] and
    /// [`V3Parser`]. Extension parsers stack on top via
    /// [`Self::register`].
    pub fn with_defaults() -> Self {
        let mut r = Self::empty();
        r.register(Box::new(V2Parser));
        r.register(Box::new(V3Parser));
        r
    }

    /// Register a parser. First registration for a given `spec()`
    /// wins; subsequent registrations with the same spec are silently
    /// ignored so third-party packs cannot accidentally shadow a
    /// default.
    pub fn register(&mut self, parser: Box<dyn CharacterCardParser>) {
        let spec = parser.spec();
        if self.parsers.iter().any(|p| p.spec() == spec) {
            return;
        }
        self.parsers.push(parser);
    }

    /// Look up a parser by its `spec` string.
    pub fn get(&self, spec: &str) -> Option<&dyn CharacterCardParser> {
        self.parsers
            .iter()
            .find(|p| p.spec() == spec)
            .map(|b| b.as_ref())
    }

    /// Dispatch an envelope. Returns `None` when no registered parser
    /// claims the spec; the caller can then decide between skipping
    /// or forcing a fallback (e.g. `V2Parser.parse(env, ctx)` against
    /// an unknown vendor slug that is still V2-shaped).
    pub fn dispatch(&self, env: &CardEnvelope, ctx: &CardContext<'_>) -> Option<Vec<Footprint>> {
        self.get(&env.spec).map(|p| p.parse(env, ctx))
    }
}

impl Default for CardParserRegistry {
    fn default() -> Self {
        Self::with_defaults()
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
            locator: "/tmp/a.png",
            session_id: "s",
            occurred_at: Utc::now(),
            platform: None,
        }
    }

    #[test]
    fn defaults_dispatch_v2_and_v3() {
        let reg = CardParserRegistry::with_defaults();
        let env_v2 = CardEnvelope::from_json(json!({
            "spec": "chara_card_v2", "data": {"name": "A"}
        }))
        .unwrap();
        let env_v3 = CardEnvelope::from_json(json!({
            "spec": "chara_card_v3", "data": {"name": "A"}
        }))
        .unwrap();
        assert!(reg.dispatch(&env_v2, &ctx()).is_some());
        assert!(reg.dispatch(&env_v3, &ctx()).is_some());
    }

    #[test]
    fn unknown_spec_returns_none() {
        let reg = CardParserRegistry::with_defaults();
        let env = CardEnvelope::from_json(json!({
            "spec": "unknown_spec", "data": {}
        }))
        .unwrap();
        assert!(reg.dispatch(&env, &ctx()).is_none());
    }

    #[test]
    fn first_registration_wins_on_collision() {
        struct Sentinel;
        impl CharacterCardParser for Sentinel {
            fn spec(&self) -> &'static str {
                "chara_card_v2"
            }
            fn parse(&self, _env: &CardEnvelope, _ctx: &CardContext<'_>) -> Vec<Footprint> {
                Vec::new() // distinguishable from V2Parser output
            }
        }
        let mut reg = CardParserRegistry::empty();
        reg.register(Box::new(V2Parser));
        reg.register(Box::new(Sentinel)); // silently ignored
        let env = CardEnvelope::from_json(json!({
            "spec": "chara_card_v2", "data": {"name": "A"}
        }))
        .unwrap();
        let out = reg.dispatch(&env, &ctx()).unwrap();
        assert!(!out.is_empty(), "V2Parser wins, not Sentinel");
    }
}
