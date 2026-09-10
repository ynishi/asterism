//! The C2PA manifest *definition* — what would be signed, built as a
//! value, with nothing here able to sign it.
//!
//! `c2pa::Builder::with_definition` takes a JSON document describing the
//! manifest; producing that document is a mapping problem (what the
//! database holds → what the manifest asserts) and producing a
//! *signature* is a key-management problem. They are separated here so
//! the mapping can be tested exhaustively on a machine with no
//! certificate, which is every machine this repository has today.
//!
//! # Two assertions, and why the second one exists
//!
//! `c2pa.actions` carries the standard claim: this asset was created,
//! and its `digitalSourceType` is the same IPTC URI the XMP packet
//! states. That is the half a validator understands.
//!
//! `io.github.ynishi.asterism.disclosure` carries what the database
//! knows and the standard has no field for: the asset id, the dispatch
//! the file left through, and the ids it was derived from. A reader that
//! has this Asterism instance can resolve those; a reader that does not
//! at least learns that the lineage exists and is recorded somewhere.
//!
//! The label is reverse-DNS under a domain that resolves to the author,
//! which is the convention the C2PA specification asks third-party
//! assertions to follow. A label invented outside a controlled namespace
//! is one that can collide with somebody else's meaning of the same
//! words.
//!
//! # Why the parents are ids and not ingredients
//!
//! C2PA has a first-class way to say "this was made from that": an
//! ingredient, which carries the parent's own hash and, where it has
//! one, its manifest. That is a stronger statement than an id, and it is
//! one this path cannot honestly make — signing happens over a file that
//! has been exported, and the parents' bytes are not in hand at that
//! moment (they may have been purged, and re-reading them would make an
//! export's cost depend on the depth of its lineage). An ingredient
//! constructed without the parent's bytes would be an assertion about a
//! hash nobody computed.
//!
//! So the ids go in the custom assertion, where they are what they are:
//! a pointer into the library that recorded the edge. Promoting them to
//! ingredients is a later change that needs the parent files, not a
//! detail left out.

use serde_json::{Value, json};

use asterism_core::domain::disclosure::{DisclosureRecord, ReleaseAct, ReleaseDisclosure};

/// Label of the standard actions assertion.
///
/// The base label rather than `c2pa.actions.v2`: the SDK picks the
/// versioned spelling from the claim version it is building, and
/// pinning the versioned one here would make this definition wrong the
/// moment the builder targets a different claim version.
pub const ACTIONS_LABEL: &str = "c2pa.actions";

/// The action a generated file's manifest states.
const ACTION_CREATED: &str = "c2pa.created";

/// Label of Asterism's own assertion (module docs on the namespace).
///
/// # Why this could still be renamed, and only now
///
/// A signed assertion label is the one identifier in this feature that
/// cannot be corrected later: it sits inside a document whose whole
/// point is that it is tamper-evident, in files that have left the
/// machine, and a reader keys on it. That argument is what nearly kept
/// the older spelling — `…asterism.provenance`, from before this
/// feature and the derived-from claim graph were told apart.
///
/// It does not apply yet. No build has ever signed anything: the
/// composition root constructs the writer `unsigned()`, and the one
/// constructor that takes a certificate has no caller and no
/// configuration surface. Every manifest that has ever carried this
/// label was produced in a test, by a throwaway certificate, into a
/// temporary directory. There is no file in the world to be
/// compatible with — so the cost of the rename is zero today and
/// permanent from the first signed export.
pub const ASTERISM_LABEL: &str = "io.github.ynishi.asterism.disclosure";

/// Version of the payload under [`ASTERISM_LABEL`].
///
/// Beside the payload rather than implied by the label, on the same
/// terms the export sidecar carries `asterism.sidecar/1`: a file signed
/// today can be read years later by a build that has moved on, and the
/// reader needs to know which shape it is holding before it starts
/// walking it.
///
/// Still `/1` after the rename: a version distinguishes shapes a reader
/// may hold, and nothing has ever read shape 1 under the old name.
pub const ASTERISM_ASSERTION_SCHEMA: &str = "asterism.disclosure/1";

/// Renders the manifest definition for one record.
///
/// The result is a `serde_json::Value` ready for
/// `c2pa::Builder::with_definition`. It always contains the Asterism
/// assertion (the asset id alone is worth recording) and contains the
/// actions assertion only when a digital source type was established —
/// an action that names no source type asserts less than nothing, since
/// a validator reads the absence as "created, provenance unstated".
pub fn definition(record: &DisclosureRecord) -> Value {
    let mut assertions = Vec::with_capacity(2);

    if let Some(source_type) = record.source_type {
        assertions.push(json!({
            "label": ACTIONS_LABEL,
            "data": {
                "actions": [{
                    "action": ACTION_CREATED,
                    "digitalSourceType": source_type.uri(),
                }]
            }
        }));
    }

    // Only the fields that were established. A key present with a null
    // value says "asked and unknown", which is not what an absent edge
    // or an unrecorded dispatch means — the same distinction the
    // `_trace` note keeps on the ingest side.
    let mut asterism = serde_json::Map::new();
    asterism.insert("schema".into(), json!(ASTERISM_ASSERTION_SCHEMA));
    asterism.insert("asset_id".into(), json!(record.asset_id));
    if let Some(dispatch_id) = &record.dispatch_id {
        asterism.insert("dispatch_id".into(), json!(dispatch_id));
    }
    if !record.parents.is_empty() {
        asterism.insert("derived_from".into(), json!(record.parents));
    }
    if let Some(system) = &record.ai_system {
        asterism.insert("ai_system".into(), json!(system));
    }
    if let Some(version) = &record.ai_system_version {
        asterism.insert("ai_system_version".into(), json!(version));
    }
    // The extracted generator parameters, verbatim as the container
    // stated them — the seed as its literal text rather than a JSON
    // number, so no serialiser's habits enter a signed claim. Optional
    // fields added to shape 1 rather than a shape 2: a reader keyed on
    // the schema walks what is present, and nothing has ever read
    // shape 1 without them.
    if let Some(model) = &record.model {
        asterism.insert("model".into(), json!(model));
    }
    if let Some(seed) = &record.seed {
        asterism.insert("seed".into(), json!(seed));
    }
    // The work that chose this file, when it left through a release.
    // Another optional field of shape 1, on the same terms as the two
    // above: a reader keyed on the schema walks what is present, and a
    // file that did not leave through a release carries no key rather
    // than an empty one.
    if let Some(release) = &record.release {
        asterism.insert("release".into(), released(release));
    }
    assertions.push(json!({ "label": ASTERISM_LABEL, "data": Value::Object(asterism) }));

    let mut definition = serde_json::Map::new();
    if let Some(title) = &record.title {
        definition.insert("title".into(), json!(title));
    }
    // Name and version. The specification requires `name` and makes
    // `version` optional, and the optional half was withheld for as long
    // as every crate here inherited the same `0.0.0`: a number that never
    // moves puts a claim into a signed, uncorrectable document that every
    // build ever made also carries, which tells a reader nothing and
    // tells them it confidently. The note that stood here said the field
    // comes back when releases start carrying a number that distinguishes
    // them, and v0.1.0 is that number.
    //
    // `xmp::render` still declines the same claim in `x:xmptk`. It had
    // two reasons that happened to agree; this bump retires the first and
    // leaves the second, which is not about version strings at all — that
    // packet may be a function of the record and of nothing else.
    definition.insert(
        "claim_generator_info".into(),
        json!([{ "name": "asterism", "version": env!("CARGO_PKG_VERSION") }]),
    );
    definition.insert("assertions".into(), json!(assertions));
    Value::Object(definition)
}

/// The release block of the Asterism assertion.
///
/// Two acts and the shape of the work between them. Each act is
/// `{ "at": <RFC 3339>, "by": "person" | "rule" }` — the pair the forge
/// records on every node, and the pair that answers the question this
/// whole feature exists for: a rule wrote this one is a different
/// answer from a person chose this one.
///
/// The instant is RFC 3339 in the timezone the value carries, which is
/// always UTC, and never an epoch integer: this is a document a person
/// may end up reading in a dispute, and a number nobody can read
/// without a converter is a worse record than a longer string.
///
/// `intent_title` is absent when the work was never given a name,
/// rather than empty — the same distinction the fields above it keep.
fn released(release: &ReleaseDisclosure) -> Value {
    let mut block = serde_json::Map::new();
    block.insert("released".into(), act(&release.released));
    block.insert("pursuit_id".into(), json!(release.pursuit_id));
    if let Some(title) = &release.intent_title {
        block.insert("intent_title".into(), json!(title));
    }
    block.insert("rounds".into(), json!(release.rounds));
    block.insert("closed".into(), act(&release.closed));
    Value::Object(block)
}

/// One act, as the assertion states it.
fn act(act: &ReleaseAct) -> Value {
    json!({ "at": act.at.to_rfc3339(), "by": act.by.as_str() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::disclosure::{DigitalSourceType, Hand};

    /// A fixed instant, so what the assertion states can be pinned
    /// exactly rather than matched with a pattern.
    fn at(minute: u32) -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc
            .with_ymd_and_hms(2026, 9, 11, 12, minute, 0)
            .unwrap()
    }

    fn released_shape(title: Option<&str>) -> ReleaseDisclosure {
        ReleaseDisclosure::new(
            ReleaseAct::new(at(30), Hand::Person),
            "pursuit-1",
            title.map(str::to_string),
            3,
            ReleaseAct::new(at(10), Hand::Rule),
        )
    }

    fn assertion<'a>(definition: &'a Value, label: &str) -> Option<&'a Value> {
        definition["assertions"]
            .as_array()?
            .iter()
            .find(|a| a["label"] == label)
    }

    #[test]
    fn the_actions_assertion_states_the_same_uri_the_packet_does() {
        // The two emitters disagreeing is the failure this whole crate
        // is shaped to prevent: a file whose XMP says one thing and
        // whose signed manifest says another is worse than a file that
        // says neither, because one of the two is provably false.
        let record = DisclosureRecord::for_asset("asset-1")
            .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia);
        let definition = definition(&record);
        let actions = assertion(&definition, ACTIONS_LABEL).expect("actions assertion");
        assert_eq!(actions["data"]["actions"][0]["action"], ACTION_CREATED);
        assert_eq!(
            actions["data"]["actions"][0]["digitalSourceType"],
            DigitalSourceType::TrainedAlgorithmicMedia.uri()
        );
        let packet = crate::xmp::render(&record).unwrap();
        assert!(packet.contains(DigitalSourceType::TrainedAlgorithmicMedia.uri()));
    }

    #[test]
    fn no_source_type_means_no_actions_assertion() {
        // "Created, provenance unstated" is a claim, and not one this
        // path is entitled to make on a file nothing established.
        let definition = definition(&DisclosureRecord::for_asset("asset-1"));
        assert!(assertion(&definition, ACTIONS_LABEL).is_none());
        assert!(
            assertion(&definition, ASTERISM_LABEL).is_some(),
            "the asset id is still worth recording"
        );
    }

    #[test]
    fn the_asterism_assertion_is_versioned_and_names_the_lineage() {
        let record = DisclosureRecord::for_asset("asset-1")
            .with_dispatch("dispatch-1")
            .with_parents(vec!["parent-1".into(), "parent-2".into()])
            .with_ai_system("ComfyUI", Some("0.3.0".into()));
        let definition = definition(&record);
        let data = &assertion(&definition, ASTERISM_LABEL).unwrap()["data"];
        assert_eq!(data["schema"], ASTERISM_ASSERTION_SCHEMA);
        assert_eq!(data["asset_id"], "asset-1");
        assert_eq!(data["dispatch_id"], "dispatch-1");
        assert_eq!(data["derived_from"], json!(["parent-1", "parent-2"]));
        assert_eq!(data["ai_system"], "ComfyUI");
        assert_eq!(data["ai_system_version"], "0.3.0");
    }

    #[test]
    fn unestablished_fields_are_absent_rather_than_null() {
        // A null reads as "asked and unknown". An asset with no parents
        // is not an asset whose parents are unknown — and an asset
        // whose seed was refused or never extracted is not one whose
        // seed is null.
        let definition = definition(&DisclosureRecord::for_asset("asset-1"));
        let data = assertion(&definition, ASTERISM_LABEL).unwrap()["data"]
            .as_object()
            .unwrap()
            .clone();
        assert!(!data.contains_key("derived_from"));
        assert!(!data.contains_key("dispatch_id"));
        assert!(!data.contains_key("ai_system"));
        assert!(!data.contains_key("model"));
        assert!(!data.contains_key("seed"));
        assert!(!data.contains_key("release"));
    }

    #[test]
    fn a_released_file_names_the_work_that_chose_it() {
        let record =
            DisclosureRecord::for_asset("asset-1").with_release(released_shape(Some("the crop")));
        let definition = definition(&record);
        let release = &assertion(&definition, ASTERISM_LABEL).unwrap()["data"]["release"];

        assert_eq!(release["released"]["at"], at(30).to_rfc3339());
        assert_eq!(release["released"]["by"], "person");
        assert_eq!(release["pursuit_id"], "pursuit-1");
        assert_eq!(release["intent_title"], "the crop");
        assert_eq!(release["rounds"], 3);
        assert_eq!(release["closed"]["at"], at(10).to_rfc3339());
        assert_eq!(release["closed"]["by"], "rule");
    }

    /// Work nobody named is work with no title, not work whose title is
    /// empty — the same reading the fields beside it get.
    #[test]
    fn an_unnamed_pursuit_contributes_no_title() {
        let record = DisclosureRecord::for_asset("asset-1").with_release(released_shape(None));
        let definition = definition(&record);
        let release = assertion(&definition, ASTERISM_LABEL).unwrap()["data"]["release"]
            .as_object()
            .unwrap()
            .clone();

        assert!(!release.contains_key("intent_title"));
        assert!(release.contains_key("pursuit_id"));
    }

    /// The deliberation stays home. A round note is free text somebody
    /// wrote for themselves and an operation is what was tried rather
    /// than what was chosen — neither belongs in a document that cannot
    /// be corrected once it has left.
    #[test]
    fn the_deliberation_does_not_travel_with_the_release() {
        let record = DisclosureRecord::for_asset("asset-1")
            .with_prompt("1girl")
            .with_release(released_shape(Some("the crop")));
        let rendered = definition(&record).to_string();

        assert!(!rendered.contains("1girl"), "the prompt is packet-only");
        // The count is what travels; nothing names a round or what one
        // did, so no operation vocabulary can appear.
        for word in ["note", "add", "replace", "rename", "remove", "ops"] {
            assert!(
                !rendered.contains(&format!("\"{word}\"")),
                "the assertion states {word:?}, which is deliberation"
            );
        }
        assert!(rendered.contains("\"rounds\":3"));
    }

    #[test]
    fn the_extracted_params_reach_the_assertion_as_the_container_stated_them() {
        // The seed stays the literal text the container carried — a
        // string in the JSON, not a number a serialiser re-rendered.
        let record = DisclosureRecord::for_asset("asset-1")
            .with_model("cetus-mix_v4.safetensors")
            .with_seed("620206974400");
        let definition = definition(&record);
        let data = &assertion(&definition, ASTERISM_LABEL).unwrap()["data"];
        assert_eq!(data["model"], "cetus-mix_v4.safetensors");
        assert_eq!(data["seed"], "620206974400");
        assert!(data["seed"].is_string(), "verbatim text, not a number");
    }

    #[test]
    fn the_prompt_does_not_travel_into_the_manifest() {
        // It is disclosed once, in the packet, under the IPTC property
        // defined for it. Writing it twice would make a file where the
        // two copies can be edited apart — and the signed copy is the
        // one nobody can correct afterwards.
        let record = DisclosureRecord::for_asset("asset-1").with_prompt("1girl");
        let rendered = definition(&record).to_string();
        assert!(!rendered.contains("1girl"));
    }

    /// The generator names itself and the release it was built from.
    ///
    /// Deliberately not `assert_eq!(info["version"], env!("CARGO_PKG_VERSION"))`,
    /// which is what stood here before the field was withdrawn: that is
    /// the same expression the code under test uses, so it holds at any
    /// value — including the `0.0.0` this workspace carried until v0.1.0,
    /// which is the one value the field exists to stop being. What is
    /// pinned instead is what reaches the signature: a release number,
    /// and not the placeholder.
    #[test]
    fn the_generator_names_itself_and_the_release_it_came_from() {
        let definition = definition(&DisclosureRecord::for_asset("asset-1"));
        let info = definition["claim_generator_info"][0]
            .as_object()
            .expect("claim_generator_info holds one generator map");
        assert_eq!(info["name"], "asterism");
        let version = info["version"].as_str().expect("version is a string");
        assert_ne!(
            version, "0.0.0",
            "the placeholder is not a release, and a signed document \
             carrying it says nothing confidently"
        );
        assert!(
            version
                .split('.')
                .next()
                .is_some_and(|major| major.parse::<u32>().is_ok()),
            "a release number, not a build label: {version}"
        );
    }
}
