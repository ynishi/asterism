//! Does this encoder separate one asset's words from another's? (#32)
//!
//! The tree has measured this model twice — image against image (the
//! visual edge floor) and text against image (the tag floor). It has
//! never measured text against text, which is the question #32's layer
//! rests on: an asset's derived words are embedded, a query is
//! embedded, and the two are compared. A model that is excellent at
//! matching a tag name to a picture is not thereby known to separate
//! one caption from another.
//!
//! Same opt-ins as the model eval beside it: the `fixtures` and `onnx`
//! features at compile time, `ASTERISM_TEST_MODEL_DIR` at run time. CI
//! has neither, so this is a no-op there by construction.
//!
//! # Why the fixtures answer this
//!
//! A scene's spec *is* its ground truth, captions and tags derived from
//! the same declaration that drew it. So two texts are known to be
//! about the same picture, or known not to be, without anybody having
//! annotated anything — and the relation stream already draws the two
//! cases that matter here. A semantic sibling carries the same tag set
//! over different geometry, so its words should sit close. A hard
//! negative is drawn so its tags do not overlap at all, so its words
//! should sit far. Whether they do is what this file reports.
#![cfg(all(feature = "fixtures", feature = "onnx"))]

use asterism_vision::encoder::Encoder;
use asterism_vision::fixtures::relations::{RelationStream, unrelated_queries_en};
use asterism_vision::fixtures::scene::SceneSpec;
use asterism_vision::package::ModelPackage;

fn encoder_or_skip() -> Option<Encoder> {
    let dir = match std::env::var("ASTERISM_TEST_MODEL_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => {
            eprintln!("text recall eval skipped: ASTERISM_TEST_MODEL_DIR is unset");
            return None;
        }
    };
    let package = ModelPackage::open(&dir).expect("the named package must open");
    Some(Encoder::load(&package).expect("the package must load"))
}

/// Both towers L2-normalise, so the inner product is the cosine.
fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// What the composer would hand this layer for a generated image: the
/// caption first, then the tags, one per line.
///
/// Deliberately short. `derive_text` composes far more than this for an
/// asset with a body, and whether that survives the encoder's window is
/// the second test's question rather than this one's.
fn words_of(spec: &SceneSpec) -> String {
    let mut out = spec.caption_en();
    for tag in spec.tags_en() {
        out.push('\n');
        out.push_str(&tag);
    }
    out
}

fn report(name: &str, xs: &[f32]) {
    if xs.is_empty() {
        eprintln!("{name}: n=0");
        return;
    }
    let min = xs.iter().copied().fold(f32::INFINITY, f32::min);
    let max = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mean = xs.iter().sum::<f32>() / xs.len() as f32;
    eprintln!(
        "{name}: n={} min={min:.3} max={max:.3} mean={mean:.3}",
        xs.len()
    );
}

/// Where each population lands when text meets text.
///
/// The assertion is the one a threshold needs: the words of pictures
/// that share a subject sit closer than the words of pictures that
/// share nothing. If that fails, no floor separates them and the layer
/// wants a different encoder or a different input — which is a finding,
/// and the reason this runs before anything is indexed.
#[test]
fn text_against_text_separates_by_subject() {
    let Some(mut encoder) = encoder_or_skip() else {
        return;
    };

    let mut bases: Vec<(SceneSpec, Vec<f32>)> = Vec::new();
    let mut siblings: Vec<f32> = Vec::new();
    let mut negatives: Vec<f32> = Vec::new();

    for related in RelationStream::new(42).take(24) {
        let base_words = words_of(&related.scene);
        let base = encoder.encode_text(&base_words).expect("encode base");

        if let Some(sibling) = &related.semantic_sibling {
            let v = encoder.encode_text(&words_of(sibling)).expect("encode");
            siblings.push(similarity(&base, &v));
        }
        if let Some(negative) = &related.hard_negative {
            let v = encoder.encode_text(&words_of(negative)).expect("encode");
            negatives.push(similarity(&base, &v));
        }
        bases.push((related.scene, base));
    }

    // Every base against every other: pictures with nothing declared in
    // common, which is what a library mostly holds.
    let mut strangers: Vec<f32> = Vec::new();
    for (i, (_, a)) in bases.iter().enumerate() {
        for (_, b) in &bases[i + 1..] {
            strangers.push(similarity(a, b));
        }
    }

    // A query about nothing in the set. The honest-failure case: these
    // have to sit below whatever floor the populations above suggest,
    // or a miss returns padding.
    let mut unrelated: Vec<f32> = Vec::new();
    for query in unrelated_queries_en() {
        let q = encoder.encode_text(&query).expect("encode query");
        for (_, v) in &bases {
            unrelated.push(similarity(&q, v));
        }
    }

    report("semantic sibling", &siblings);
    report("hard negative", &negatives);
    report("stranger", &strangers);
    report("unrelated query", &unrelated);

    let closest_sibling = siblings.iter().copied().fold(f32::INFINITY, f32::min);
    let furthest_negative = negatives.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    eprintln!("closest sibling={closest_sibling:.3} furthest negative={furthest_negative:.3}");

    assert!(
        closest_sibling > furthest_negative,
        "the words of two pictures sharing a subject sat {closest_sibling:.3} apart \
         while two sharing none sat {furthest_negative:.3} — no floor separates them, \
         so this encoder does not answer the question this layer asks of it"
    );
}

/// Recall from a query, which is the shape the layer actually serves.
///
/// The two tests above compare one asset's words with another's. What a
/// person does is type words that were never written down anywhere and
/// expect the right pictures back — so the query side has to be
/// measured on its own, and the number that matters is not a similarity
/// but a count of misses.
///
/// The answer set is known without anybody annotating it: a scene's
/// spec declares its tags, so "every asset whose spec says *a red
/// circle*" is the ground truth for the query `a red circle`. That is
/// weaker than the twenty real queries #32 asks for — a tag is a phrase
/// the words already contain, and a person's remembered phrase is not —
/// so read this as the floor of what the layer can do rather than as
/// what it will do.
#[test]
fn a_query_reaches_the_assets_whose_words_answer_it() {
    let Some(mut encoder) = encoder_or_skip() else {
        return;
    };

    let scenes: Vec<SceneSpec> = RelationStream::new(42)
        .take(24)
        .map(|related| related.scene)
        .collect();
    let mut library: Vec<(Vec<String>, Vec<f32>)> = Vec::new();
    for spec in &scenes {
        let vector = encoder.encode_text(&words_of(spec)).expect("encode asset");
        library.push((spec.tags_en(), vector));
    }

    let mut queries: Vec<String> = library
        .iter()
        .flat_map(|(tags, _)| tags.iter().cloned())
        .collect();
    queries.sort();
    queries.dedup();

    // Scored once, read at several thresholds: the sweep is the shape
    // a floor is chosen from, and choosing one is not this test's job.
    let mut scored: Vec<(bool, f32)> = Vec::new();
    let mut top5_hits = 0usize;
    let mut top5_possible = 0usize;
    for query in &queries {
        let q = encoder.encode_text(query).expect("encode query");
        let mut ranked: Vec<(bool, f32)> = library
            .iter()
            .map(|(tags, v)| (tags.contains(query), similarity(&q, v)))
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));

        let answers = ranked.iter().filter(|(is_answer, _)| *is_answer).count();
        top5_possible += answers.min(5);
        top5_hits += ranked
            .iter()
            .take(5)
            .filter(|(is_answer, _)| *is_answer)
            .count();
        scored.extend(ranked);
    }

    eprintln!(
        "queries={} library={} answers-in-top-5={top5_hits}/{top5_possible}",
        queries.len(),
        library.len()
    );
    let mut best: Option<(f32, f32, f32)> = None;
    for step in 0..=10u8 {
        let threshold = 0.70 + f32::from(step) * 0.03;
        let tp = scored.iter().filter(|(a, s)| *a && *s >= threshold).count() as f32;
        let fp = scored
            .iter()
            .filter(|(a, s)| !*a && *s >= threshold)
            .count() as f32;
        let miss = scored.iter().filter(|(a, s)| *a && *s < threshold).count() as f32;
        let precision = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
        let recall = if tp + miss > 0.0 {
            tp / (tp + miss)
        } else {
            0.0
        };
        eprintln!(
            "threshold {threshold:.2}: precision {precision:.3} recall {recall:.3} \
             (tp {tp:.0} fp {fp:.0} missed {miss:.0})"
        );
        if precision >= 0.5 && best.is_none_or(|(_, _, r)| recall > r) {
            best = Some((threshold, precision, recall));
        }
    }

    match best {
        Some((t, p, r)) => eprintln!(
            "widest floor holding precision at 0.5: {t:.2} — precision {p:.3} recall {r:.3}"
        ),
        None => eprintln!("no threshold held precision at 0.5"),
    }

    // The floor of usefulness: a query has to reach at least one of its
    // own answers before anything else is worth tuning.
    assert!(
        top5_hits > 0,
        "no query reached any of its answers in the top five, so ranking by \
         this similarity does not order the library by what a query asks for"
    );
}

/// Where a long body stops being read.
///
/// `encode_text` truncates to a fixed window. A tag name never reaches
/// it; a composed document — body, title, cover, labels, keywords,
/// embedded metadata, comments — passes it early, and everything after
/// the cut is invisible to the vector. Two documents that agree up to
/// the window and diverge after it encode identically, and a recall
/// layer built on that would answer the same for both.
///
/// This does not assert a limit. It reports where the cut falls for
/// this package, which is the number that decides whether the layer
/// chunks a document or takes a shorter input.
#[test]
fn the_window_is_where_a_long_body_stops_being_read() {
    let Some(mut encoder) = encoder_or_skip() else {
        return;
    };

    let scenes: Vec<SceneSpec> = RelationStream::new(7)
        .take(8)
        .map(|related| related.scene)
        .collect();
    let head = words_of(&scenes[0]);

    // Same opening, different continuations, growing. The point at
    // which the two stop differing is the window.
    let mut prefix = head.clone();
    let mut first_identical: Option<usize> = None;
    for (step, scene) in scenes[1..].iter().enumerate() {
        prefix.push('\n');
        prefix.push_str(&words_of(scene));

        let a = encoder
            .encode_text(&format!("{prefix}\nthe ending is a red circle"))
            .expect("encode");
        let b = encoder
            .encode_text(&format!("{prefix}\nthe ending is a blue square"))
            .expect("encode");
        let sim = similarity(&a, &b);
        eprintln!(
            "after {} scenes ({} chars): endings differ by {:.4}",
            step + 2,
            prefix.len(),
            1.0 - sim
        );
        if sim > 0.999_9 && first_identical.is_none() {
            first_identical = Some(prefix.len());
        }
    }

    match first_identical {
        Some(chars) => eprintln!(
            "the window closes near {chars} characters: past it, two documents \
             differing only at the end encode the same"
        ),
        None => eprintln!(
            "the window was not reached within this fixture's length; a real \
             body is longer than anything built here"
        ),
    }
}
