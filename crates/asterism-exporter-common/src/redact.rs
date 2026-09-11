//! Taking a credential back out of what an adapter wrote down.
//!
//! An exporter records the call it made — the request as sent, the
//! backend's answer, the text of an error — and every one of those
//! reaches the dispatch row, which is handed back on every read of the
//! dispatch. A credential that travelled in any of them would be
//! readable by anything that can list dispatches, so the recorded copy
//! passes through here first.
//!
//! # What a scrub can and cannot reach
//!
//! It removes a value the adapter was *told* is a credential. A token a
//! profile built out of its own params and interpolated into a URL or a
//! body is not one of those: the adapter never learned it was a secret,
//! and no amount of searching here would find it. The way out of that is
//! for the profile to name the credential — which is what the adapters'
//! own `secret_ref`-style blocks are for, and what their docs say.
//!
//! # Why an empty value is left alone
//!
//! Replacing the empty substring would rewrite every string it is asked
//! about. A variable set to the empty string is a profile pointing at
//! nothing rather than a secret to hide, and an unset one never reaches
//! here — an adapter fails at resolution.

use asterism_dispatch_sdk::ExporterError;
use serde_json::Value;

/// What a redacted value is replaced by wherever an adapter writes down
/// a call it made.
///
/// One token for every adapter rather than one each: a reader looking at
/// an attempt record should not have to know which adapter wrote it in
/// order to recognise that something was taken out.
pub const REDACTED: &str = "«redacted»";

/// The credentials one call was made with, so they can be taken back out
/// of what it recorded.
///
/// Holds values rather than reading them per scrub, because a single
/// call must not straddle an environment change and redact against one
/// credential what it sent with another.
#[derive(Debug, Clone, Default)]
pub struct Redaction {
    values: Vec<String>,
}

impl Redaction {
    /// The scrub for a call made with these values. Empty ones are
    /// dropped, for the reason the module docs give.
    pub fn of(values: impl IntoIterator<Item = String>) -> Self {
        Self {
            values: values.into_iter().filter(|v| !v.is_empty()).collect(),
        }
    }

    /// The scrub for a call made with no credential at all: the
    /// identity, because there is nothing to look for.
    pub fn none() -> Self {
        Self::default()
    }

    /// Removes every credential from one string.
    pub fn text(&self, s: &str) -> String {
        let mut out = s.to_string();
        for value in &self.values {
            out = out.replace(value.as_str(), REDACTED);
        }
        out
    }

    /// Removes every credential from every string in a JSON document.
    pub fn json(&self, value: Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.text(&s)),
            Value::Array(items) => Value::Array(items.into_iter().map(|v| self.json(v)).collect()),
            Value::Object(fields) => {
                Value::Object(fields.into_iter().map(|(k, v)| (k, self.json(v))).collect())
            }
            other => other,
        }
    }

    /// Rewrites an error so its message cannot carry a credential.
    ///
    /// The runner persists an [`ExporterError`]'s `to_string()` verbatim
    /// as the dispatch's failure message, and that message is handed
    /// back on every read of the dispatch. A URL with the credential in
    /// its query and a backend that echoes what it was sent both arrive
    /// here.
    pub fn error(&self, err: ExporterError) -> ExporterError {
        match err {
            ExporterError::BackendRejected(message) => {
                ExporterError::BackendRejected(self.text(&message))
            }
            ExporterError::Other(err) => {
                ExporterError::Other(anyhow::anyhow!("{}", self.text(&err.to_string())))
            }
            // Neither carries adapter-composed text: both are built from
            // slugs the core owns.
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_credential_goes_at_every_depth() {
        let scrub = Redaction::of(["k-123".to_string(), "pass-9".to_string()]);

        assert_eq!(
            scrub.json(json!({
                "nested": { "auth": "Key k-123" },
                "list": ["pass-9", 7],
                "kept": "unrelated"
            })),
            json!({
                "nested": { "auth": format!("Key {REDACTED}") },
                "list": [REDACTED, 7],
                "kept": "unrelated"
            })
        );
    }

    #[test]
    fn an_empty_credential_does_not_rewrite_everything() {
        let scrub = Redaction::of([String::new()]);

        assert_eq!(scrub.text("nothing to hide"), "nothing to hide");
    }

    #[test]
    fn an_error_message_cannot_carry_a_credential() {
        let scrub = Redaction::of(["k-123".to_string()]);

        let text = scrub
            .error(ExporterError::BackendRejected(
                "PUT sftp://host/dir failed for k-123".into(),
            ))
            .to_string();

        assert!(!text.contains("k-123"), "{text}");
        assert!(text.contains(REDACTED), "{text}");
    }

    #[test]
    fn with_nothing_to_look_for_the_scrub_is_the_identity() {
        assert_eq!(Redaction::none().text("Key k-123"), "Key k-123");
    }
}
