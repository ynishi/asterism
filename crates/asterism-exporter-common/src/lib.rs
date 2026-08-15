//! # asterism-exporter-common
//!
//! What a *schema-driven* exporter needs in order to be configured
//! rather than written: a `{{...}}` substitution over the dispatch
//! ([`template`]) and a path grammar for reading the backend's answer
//! ([`jsonpath`]). Two adapters want both — `asterism-exporter-http`
//! today, the cloud adapter next — and a grammar with two spellings is
//! worse than either spelling on its own.
//!
//! ## Why not in the SDK
//!
//! `asterism-dispatch-sdk` is the port. It publishes the `Exporter`
//! trait, the types that cross it, and the schema artifacts a backend
//! author reads, and an adapter that hard-codes its backend's protocol
//! implements that port without ever meeting a template. Shared
//! *implementation* between adapters is a different thing from the
//! contract adapters are written against, and it belongs one layer
//! out: this crate depends on the SDK, the SDK does not know this
//! crate exists.
//!
//! ## The traits, and what they are for
//!
//! A concrete adapter does not reach for [`template::render`] and
//! [`jsonpath::many`] directly — it takes [`TemplateAdapter`] and
//! [`ResponsePath`] and receives [`CommonExportAdapter`] as the default
//! implementation of both. That is one line more at the definition
//! site, and it buys the thing the direct call cannot: the grammar
//! becomes substitutable per adapter without every adapter's call sites
//! changing shape.
//!
//! That is not hypothetical. A cloud profile resolves its credential
//! from the environment rather than out of the params blob, which means
//! a placeholder root the HTTP adapter must not have — an adapter that
//! wraps [`CommonExportAdapter`] and overrides [`TemplateAdapter::render`]
//! adds it without touching the shared engine, and the JSON-leaf and
//! header traversals keep working because they are default methods
//! written in terms of `render`.
//!
//! ```no_run
//! use asterism_exporter_common::{CommonExportAdapter, ResponsePath, TemplateAdapter};
//!
//! struct MyExporter<A = CommonExportAdapter> {
//!     grammar: A,
//! }
//!
//! impl<A: TemplateAdapter + ResponsePath> MyExporter<A> {
//!     fn status(&self, response: &serde_json::Value) -> Option<String> {
//!         self.grammar
//!             .select_first(response, "$.status")
//!             .and_then(|v| self.grammar.display_string(v))
//!     }
//! }
//! ```

pub mod jsonpath;
pub mod template;

use std::collections::BTreeMap;

use asterism_dispatch_sdk::ExporterError;
use serde_json::Value;

pub use template::TemplateEnv;

/// The `{{...}}` half of a profile's grammar.
///
/// Only [`TemplateAdapter::render`] is required. The rest are traversals
/// expressed in terms of it, so an implementation that changes what a
/// placeholder resolves to inherits consistent behaviour on JSON bodies
/// and header maps instead of having to restate it — and, more to the
/// point, cannot accidentally give a placeholder one meaning in a body
/// and another in a header.
pub trait TemplateAdapter {
    /// Renders one template string, resolving each placeholder against
    /// `env`.
    fn render(&self, template: &str, env: &TemplateEnv<'_>) -> Result<String, ExporterError>;

    /// Renders every string leaf of a JSON document in place.
    ///
    /// Numbers, booleans and nulls pass through untouched, so a body
    /// template can carry a real `"steps": 20` rather than a string the
    /// backend has to coerce.
    fn render_json(&self, value: &Value, env: &TemplateEnv<'_>) -> Result<Value, ExporterError> {
        match value {
            Value::String(s) => Ok(Value::String(self.render(s, env)?)),
            Value::Array(arr) => arr
                .iter()
                .map(|v| self.render_json(v, env))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Value::Object(obj) => {
                let mut out = serde_json::Map::with_capacity(obj.len());
                for (k, v) in obj {
                    out.insert(k.clone(), self.render_json(v, env)?);
                }
                Ok(Value::Object(out))
            }
            other => Ok(other.clone()),
        }
    }

    /// Renders every value of a header map. Keys are copied verbatim —
    /// a templated header *name* has no use case and would make a
    /// profile's headers impossible to read at a glance.
    fn render_headers(
        &self,
        headers: &BTreeMap<String, String>,
        env: &TemplateEnv<'_>,
    ) -> Result<BTreeMap<String, String>, ExporterError> {
        let mut out = BTreeMap::new();
        for (k, v) in headers {
            out.insert(k.clone(), self.render(v, env)?);
        }
        Ok(out)
    }

    /// How a resolved JSON value is spelled when it lands in a string.
    ///
    /// Shared with the path half: an adapter reading a progress message
    /// out of a response spells it the same way a template would.
    fn display_string(&self, value: Value) -> Option<String> {
        template::value_to_display_string(value)
    }
}

/// The path half of a profile's grammar — how an adapter reads the
/// backend's answer.
///
/// Missing is not an error here. A path that matches nothing yields an
/// empty vector, and the caller decides what that means: a poll
/// predicate reads it as "not yet", a handle extraction reads it as a
/// backend that did not answer the way its profile said it would.
pub trait ResponsePath {
    /// Every value the expression selects, in document order.
    fn select(&self, root: &Value, expr: &str) -> Vec<Value>;

    /// The first value the expression selects, or `None`.
    fn select_first(&self, root: &Value, expr: &str) -> Option<Value> {
        self.select(root, expr).into_iter().next()
    }
}

/// The implementation both adapters get unless they say otherwise:
/// [`template`] for substitution, [`jsonpath`] for selection.
///
/// A unit struct. It carries no configuration because the grammars
/// carry none — they are documented in the params example a profile
/// author reads, and a knob here would be a knob that example cannot
/// show.
#[derive(Debug, Clone, Copy, Default)]
pub struct CommonExportAdapter;

impl TemplateAdapter for CommonExportAdapter {
    fn render(&self, template: &str, env: &TemplateEnv<'_>) -> Result<String, ExporterError> {
        template::render(template, env)
    }
}

impl ResponsePath for CommonExportAdapter {
    fn select(&self, root: &Value, expr: &str) -> Vec<Value> {
        jsonpath::many(root, expr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_contract::dto::AssetCardDto;
    use asterism_dispatch_sdk::DispatchContext;
    use serde_json::json;

    fn ctx<'a>(inputs: &'a [AssetCardDto], params: &'a Value) -> DispatchContext<'a> {
        DispatchContext {
            inputs,
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            pursuit_id: None,
            persona_id: "persona-1",
            action: "run",
            params,
        }
    }

    /// An adapter that overrides only `render` — the case the trait
    /// exists for. Its JSON and header traversals have to pick the
    /// override up, or the default methods are a trap rather than a
    /// convenience.
    struct Shouty;

    impl TemplateAdapter for Shouty {
        fn render(&self, template: &str, env: &TemplateEnv<'_>) -> Result<String, ExporterError> {
            Ok(CommonExportAdapter.render(template, env)?.to_uppercase())
        }
    }

    #[test]
    fn the_default_traversals_are_written_in_terms_of_render() {
        let params = json!({ "who": "dolly" });
        let c = ctx(&[], &params);
        let env = TemplateEnv::pre_handle(&c, &params);

        // Non-strings pass through untouched at every depth, so a body
        // template keeps a real `"steps": 20` instead of a string the
        // backend has to coerce.
        assert_eq!(
            Shouty
                .render_json(
                    &json!({
                        "prompt": "{{params.who}}",
                        "steps": 20,
                        "flags": [true, "{{dispatch_id}}"]
                    }),
                    &env
                )
                .unwrap(),
            json!({ "prompt": "DOLLY", "steps": 20, "flags": [true, "DISP-1"] })
        );

        let headers = BTreeMap::from([("x-who".to_string(), "{{params.who}}".to_string())]);
        assert_eq!(
            Shouty.render_headers(&headers, &env).unwrap(),
            BTreeMap::from([("x-who".to_string(), "DOLLY".to_string())])
        );
    }

    #[test]
    fn the_shipped_adapter_delegates_to_both_grammars() {
        let params = json!({ "who": "dolly" });
        let c = ctx(&[], &params);
        let env = TemplateEnv::pre_handle(&c, &params);

        assert_eq!(
            CommonExportAdapter.render("{{params.who}}", &env).unwrap(),
            "dolly"
        );

        let resp = json!({ "outputs": [{ "url": "a.png" }, { "url": "b.png" }] });
        assert_eq!(
            CommonExportAdapter.select(&resp, "$.outputs[*].url"),
            vec![json!("a.png"), json!("b.png")]
        );
        assert_eq!(
            CommonExportAdapter.select_first(&resp, "$.outputs[*].url"),
            Some(json!("a.png"))
        );
    }
}
