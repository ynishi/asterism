//! `{{...}}` substitution over a dispatch — the other half of what a
//! schema-driven exporter needs in order to be configured rather than
//! written.
//!
//! Simple textual replacement, no arithmetic and no conditionals. The
//! resolvable roots are the dispatch's own ids, the input assets, the
//! params blob itself, the handle once one exists, and the current item
//! during a per-item mapping:
//!
//! ```text
//! {{selection_id}} {{dispatch_id}} {{persona_id}} {{action}}
//! {{input[0].source_locator}}
//! {{params.some.nested.key}}
//! {{handle.job_id}}
//! {{item.url}}
//! ```
//!
//! A trailing `?` (`{{item.caption?}}`) resolves a missing path to the
//! empty string instead of failing. Without it an unresolved placeholder
//! is an error, and that is the right default: a profile that names a
//! field the backend does not send has a bug in it, and silently sending
//! an empty prompt is a worse way to find out than a rejected dispatch.
//!
//! # `params` is the caller's namespace
//!
//! The exporter knows nothing about what is in the params blob beyond
//! the fields its own schema names. `{{params.<dot.path>}}` reaches
//! anywhere inside it, so a profile author nests per-backend values
//! wherever they like and references them from templates.
//!
//! Two consequences worth stating together, because the first is the
//! reason the second matters. Params are persisted unedited and handed
//! back on every read of the dispatch — nothing on that path filters or
//! redacts. So a value reachable by `{{params.…}}` is readable by
//! anything that can list dispatches, and a credential does not belong
//! there. An adapter that needs one resolves it outside the blob.
//!
//! # Where this lives and why
//!
//! It moved into the SDK when a second exporter needed it, for the
//! reason given in [`crate::jsonpath`]: one grammar with two spellings
//! is worse than either spelling on its own.

use serde_json::{Map, Value};

use crate::exporter::{DispatchContext, ExporterError};
use asterism_contract::dto::AssetCardDto;

/// What a placeholder can be resolved against.
///
/// Built per phase: `dispatch` has no handle yet, `poll` and `harvest`
/// do, and the per-item mapping inside a harvest adds the item. The
/// phase distinction is carried in the type rather than by convention,
/// so a `{{handle.…}}` in a dispatch template resolves to nothing and
/// is reported as an unresolved placeholder instead of reaching for a
/// value that does not exist yet.
pub struct TemplateEnv<'a> {
    ctx: &'a DispatchContext<'a>,
    params: &'a Value,
    handle: Option<&'a Value>,
    item: Option<&'a Value>,
}

impl<'a> TemplateEnv<'a> {
    /// For the submit phase, before any handle exists.
    pub fn pre_handle(ctx: &'a DispatchContext<'a>, params: &'a Value) -> Self {
        Self {
            ctx,
            params,
            handle: None,
            item: None,
        }
    }

    /// For every phase after the backend has issued a handle.
    pub fn with_handle(ctx: &'a DispatchContext<'a>, params: &'a Value, handle: &'a Value) -> Self {
        Self {
            ctx,
            params,
            handle: Some(handle),
            item: None,
        }
    }

    /// Narrows an existing environment to one element of a per-item
    /// mapping. Everything else stays reachable, so an item template can
    /// still refer to `{{dispatch_id}}`.
    pub fn with_item<'b: 'a>(&'b self, item: &'a Value) -> TemplateEnv<'a> {
        TemplateEnv {
            ctx: self.ctx,
            params: self.params,
            handle: self.handle,
            item: Some(item),
        }
    }

    fn resolve(&self, key: &str, optional: bool) -> Result<String, ExporterError> {
        let value = self.lookup(key);
        match value {
            Some(v) => Ok(value_to_display_string(v).unwrap_or_default()),
            None if optional => Ok(String::new()),
            None => Err(ExporterError::BackendRejected(format!(
                "template placeholder {{{{{key}}}}} did not resolve"
            ))),
        }
    }

    fn lookup(&self, key: &str) -> Option<Value> {
        // Top-level context ids.
        match key {
            "selection_id" => return Some(Value::String(self.ctx.selection_id.into())),
            "dispatch_id" => return Some(Value::String(self.ctx.dispatch_id.into())),
            "persona_id" => return Some(Value::String(self.ctx.persona_id.into())),
            "action" => return Some(Value::String(self.ctx.action.into())),
            _ => {}
        }
        // input[N].<field>
        if let Some(rest) = key.strip_prefix("input[")
            && let Some(bracket_end) = rest.find(']')
        {
            let (idx_str, tail) = rest.split_at(bracket_end);
            let tail = &tail[1..]; // skip `]`
            let idx: usize = idx_str.parse().ok()?;
            let input = self.ctx.inputs.get(idx)?;
            let field = tail.strip_prefix('.').unwrap_or(tail);
            return input_field(input, field);
        }
        // params.<dot.path>
        if let Some(rest) = key.strip_prefix("params.") {
            return dot_path(self.params, rest);
        }
        if key == "params" {
            return Some(self.params.clone());
        }
        // handle.<dot.path>
        if let Some(rest) = key.strip_prefix("handle.") {
            return self.handle.and_then(|h| dot_path(h, rest));
        }
        if key == "handle" {
            return self.handle.cloned();
        }
        // item.<dot.path>
        if let Some(rest) = key.strip_prefix("item.") {
            return self.item.and_then(|i| dot_path(i, rest));
        }
        if key == "item" {
            return self.item.cloned();
        }
        None
    }
}

/// The input-asset fields a template may name.
///
/// An allowlist rather than a projection of the whole card: these are
/// the fields a backend can act on, and widening it later is a decision
/// about what an adapter may send outward rather than an oversight to
/// be fixed by reflection.
fn input_field(input: &AssetCardDto, field: &str) -> Option<Value> {
    match field {
        "" | "id" => Some(Value::String(input.id.clone())),
        "persona_id" => Some(Value::String(input.persona_id.clone())),
        "source_locator" => Some(Value::String(input.source_locator.clone())),
        // Unclassified inputs (asset-model v4) template as "".
        "modality" => Some(Value::String(input.modality.clone().unwrap_or_default())),
        "cover" => input.cover.clone().map(Value::String),
        _ => None,
    }
}

/// Dot-access with `[n]` indexing anywhere in the chain.
fn dot_path(root: &Value, path: &str) -> Option<Value> {
    let mut cur = root;
    for seg in path.split('.') {
        // Support `arr[0]` inside the dot chain.
        if let Some(open) = seg.find('[') {
            let field = &seg[..open];
            let rest = &seg[open + 1..];
            let close = rest.find(']')?;
            let idx: usize = rest[..close].parse().ok()?;
            let after_bracket = &rest[close + 1..];
            if !field.is_empty() {
                cur = cur.get(field)?;
            }
            cur = cur.get(idx)?;
            if !after_bracket.is_empty() {
                // Additional trailing chain like `[0].name` — recurse
                // on the remainder starting after the `].` we just
                // consumed.
                return dot_path(cur, after_bracket.trim_start_matches('.'));
            }
        } else {
            cur = cur.get(seg)?;
        }
    }
    Some(cur.clone())
}

/// How a resolved value is spelled when it lands in a string.
///
/// Null resolves to nothing at all — which an optional placeholder turns
/// into an empty string and a required one reports as unresolved. That
/// distinction is deliberate: a backend that sends `"caption": null` is
/// saying the same thing as one that omits the field, and a template
/// should not have to know which shape it is talking to.
pub fn value_to_display_string(v: Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(_) | Value::Object(_) => Some(v.to_string()),
    }
}

/// Renders one template string.
pub fn render(template: &str, env: &TemplateEnv<'_>) -> Result<String, ExporterError> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    // Byte offset where the current literal run started. Literals are
    // copied as `&str` slices rather than byte-by-byte so multi-byte
    // UTF-8 sequences survive intact. Every byte of a multi-byte
    // sequence is >= 0x80 — lead bytes (0xC2-0xF4) as well as
    // continuation bytes (0x80-0xBF) — so none can equal `b'{'`, and
    // `i` is a char boundary whenever a placeholder opens.
    // `literal_start` is a boundary for the same reason: it is either 0
    // or the offset just past a `}}`, both of which sit outside any
    // multi-byte sequence.
    let mut literal_start = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            out.push_str(&template[literal_start..i]);
            let end = template[i + 2..].find("}}").ok_or_else(|| {
                ExporterError::BackendRejected(format!(
                    "unterminated template placeholder starting at byte {i}"
                ))
            })?;
            let raw = &template[i + 2..i + 2 + end];
            let key = raw.trim();
            let (real_key, optional) = match key.strip_suffix('?') {
                Some(k) => (k.trim(), true),
                None => (key, false),
            };
            out.push_str(&env.resolve(real_key, optional)?);
            i += 2 + end + 2;
            literal_start = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&template[literal_start..]);
    Ok(out)
}

/// Renders every string leaf of a JSON document in place.
///
/// Numbers, booleans and nulls pass through untouched, so a body
/// template can carry a real `"steps": 20` rather than a string that the
/// backend has to coerce.
pub fn substitute_json_leaves(
    value: &Value,
    env: &TemplateEnv<'_>,
) -> Result<Value, ExporterError> {
    match value {
        Value::String(s) => Ok(Value::String(render(s, env)?)),
        Value::Array(arr) => arr
            .iter()
            .map(|v| substitute_json_leaves(v, env))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(obj) => {
            let mut out = Map::with_capacity(obj.len());
            for (k, v) in obj {
                out.insert(k.clone(), substitute_json_leaves(v, env)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

/// Renders every value of a header map.
pub fn render_headers(
    headers: &std::collections::BTreeMap<String, String>,
    env: &TemplateEnv<'_>,
) -> Result<std::collections::BTreeMap<String, String>, ExporterError> {
    let mut out = std::collections::BTreeMap::new();
    for (k, v) in headers {
        out.insert(k.clone(), render(v, env)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn card(id: &str, locator: &str) -> AssetCardDto {
        AssetCardDto {
            id: id.into(),
            persona_id: "persona-1".into(),
            modality: Some("image".into()),
            mime: Some("image/png".into()),
            media: "image".into(),
            occurred_at_ms: 0,
            cover: None,
            labels: vec![],
            file_size_bytes: None,
            duration_ms: None,
            pixel_count: None,
            source_locator: locator.into(),
            group_ids: vec![],
            primary_group_position: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            rating: None,
            palette: None,
            has_note: false,
            has_thread: false,
            role: "item".into(),
            title: None,
            member_count: 0,
            score: None,
            snippet: None,
            author_kind: None,
            author_subject: None,
            operator_ai: None,
        }
    }

    fn ctx<'a>(inputs: &'a [AssetCardDto], params: &'a Value) -> DispatchContext<'a> {
        DispatchContext {
            inputs,
            selection_id: "sel-1",
            dispatch_id: "disp-1",
            persona_id: "persona-1",
            action: "run",
            params,
        }
    }

    #[test]
    fn ids_inputs_and_params_all_resolve() {
        let inputs = [card("a-1", "/in/one.png")];
        let params = json!({ "extras": { "prompt": "a studio portrait" } });
        let c = ctx(&inputs, &params);
        let env = TemplateEnv::pre_handle(&c, &params);

        assert_eq!(render("{{dispatch_id}}", &env).unwrap(), "disp-1");
        assert_eq!(
            render("{{input[0].source_locator}}", &env).unwrap(),
            "/in/one.png"
        );
        assert_eq!(
            render("{{params.extras.prompt}}", &env).unwrap(),
            "a studio portrait"
        );
    }

    #[test]
    fn an_unresolved_placeholder_is_an_error_unless_it_is_marked_optional() {
        let inputs = [card("a-1", "/in/one.png")];
        let params = json!({});
        let c = ctx(&inputs, &params);
        let env = TemplateEnv::pre_handle(&c, &params);

        assert!(render("{{params.nope}}", &env).is_err());
        assert_eq!(render("{{params.nope?}}", &env).unwrap(), "");
    }

    /// The phase distinction is the point of the three constructors: a
    /// handle placeholder in a submit template is a profile bug, and it
    /// is reported rather than silently emptied.
    #[test]
    fn a_handle_placeholder_does_not_resolve_before_there_is_a_handle() {
        let inputs = [card("a-1", "/in/one.png")];
        let params = json!({});
        let c = ctx(&inputs, &params);

        let pre = TemplateEnv::pre_handle(&c, &params);
        assert!(render("{{handle.job_id}}", &pre).is_err());

        let handle = json!({ "job_id": "j-9" });
        let post = TemplateEnv::with_handle(&c, &params, &handle);
        assert_eq!(render("{{handle.job_id}}", &post).unwrap(), "j-9");
    }

    /// Non-ASCII either side of a placeholder is the case a byte-by-byte
    /// copy corrupts. Pinned because prompts are the main thing these
    /// templates carry, and prompts are where the accents are.
    #[test]
    fn multibyte_text_around_a_placeholder_survives() {
        let inputs = [card("a-1", "/in/one.png")];
        let params = json!({ "who": "夕焼け" });
        let c = ctx(&inputs, &params);
        let env = TemplateEnv::pre_handle(&c, &params);

        assert_eq!(
            render("背景は{{params.who}}、前景は逆光", &env).unwrap(),
            "背景は夕焼け、前景は逆光"
        );
    }

    #[test]
    fn json_leaves_are_rendered_and_non_strings_pass_through() {
        let inputs = [card("a-1", "/in/one.png")];
        let params = json!({ "extras": { "prompt": "hello" } });
        let c = ctx(&inputs, &params);
        let env = TemplateEnv::pre_handle(&c, &params);

        let body = json!({
            "prompt": "{{params.extras.prompt}}",
            "steps": 20,
            "flags": [true, "{{dispatch_id}}"]
        });
        assert_eq!(
            substitute_json_leaves(&body, &env).unwrap(),
            json!({
                "prompt": "hello",
                "steps": 20,
                "flags": [true, "disp-1"]
            })
        );
    }

    #[test]
    fn an_item_environment_still_sees_the_dispatch() {
        let inputs = [card("a-1", "/in/one.png")];
        let params = json!({});
        let c = ctx(&inputs, &params);
        let handle = json!({});
        let env = TemplateEnv::with_handle(&c, &params, &handle);
        let item = json!({ "url": "out.png" });
        let item_env = env.with_item(&item);

        assert_eq!(
            render("{{dispatch_id}}/{{item.url}}", &item_env).unwrap(),
            "disp-1/out.png"
        );
    }

    #[test]
    fn an_unterminated_placeholder_is_rejected() {
        let inputs = [card("a-1", "/in/one.png")];
        let params = json!({});
        let c = ctx(&inputs, &params);
        let env = TemplateEnv::pre_handle(&c, &params);

        assert!(render("{{dispatch_id", &env).is_err());
    }

    /// The offset in that message indexes bytes, and a profile author
    /// counting characters to find the typo would land in the wrong
    /// place on any template carrying non-ASCII text — which prompts do.
    /// `夜` is three bytes, so the unterminated `{{` opens at byte 3.
    #[test]
    fn the_unterminated_offset_is_a_byte_index() {
        let inputs = [card("a-1", "/in/one.png")];
        let params = json!({});
        let c = ctx(&inputs, &params);
        let env = TemplateEnv::pre_handle(&c, &params);

        let msg = render("夜{{a", &env).unwrap_err().to_string();
        assert!(msg.contains("byte 3"), "unexpected message: {msg}");
    }
}
