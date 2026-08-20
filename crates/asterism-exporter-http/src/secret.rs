//! The profile grammar this adapter uses: the shared one, plus
//! `{{secret}}` when the profile names a credential.
//!
//! The root is bound per call, from the environment variable an `auth`
//! block names, and the value is never written into the params blob —
//! which matters because that blob is persisted unedited and handed back
//! on every read of the dispatch. A profile that instead interpolates a
//! token it stored in its own params (`{{params.extras.api_key}}`, the
//! pattern this adapter shipped with before it had an `auth` block) is
//! still allowed and still works; what it loses is exactly this: nothing
//! can scrub a value the adapter was never told was a credential.
//!
//! A profile with no `auth` block binds no secret. `{{secret}}` is then
//! an error rather than an empty string: rendering it away would send an
//! unauthenticated request that looks like an authenticated one, and the
//! backend's 401 is a worse place to learn it than the dispatch that
//! refuses to start.
//!
//! Only [`TemplateAdapter::render`] is overridden. The JSON-leaf and
//! header traversals are the trait's default methods, written in terms
//! of `render`, so `{{secret}}` means the same thing in a header, in a
//! body field and in a query string without three implementations
//! agreeing to.

use std::collections::BTreeMap;

use asterism_dispatch_sdk::ExporterError;
use asterism_exporter_common::{CommonExportAdapter, ResponsePath, TemplateAdapter, TemplateEnv};
use serde_json::Value;

use crate::REDACTED;

/// The placeholder that resolves to the profile's credential.
const SECRET_PLACEHOLDER: &str = "{{secret}}";

/// [`CommonExportAdapter`] plus the `{{secret}}` root.
#[derive(Debug, Clone, Default)]
pub struct SecretGrammar {
    secret: Option<String>,
}

impl SecretGrammar {
    /// Binds a resolved credential for the duration of one call.
    ///
    /// Held by value rather than re-read per placeholder so a single
    /// call cannot straddle an environment change and send one header
    /// with the old credential and another with the new.
    pub fn new(secret: String) -> Self {
        Self {
            secret: Some(secret),
        }
    }

    /// The grammar for a profile that names no credential.
    ///
    /// Everything the shared adapter resolves still resolves; only
    /// `{{secret}}` is refused, and the scrubs become the identity —
    /// there is no value to look for.
    pub fn unauthenticated() -> Self {
        Self { secret: None }
    }

    /// Replaces the auth header's value, and scrubs the credential from
    /// every other header value.
    ///
    /// Both halves are needed. The named header is redacted whatever it
    /// contains, because that is where the credential is by
    /// construction; the scrub catches a profile that also reached for
    /// `{{secret}}` somewhere else, which is allowed and would
    /// otherwise be recorded verbatim.
    ///
    /// `auth_header` is `None` for a profile that names no credential,
    /// and then **every** header value is redacted rather than none. The
    /// adapter cannot tell which of them a profile built out of its own
    /// params carries a token — `authorization` is where both shipped
    /// examples put one — so the recorded copy keeps the header names
    /// and drops the values. The residue is stated where the record is
    /// documented: a credential a profile interpolates into a URL or a
    /// body from its params is recorded as sent, and the way out of that
    /// is an `auth` block rather than a cleverer guess here.
    pub fn redact_headers(
        &self,
        headers: &BTreeMap<String, String>,
        auth_header: Option<&str>,
    ) -> BTreeMap<String, String> {
        headers
            .iter()
            .map(|(name, value)| {
                let redacted = match auth_header {
                    Some(auth) if name.eq_ignore_ascii_case(auth) => REDACTED.to_string(),
                    Some(_) => self.scrub_str(value),
                    None => REDACTED.to_string(),
                };
                (name.clone(), redacted)
            })
            .collect()
    }

    /// Removes the credential from one string.
    ///
    /// Public because a rendered string reaches the dispatch row by more
    /// routes than the JSON body: the URL a profile built with
    /// `{{secret}}` in its query, and the text of an error that quotes
    /// that URL back. Every one of them has to pass through here, so the
    /// scrub is not a property of the body it was written for.
    pub fn scrub_text(&self, s: &str) -> String {
        self.scrub_str(s)
    }

    /// Rewrites an error so its message cannot carry the credential.
    ///
    /// The runner persists an `ExporterError`'s `to_string()` verbatim
    /// as the dispatch's failure message
    /// (`asterism_infra::dispatch::runtime`), and that message is handed
    /// back on every read of the dispatch. A URL with the key in its
    /// query and a backend that echoes the request both arrive here.
    pub fn scrub_error(&self, err: ExporterError) -> ExporterError {
        match err {
            ExporterError::BackendRejected(message) => {
                ExporterError::BackendRejected(self.scrub_str(&message))
            }
            ExporterError::Other(err) => {
                ExporterError::Other(anyhow::anyhow!("{}", self.scrub_str(&err.to_string())))
            }
            // Neither carries adapter-composed text: both are built from
            // slugs the core owns.
            other => other,
        }
    }

    /// Removes the credential from every string in a JSON document.
    pub fn scrub(&self, value: Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.scrub_str(&s)),
            Value::Array(items) => Value::Array(items.into_iter().map(|v| self.scrub(v)).collect()),
            Value::Object(fields) => Value::Object(
                fields
                    .into_iter()
                    .map(|(k, v)| (k, self.scrub(v)))
                    .collect(),
            ),
            other => other,
        }
    }

    /// Scrubs one string.
    ///
    /// An empty credential would make this replace every empty
    /// substring, so it is left alone — an unset variable never gets
    /// this far (the exporter fails at resolution), and a variable set
    /// to the empty string is a profile pointing at nothing rather than
    /// a secret to hide. No credential at all is the same identity, for
    /// the same reason: there is nothing to look for.
    fn scrub_str(&self, s: &str) -> String {
        match self.secret.as_deref() {
            Some(secret) if !secret.is_empty() => s.replace(secret, REDACTED),
            _ => s.to_string(),
        }
    }
}

impl TemplateAdapter for SecretGrammar {
    fn render(&self, template: &str, env: &TemplateEnv<'_>) -> Result<String, ExporterError> {
        let Some(secret) = self.secret.as_deref() else {
            if template.contains(SECRET_PLACEHOLDER) {
                return Err(ExporterError::BackendRejected(format!(
                    "template uses {SECRET_PLACEHOLDER} but the profile has no auth block naming a credential"
                )));
            }
            return CommonExportAdapter.render(template, env);
        };
        // Substituted before the shared engine runs, so the engine
        // never sees a root it does not know and never reports
        // `{{secret}}` as unresolved. The credential is substituted
        // into the *output*, not into the template the engine parses,
        // so a credential that happens to contain `{{` cannot become a
        // placeholder.
        let mut out = String::with_capacity(template.len());
        for (index, piece) in template.split(SECRET_PLACEHOLDER).enumerate() {
            if index > 0 {
                out.push_str(secret);
            }
            out.push_str(&CommonExportAdapter.render(piece, env)?);
        }
        Ok(out)
    }
}

impl ResponsePath for SecretGrammar {
    fn select(&self, root: &Value, expr: &str) -> Vec<Value> {
        CommonExportAdapter.select(root, expr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_dispatch_sdk::DispatchContext;
    use serde_json::json;

    fn env_ctx<'a>(params: &'a Value) -> DispatchContext<'a> {
        DispatchContext {
            inputs: &[],
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "persona-1",
            action: "txt2img",
            params,
            attempt: &asterism_dispatch_sdk::DISCARD_ATTEMPTS,
        }
    }

    #[test]
    fn the_secret_root_resolves_and_the_shared_roots_still_do() {
        let params = json!({ "model": "flux" });
        let ctx = env_ctx(&params);
        let env = TemplateEnv::pre_handle(&ctx, &params);
        let grammar = SecretGrammar::new("k-123".into());

        assert_eq!(grammar.render("Key {{secret}}", &env).unwrap(), "Key k-123");
        assert_eq!(
            grammar
                .render("{{params.model}}/{{dispatch_id}}", &env)
                .unwrap(),
            "flux/disp-1"
        );
    }

    /// The trait's default traversals have to pick the override up, or
    /// a profile could put `{{secret}}` in a body field and get an
    /// unresolved-placeholder error instead of the credential.
    #[test]
    fn the_secret_root_reaches_bodies_and_headers_too() {
        let params = json!({});
        let ctx = env_ctx(&params);
        let env = TemplateEnv::pre_handle(&ctx, &params);
        let grammar = SecretGrammar::new("k-123".into());

        assert_eq!(
            grammar
                .render_json(&json!({ "key": "{{secret}}", "steps": 20 }), &env)
                .unwrap(),
            json!({ "key": "k-123", "steps": 20 })
        );
        assert_eq!(
            grammar
                .render_headers(
                    &BTreeMap::from([("x-key".to_string(), "{{secret}}".to_string())]),
                    &env
                )
                .unwrap()["x-key"],
            "k-123"
        );
    }

    /// A credential is arbitrary bytes. One containing `{{` must not be
    /// re-parsed as a placeholder by the engine it was substituted for.
    #[test]
    fn a_credential_that_looks_like_a_placeholder_is_not_re_expanded() {
        let params = json!({});
        let ctx = env_ctx(&params);
        let env = TemplateEnv::pre_handle(&ctx, &params);
        let grammar = SecretGrammar::new("{{dispatch_id}}".into());

        assert_eq!(
            grammar.render("Key {{secret}}", &env).unwrap(),
            "Key {{dispatch_id}}"
        );
    }

    #[test]
    fn the_auth_header_is_redacted_by_name_and_the_value_scrubbed_elsewhere() {
        let grammar = SecretGrammar::new("k-123".into());
        let headers = BTreeMap::from([
            ("Authorization".to_string(), "Key k-123".to_string()),
            ("x-echo".to_string(), "prefix k-123 suffix".to_string()),
            ("x-plain".to_string(), "nothing here".to_string()),
        ]);

        let redacted = grammar.redact_headers(&headers, Some("authorization"));
        assert_eq!(redacted["Authorization"], REDACTED);
        assert_eq!(redacted["x-echo"], format!("prefix {REDACTED} suffix"));
        assert_eq!(redacted["x-plain"], "nothing here");
    }

    /// A profile with no `auth` block can still carry a token — in its
    /// own params, the pattern this adapter shipped with. Nothing here
    /// knows which header that is, so the recorded copy keeps the names
    /// and drops every value rather than guessing one of them.
    #[test]
    fn without_a_credential_every_recorded_header_value_goes() {
        let grammar = SecretGrammar::unauthenticated();
        let headers = BTreeMap::from([
            (
                "Authorization".to_string(),
                "Bearer from-params".to_string(),
            ),
            ("x-plain".to_string(), "nothing here".to_string()),
        ]);

        let redacted = grammar.redact_headers(&headers, None);
        assert_eq!(redacted["Authorization"], REDACTED);
        assert_eq!(redacted["x-plain"], REDACTED);
        assert_eq!(redacted.len(), 2, "the names stay: {redacted:?}");
    }

    /// Rendering `{{secret}}` away as an empty string would send an
    /// unauthenticated request shaped like an authenticated one. The
    /// dispatch refuses instead, naming what the profile is missing.
    #[test]
    fn the_secret_root_without_an_auth_block_is_refused() {
        let params = json!({});
        let ctx = env_ctx(&params);
        let env = TemplateEnv::pre_handle(&ctx, &params);

        let err = SecretGrammar::unauthenticated()
            .render("Key {{secret}}", &env)
            .expect_err("no credential is bound");
        assert!(err.to_string().contains("auth block"), "{err}");

        // Everything else the shared grammar resolves still resolves.
        assert_eq!(
            SecretGrammar::unauthenticated()
                .render("{{dispatch_id}}", &env)
                .expect("the shared roots do not need a credential"),
            "disp-1"
        );
    }

    /// Query-parameter auth is a real shape, so a profile is allowed to
    /// put `{{secret}}` in a path. What it must not do is put the
    /// rendered result on the dispatch row.
    #[test]
    fn a_url_carrying_the_credential_is_scrubbed() {
        let grammar = SecretGrammar::new("k-123".into());
        assert_eq!(
            grammar.scrub_text("https://api.test/run?api_key=k-123&n=1"),
            format!("https://api.test/run?api_key={REDACTED}&n=1")
        );
    }

    /// The runner persists an error's text as the dispatch's failure
    /// message, so an error quoting a URL or echoing a response body is
    /// a write to the row.
    #[test]
    fn an_error_message_cannot_carry_the_credential() {
        let grammar = SecretGrammar::new("k-123".into());

        let rejected = grammar.scrub_error(ExporterError::BackendRejected(
            "POST https://api.test/run?key=k-123 HTTP 401: {\"sent\":\"k-123\"}".into(),
        ));
        let text = rejected.to_string();
        assert!(!text.contains("k-123"), "{text}");
        assert_eq!(text.matches(REDACTED).count(), 2, "{text}");

        let other = grammar
            .scrub_error(ExporterError::Other(anyhow::anyhow!(
                "GET https://api.test/x?key=k-123: connection reset"
            )))
            .to_string();
        assert!(!other.contains("k-123"), "{other}");
    }

    #[test]
    fn the_credential_is_scrubbed_at_every_depth_of_a_document() {
        let grammar = SecretGrammar::new("k-123".into());
        assert_eq!(
            grammar.scrub(json!({
                "nested": { "auth": "Key k-123" },
                "list": ["k-123", 7],
                "kept": "unrelated"
            })),
            json!({
                "nested": { "auth": format!("Key {REDACTED}") },
                "list": [REDACTED, 7],
                "kept": "unrelated"
            })
        );
    }
}
