//! What the platform said, and — when it said nothing — why.
//!
//! A record with no seed in it does not say why on its own, and there
//! are three different reasons behind that one absence. A field carries
//! one of four states, and the first of them is the field being there:
//!
//! - **captured** — the profile said where to look and the value was
//!   there. Not one of the three; it is what they are the absence of.
//! - **not captured** — the profile said where to look and nothing was
//!   there. This is the only one that is a gap on our side, and the only
//!   one worth acting on.
//! - **not returned** / **not supported** — the platform ran with a seed
//!   and does not tell you which, or the parameter does not exist on
//!   this model at all. Both are properties of the platform, fixed
//!   before any call is made, and the profile author is the person who
//!   read the documentation that says so.
//!
//! A `null` reports all three absences identically, which is why the
//! status sits beside the value rather than as a marker written into
//! the value's own slot. [#17] argues the same from `getxattr(2)` (which
//! separates "the filesystem does not support this" from "the attribute
//! is not there" from a value) against `statx(2)` (which collapses them
//! and fills in a plausible-looking dummy).
//!
//! [#17]: https://github.com/ynishi/asterism/issues/17
//!
//! # Why the profile is the place to say it
//!
//! Captured and not-captured are decided per artefact, out of what the
//! response held. The other two are not: which of them applies is fixed
//! per platform, before any call. This adapter already treats a
//! platform as a profile rather than as an adapter of its own — the
//! crate doc argues that and lists what a profile may declare — so a
//! declared absence is one more thing the profile author knows from
//! reading the platform's documentation, stated once, in the same
//! place as the rest.
//!
//! Per profile and not per action, because the declaration travels in
//! the params blob and that blob is written per dispatch. A family
//! where one model reports a seed and another does not is two profiles
//! — which is what the endpoint, the paths and the deadline beside it
//! already are.
//!
//! # Declaring one is not the same as failing to capture it
//!
//! A profile may not both name a path for a field and declare it absent.
//! That is a profile contradicting itself — it says the platform does
//! not return a value and then says where the value is — and it is
//! refused when the params are parsed, on every phase, so the dispatch
//! fails before the backend is touched.
//!
//! [`RecordSchema::evaluate`] does write absences after paths and would
//! overwrite one with the other, which is a rule of sorts. It is not
//! the one this design rests on: nothing in the shipped path reaches
//! that function with a field in both, because the refusal comes first.
//! Ranking them instead of refusing would let the contradiction survive
//! into the record it was meant to describe.
//!
//! # Where a path points
//!
//! Paths are evaluated against a document assembled per artefact:
//!
//! ```text
//! { "response": <the harvest response>,
//!   "item":     <this artefact's element of it>,
//!   "params":   <the dispatch params> }
//! ```
//!
//! One document because the fields a record wants do not all come from
//! one place. A platform may return the seed once for the whole response
//! and the URL per image; another generating a batch with one seed each
//! returns it per item. `$.response.seed` and `$.item.seed` are both
//! sayable, in the grammar the profile already uses for `items_path`,
//! and neither needs a root that only exists here.

use std::collections::BTreeMap;

use asterism_dispatch_sdk::ExporterError;
use asterism_exporter_common::ResponsePath;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::secret::SecretGrammar;

/// Why a value is not in the record, as declared by the profile.
///
/// Both are statements about the platform rather than about this call,
/// which is why they are declared once in the profile instead of being
/// discovered per response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Absence {
    /// The platform used a value and does not report which.
    NotReturned,
    /// The parameter does not exist on this model.
    NotSupported,
}

/// One field of the record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FieldRecord {
    /// The profile's path resolved.
    Captured {
        /// What it resolved to, with the credential scrubbed as
        /// everywhere else.
        value: Value,
    },
    /// The profile's path resolved to nothing, and the profile did not
    /// say the platform withholds it. A gap on our side: either the path
    /// is wrong or the platform changed its response.
    NotCaptured,
    /// The platform withholds it, as declared.
    NotReturned,
    /// The parameter does not exist, as declared.
    NotSupported,
}

impl From<Absence> for FieldRecord {
    fn from(absence: Absence) -> Self {
        match absence {
            Absence::NotReturned => Self::NotReturned,
            Absence::NotSupported => Self::NotSupported,
        }
    }
}

/// What the profile says the record should contain.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecordSchema {
    /// Field name → path into the per-artefact document.
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
    /// Field name → why the platform does not supply it.
    #[serde(default)]
    pub absences: BTreeMap<String, Absence>,
}

impl RecordSchema {
    /// Rejects a profile that both names a path for a field and declares
    /// it absent.
    ///
    /// Called while parsing the params, so the dispatch fails before the
    /// backend is touched — a contradiction in a profile is an authoring
    /// mistake, and the useful moment to hear about it is the one where
    /// nothing has happened yet.
    pub fn validate(&self) -> Result<(), ExporterError> {
        let mut both: Vec<&str> = self
            .paths
            .keys()
            .filter(|field| self.absences.contains_key(*field))
            .map(String::as_str)
            .collect();
        both.sort_unstable();
        if both.is_empty() {
            return Ok(());
        }
        Err(ExporterError::BackendRejected(format!(
            "record declares {} both as a path and as an absence — a field \
             the platform does not supply has nowhere to be read from",
            both.join(", ")
        )))
    }

    /// Builds the record for one artefact.
    pub fn evaluate(&self, grammar: &SecretGrammar, document: &Value) -> Record {
        let mut out: Record = BTreeMap::new();
        for (field, path) in &self.paths {
            let found = grammar
                .select_first(document, path)
                .filter(|value| !value.is_null());
            out.insert(
                field.clone(),
                match found {
                    Some(value) => FieldRecord::Captured {
                        value: grammar.scrub(value),
                    },
                    None => FieldRecord::NotCaptured,
                },
            );
        }
        for (field, absence) in &self.absences {
            out.insert(field.clone(), (*absence).into());
        }
        out
    }

    /// True when the profile says nothing at all, which is the state
    /// every profile written before this existed is in.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.absences.is_empty()
    }
}

/// One artefact's record, field by field.
pub type Record = BTreeMap<String, FieldRecord>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> RecordSchema {
        serde_json::from_value(json!({
            "paths": {
                "seed": "$.response.seed",
                "prompt": "$.response.prompt",
                "url": "$.item.url",
                "model": "$.params.extras.model"
            },
            "absences": {
                "guidance_scale": "not_returned",
                "sampler": "not_supported"
            }
        }))
        .unwrap()
    }

    fn document() -> Value {
        json!({
            "response": { "seed": 12345, "prompt": "a studio portrait" },
            "item": { "url": "https://cdn.test/a.png" },
            "params": { "extras": { "model": "flux/dev" } }
        })
    }

    #[test]
    fn a_path_reaches_the_response_the_item_and_the_params() {
        let record = schema().evaluate(&SecretGrammar::new("k-123".into()), &document());

        assert_eq!(
            record["seed"],
            FieldRecord::Captured {
                value: json!(12345)
            },
            "a number stays a number"
        );
        assert_eq!(
            record["prompt"],
            FieldRecord::Captured {
                value: json!("a studio portrait")
            }
        );
        assert_eq!(
            record["url"],
            FieldRecord::Captured {
                value: json!("https://cdn.test/a.png")
            }
        );
        assert_eq!(
            record["model"],
            FieldRecord::Captured {
                value: json!("flux/dev")
            }
        );
    }

    /// The distinction the whole module exists for: three absences that
    /// a `null` would report identically.
    #[test]
    fn the_three_absences_are_told_apart() {
        let mut document = document();
        document["response"]["seed"] = Value::Null;

        let record = schema().evaluate(&SecretGrammar::new("k-123".into()), &document);

        assert_eq!(
            record["seed"],
            FieldRecord::NotCaptured,
            "the profile said where to look and nothing was there"
        );
        assert_eq!(record["guidance_scale"], FieldRecord::NotReturned);
        assert_eq!(record["sampler"], FieldRecord::NotSupported);
    }

    /// A missing key and an explicit null are the same statement from a
    /// platform, and reading them differently would make the gap count
    /// depend on which shape a backend happened to send.
    #[test]
    fn an_absent_key_and_an_explicit_null_are_the_same_gap() {
        let mut with_null = document();
        with_null["response"]["seed"] = Value::Null;
        let mut without_key = document();
        without_key["response"]
            .as_object_mut()
            .unwrap()
            .remove("seed");

        let grammar = SecretGrammar::new("k-123".into());
        assert_eq!(
            schema().evaluate(&grammar, &with_null)["seed"],
            schema().evaluate(&grammar, &without_key)["seed"]
        );
    }

    #[test]
    fn a_profile_cannot_both_name_a_path_and_declare_the_field_absent() {
        let contradictory: RecordSchema = serde_json::from_value(json!({
            "paths": { "seed": "$.response.seed", "steps": "$.response.steps" },
            "absences": { "seed": "not_returned", "steps": "not_supported" }
        }))
        .unwrap();

        let message = contradictory.validate().unwrap_err().to_string();
        assert!(message.contains("seed"), "{message}");
        assert!(message.contains("steps"), "{message}");

        schema().validate().expect("the fixture is consistent");
    }

    /// A captured value is a string the record persists, so it is on the
    /// same footing as every other one.
    #[test]
    fn a_captured_value_is_scrubbed() {
        let schema: RecordSchema =
            serde_json::from_value(json!({ "paths": { "echo": "$.response.echo" } })).unwrap();
        let document = json!({ "response": { "echo": "you sent k-123" } });

        assert_eq!(
            schema.evaluate(&SecretGrammar::new("k-123".into()), &document)["echo"],
            FieldRecord::Captured {
                value: json!(format!("you sent {}", crate::REDACTED))
            }
        );
    }

    /// The serialised shape is what a reader months later actually sees.
    #[test]
    fn the_serialised_shape_carries_the_status_beside_the_value() {
        let record = schema().evaluate(&SecretGrammar::new("k-123".into()), &document());
        let json = serde_json::to_value(&record).unwrap();

        assert_eq!(
            json["seed"],
            json!({ "status": "captured", "value": 12345 })
        );
        assert_eq!(json["guidance_scale"], json!({ "status": "not_returned" }));
        assert_eq!(json["sampler"], json!({ "status": "not_supported" }));
    }

    /// Every profile written before this module existed has no record
    /// block, and must keep working.
    #[test]
    fn a_profile_that_says_nothing_records_nothing() {
        let empty = RecordSchema::default();
        assert!(empty.is_empty());
        assert!(
            empty
                .evaluate(&SecretGrammar::new("k".into()), &document())
                .is_empty()
        );
        empty.validate().unwrap();
    }
}
