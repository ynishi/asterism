//! The model-gated evaluation harness (#112): fixtures × a real
//! package.
//!
//! Runs only when both opt-ins are present — the `fixtures` and `onnx`
//! features at compile time, and `ASTERISM_TEST_MODEL_DIR` (a package
//! directory) at run time. CI has neither, so the suite is a no-op
//! there by construction.
//!
//! The measurement itself lives in `fixtures::eval`, shared with the
//! provider-side qualification tool so the two cannot drift; this
//! harness owns only the *assertions* — orderings that must hold on
//! any model worth binding — and prints the measured JSON line for
//! the record. Aggregate assertions, not per pair: an encoder is
//! allowed an off day on one scene, not on the population. JA is
//! reported, not asserted: a measurement that fails an assertion would
//! hide the number the run exists to produce.
#![cfg(all(feature = "fixtures", feature = "onnx"))]

use asterism_vision::encoder::Encoder;
use asterism_vision::fixtures::eval::{EvalConfig, run};
use asterism_vision::fixtures::scene;
use asterism_vision::package::ModelPackage;

fn encoder_or_skip() -> Option<Encoder> {
    let dir = match std::env::var("ASTERISM_TEST_MODEL_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => {
            eprintln!("model eval skipped: ASTERISM_TEST_MODEL_DIR is unset");
            return None;
        }
    };
    let package = ModelPackage::open(&dir).expect("the named package must open");
    Some(Encoder::load(&package).expect("the package must load"))
}

#[test]
fn orderings_hold_and_measurements_print() {
    let Some(mut encoder) = encoder_or_skip() else {
        return;
    };
    let outcome = run(&mut encoder, &EvalConfig::default()).expect("measurement");
    println!("{}", outcome.to_json(encoder.model_id()));

    let (beats, pairs) = outcome.lookalike_beats_negative;
    assert!(
        beats * 10 >= pairs * 8,
        "look-alikes must beat hard negatives on at least 80% of pairs ({beats}/{pairs})"
    );
    assert!(
        outcome.tag.own_beats_disjoint * 10 >= outcome.tag.pairs * 8,
        "own EN tags must beat disjoint tags on at least 80% of pairs ({}/{})",
        outcome.tag.own_beats_disjoint,
        outcome.tag.pairs
    );
    assert!(
        outcome.noise.0 < outcome.lookalike_mean,
        "noise must sit below the scene family"
    );
}

/// The dim the manifest declares is the dim the towers produce — the
/// P0 record left 768 as inferred-from-defaults, and this is where the
/// inference is replaced by an assertion against the real model.
#[test]
fn declared_dim_is_asserted_against_the_model() {
    let Some(mut encoder) = encoder_or_skip() else {
        return;
    };
    let img = scene::noise_image(1, 64, 64);
    let v = encoder
        .encode_image(img.as_raw(), 64, 64)
        .expect("encode succeeds only when the dims agree");
    assert_eq!(v.len(), encoder.dim() as usize);
    let t = encoder.encode_text("a red circle").expect("text tower");
    assert_eq!(t.len(), encoder.dim() as usize);
}
