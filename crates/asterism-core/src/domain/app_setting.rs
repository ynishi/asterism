//! Application settings — the closed key registry and its stored
//! overrides (the `app_setting` table).
//!
//! Two-layer model, mirroring [`Modality`](crate::domain::value::Modality)
//! / [`ContentKind`](crate::domain::value::ContentKind):
//!
//! - **Closed** — [`SETTING_REGISTRY`] enumerates every key the
//!   application understands, together with its value kind, its code
//!   default, and (optionally) the environment variable that seeds it.
//!   A key that is not in the registry cannot be written.
//! - **Open** — the `app_setting` table holds only the rows a user has
//!   actually changed. An unset key is not a row; it resolves to the
//!   registry default.
//!
//! ## Why the backend owns this
//!
//! `localStorage` is reachable only from the webview, so `--headless`
//! runs, the HTTP server, and the job engine could never read a user
//! preference. Keeping settings in the profile database also means they
//! travel with the profile: one `asterism.db` backup carries the data
//! *and* the preferences, and `dev` / `dogfood` / `bench` stay isolated
//! from each other for free.
//!
//! ## Resolution order
//!
//! `default` → `env var` → `stored row` (last wins). **A value the user
//! chose outranks the environment**, uniformly, for every key.
//!
//! The environment is a *seed*, not a ceiling: it supplies the value
//! while nothing is stored, and steps aside once someone sets one. This
//! is the shape Open WebUI settled on for the same reason — a control
//! that accepts input and then has it silently discarded by a
//! process-wide variable breaks the only contract a settings screen
//! has.
//!
//! The inverse order (`env` last) is the convention for *deployment*
//! configuration, where an operator's variable must beat whatever a file
//! says. These keys are user preferences, so that convention does not
//! apply; mixing the two ownership domains in one key namespace is what
//! made the earlier ordering wrong. If a key ever genuinely needs
//! enforcement, the established answer is a separate, explicitly-named
//! lock — not a reordering of this stack.
//!
//! Every layer that has a value is kept, not just the winner
//! ([`EffectiveSetting::layers`]), so a caller can show where a value
//! came from and what it is shadowing. Collapsing to the winner is what
//! previously made a shadowed stored row invisible *and* unreachable.

use chrono::{DateTime, Utc};

use crate::error::DomainError;

/// Value shape a setting accepts. Kept deliberately small — a setting
/// that wants a nested object is a sign it should be its own aggregate,
/// not a row in this table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingValueKind {
    /// JSON `true` / `false`.
    Bool,
    /// JSON integer (`i64`).
    Int,
    /// JSON string.
    Text,
}

impl SettingValueKind {
    /// Stable slug used on the wire and in error messages.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Int => "int",
            Self::Text => "text",
        }
    }

    /// Checks that `json` parses and matches this kind. Returns the
    /// canonical re-serialisation so stored rows are normalised (`1 `
    /// and `1` become the same bytes).
    pub fn canonicalise(self, json: &str) -> Result<String, DomainError> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| {
            DomainError::Validation(format!("setting value is not valid JSON: {e}"))
        })?;
        let matches = match self {
            Self::Bool => value.is_boolean(),
            Self::Int => value.is_i64(),
            Self::Text => value.is_string(),
        };
        if !matches {
            return Err(DomainError::Validation(format!(
                "setting value {json} does not match kind {}",
                self.as_str()
            )));
        }
        Ok(value.to_string())
    }
}

/// One entry of the closed registry: the contract for a single key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingDef {
    /// Dotted key (`ui.clean_mode`). Namespaced by the surface that
    /// owns the behaviour, not by the widget that renders it.
    pub key: &'static str,
    /// Accepted value shape.
    pub kind: SettingValueKind,
    /// Code default, as JSON text. Applies when no row is stored.
    pub default_json: &'static str,
    /// Environment variable that seeds this key: it supplies the value
    /// while no row is stored, and steps aside once one is. `None` for
    /// preferences with no launch-time entry point.
    pub env_var: Option<&'static str>,
    /// Inclusive `(min, max)` bound for [`SettingValueKind::Int`] keys.
    /// `None` leaves the value unbounded (and is the only valid choice
    /// for `Bool` / `Text`, which have no ordering to bound).
    ///
    /// The bound lives here rather than in a client because the HTTP
    /// surface is reachable without going through any UI — a clamp in
    /// the settings screen would leave `PUT /asterism/settings/{key}`
    /// able to store a value that wedges the next launch.
    pub range: Option<(i64, i64)>,
    /// One-line description surfaced in the settings UI.
    pub summary: &'static str,
}

impl SettingDef {
    /// Canonicalises `json` against this key's kind, then applies the
    /// declared [`Self::range`].
    ///
    /// Range violations are `Validation` (a malformed request), not
    /// `Conflict`: the caller sent a value this key never accepts.
    pub fn canonicalise(&self, json: &str) -> Result<String, DomainError> {
        let value = self.kind.canonicalise(json)?;
        let Some((min, max)) = self.range else {
            return Ok(value);
        };
        // `canonicalise` already established the kind, so an Int parses.
        let n: i64 = value.parse().map_err(|_| {
            DomainError::Validation(format!("setting {} is not an integer: {value}", self.key))
        })?;
        if n < min || n > max {
            return Err(DomainError::Validation(format!(
                "setting {} must be between {min} and {max}, got {n}",
                self.key
            )));
        }
        Ok(value)
    }
}

/// Every key the application understands.
///
/// Adding a key is a code change on purpose: the default and the value
/// kind are part of the application's contract, and a typo in a free-form
/// key would otherwise silently create a setting nothing reads.
pub const SETTING_REGISTRY: &[SettingDef] = &[
    SettingDef {
        key: "ui.clean_mode",
        kind: SettingValueKind::Bool,
        default_json: "false",
        env_var: None,
        range: None,
        summary: "Reduce each grid card to modality, thumbnail, persona, and basename.",
    },
    // `ui.dialogue.show_messages` was removed with the Dialogue slug
    // (asset-model v4 P3): members live inside their container's
    // reader, not interleaved into the grid. V39 deletes any stored
    // override row.
    SettingDef {
        key: "import.auto_organize",
        kind: SettingValueKind::Bool,
        default_json: "true",
        env_var: None,
        range: None,
        summary: "Rebuild the dropped folder hierarchy as dirs and groups on import.",
    },
    SettingDef {
        key: "jobs.concurrency",
        kind: SettingValueKind::Int,
        default_json: "0",
        env_var: Some("ASTERISM_JOB_CONCURRENCY"),
        // Upper bound is a guard rail, not a tuned figure: worker counts
        // this high are already far past any useful parallelism, and an
        // unbounded value reaches `WorkerBuilder::concurrency` on the
        // next launch where only an env override could undo it.
        range: Some((0, 256)),
        summary: "Job worker parallelism, applied at startup; 0 follows the machine.",
    },
    SettingDef {
        key: "dispatch.comfy.endpoint",
        kind: SettingValueKind::Text,
        default_json: "\"http://127.0.0.1:8188\"",
        env_var: None,
        range: None,
        summary: "ComfyUI base URL prefilled when composing a dispatch.",
    },
];

/// A registry-backed key. Construction is the membership check, so any
/// `SettingKey` in hand is guaranteed to name a real setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingKey(&'static SettingDef);

impl SettingKey {
    /// Resolves `key` against [`SETTING_REGISTRY`].
    pub fn parse(key: &str) -> Result<Self, DomainError> {
        SETTING_REGISTRY
            .iter()
            .find(|def| def.key == key)
            .map(Self)
            .ok_or_else(|| DomainError::not_found("setting", key))
    }

    /// The dotted key.
    pub fn as_str(self) -> &'static str {
        self.0.key
    }

    /// The registry entry behind this key.
    pub fn def(self) -> &'static SettingDef {
        self.0
    }
}

impl std::fmt::Display for SettingKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.key)
    }
}

/// A user override as stored in `app_setting`. Absence of a row is the
/// "use the default" state — there is no stored `null`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSetting {
    /// Registry key this row overrides.
    pub key: SettingKey,
    /// Canonicalised JSON value (see
    /// [`SettingValueKind::canonicalise`]).
    pub value_json: String,
    /// When the override was last written.
    pub updated_at: DateTime<Utc>,
}

/// Which layer supplied the value the application will actually use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingSource {
    /// No override anywhere; the registry default applies.
    Default,
    /// A row in `app_setting` — the user's own choice, which outranks
    /// every other layer.
    Stored,
    /// The key's environment variable, which applies while no row is
    /// stored.
    Env,
}

impl SettingSource {
    /// Stable slug used on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Stored => "stored",
            Self::Env => "env",
        }
    }
}

/// One layer that has a value for a key.
///
/// Kept even when a higher layer wins, so a caller can render the whole
/// chain rather than a single number with no provenance — the same
/// reason `git config --show-origin` lists every scope instead of only
/// the one in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingLayer {
    /// Which layer this is.
    pub source: SettingSource,
    /// The layer's value as supplied. Canonicalised and range-checked
    /// unless [`Self::rejected`] is set, in which case this is the raw
    /// text that failed.
    pub value_json: String,
    /// Where the layer's value physically comes from, when there is
    /// something to name: the variable name for
    /// [`SettingSource::Env`]. `None` for the code default and the
    /// database row, which have no address worth showing.
    pub origin: Option<&'static str>,
    /// Why this layer contributes nothing, when it does not — an
    /// exported variable that does not parse, or a stored row outside
    /// the key's range. `None` for a layer that is in play.
    ///
    /// A rejected layer stays in the chain so the reason is visible
    /// where the user is already looking. Dropping it was the same
    /// mistake as collapsing to the winner: the value exists, is
    /// disregarded, and the person who set it has no way to find out.
    ///
    /// **Invariant: a rejected layer is never the winner.** The code
    /// default cannot be rejected (the registry's own tests assert it
    /// validates), so a non-rejected layer always exists.
    pub rejected: Option<String>,
}

/// A key resolved through the full layer stack — what a caller should
/// act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSetting {
    /// Registry key.
    pub key: SettingKey,
    /// Resolved JSON value.
    pub value_json: String,
    /// Layer that supplied [`Self::value_json`].
    pub source: SettingSource,
    /// Every layer that supplied a value, ordered from lowest
    /// precedence to highest, including ones that were rejected.
    ///
    /// [`Self::value_json`] equals the last entry whose
    /// [`SettingLayer::rejected`] is `None`; entries before it are what
    /// that value shadows, and rejected entries are values that were
    /// offered and thrown away.
    ///
    /// Never empty: the code default always contributes and is never
    /// rejected.
    pub layers: Vec<SettingLayer>,
}

impl EffectiveSetting {
    /// Reads the resolved value as a bool, or `None` when the key is not
    /// of kind [`SettingValueKind::Bool`] (or the JSON disagrees).
    pub fn as_bool(&self) -> Option<bool> {
        serde_json::from_str(&self.value_json).ok()
    }

    /// Reads the resolved value as an `i64`, same contract as
    /// [`Self::as_bool`].
    pub fn as_i64(&self) -> Option<i64> {
        serde_json::from_str(&self.value_json).ok()
    }

    /// Reads the resolved value as a `String`, same contract as
    /// [`Self::as_bool`].
    pub fn as_text(&self) -> Option<String> {
        serde_json::from_str(&self.value_json).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_keys_are_unique_and_defaults_typecheck() {
        let mut seen = Vec::new();
        for def in SETTING_REGISTRY {
            assert!(
                !seen.contains(&def.key),
                "duplicate setting key in registry: {}",
                def.key
            );
            seen.push(def.key);
            def.kind
                .canonicalise(def.default_json)
                .unwrap_or_else(|e| panic!("default for {} does not match its kind: {e}", def.key));
        }
    }

    #[test]
    fn registry_ranges_only_bound_int_keys_and_admit_their_defaults() {
        for def in SETTING_REGISTRY {
            if def.range.is_some() {
                assert_eq!(
                    def.kind,
                    SettingValueKind::Int,
                    "{} declares a range but is not an int",
                    def.key
                );
            }
            // A default the range rejects would make the key
            // unwritable through its own default value.
            def.canonicalise(def.default_json)
                .unwrap_or_else(|e| panic!("default for {} violates its own range: {e}", def.key));
        }
    }

    #[test]
    fn range_rejects_values_outside_the_declared_bound() {
        let key = SettingKey::parse("jobs.concurrency").unwrap();
        let def = key.def();
        assert_eq!(def.canonicalise("0").unwrap(), "0");
        assert_eq!(def.canonicalise("256").unwrap(), "256");
        assert!(def.canonicalise("257").is_err());
        assert!(def.canonicalise("-1").is_err());
        // The kind check still runs first.
        assert!(def.canonicalise("\"8\"").is_err());
    }

    #[test]
    fn unbounded_keys_are_unaffected_by_the_range_check() {
        let key = SettingKey::parse("dispatch.comfy.endpoint").unwrap();
        assert!(key.def().range.is_none());
        assert_eq!(
            key.def().canonicalise("\"http://h:1\"").unwrap(),
            "\"http://h:1\""
        );
    }

    #[test]
    fn unknown_key_is_not_found() {
        let err = SettingKey::parse("ui.does_not_exist").unwrap_err();
        assert!(matches!(err, DomainError::NotFound { .. }));
    }

    #[test]
    fn canonicalise_rejects_kind_mismatch() {
        assert!(SettingValueKind::Bool.canonicalise("\"true\"").is_err());
        assert!(SettingValueKind::Int.canonicalise("1.5").is_err());
        assert!(SettingValueKind::Text.canonicalise("42").is_err());
        assert!(SettingValueKind::Bool.canonicalise("not json").is_err());
    }

    #[test]
    fn canonicalise_normalises_whitespace() {
        assert_eq!(SettingValueKind::Int.canonicalise(" 7 ").unwrap(), "7");
        assert_eq!(SettingValueKind::Bool.canonicalise("true").unwrap(), "true");
    }
}
