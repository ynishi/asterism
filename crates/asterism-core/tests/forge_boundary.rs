//! What the forge is allowed to name outside itself.
//!
//! The forge is meant to be liftable: a crate of its own, depending on
//! a small shared vocabulary and on nothing else in this one. Today it
//! is a module, and a module holds nothing — `use crate::domain::asset`
//! in a forge file compiles, and the only thing between here and there
//! is somebody noticing.
//!
//! So this is the thing that notices. It reads every line of forge
//! code that is not a test, collects what it names outside the forge,
//! and refuses anything not on the list below with a reason beside it.
//!
//! # This is the list, and the `SHARED VOCABULARY` comments are not
//!
//! That comment marks the same edges at the point of use — a word
//! neither side owns, which a contract may therefore be written in —
//! and it is worth reading where it sits. It cannot be the authority:
//! an import written without one is invisible to a grep, which is
//! exactly the case that matters. The list here is checked whether
//! anybody remembered or not.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The words a contract across this boundary may be written in, and
/// why each one belongs to neither side.
///
/// Adding to this list is a decision about what the forge crate would
/// have to depend on when it is lifted out. It is not a place to
/// record that something compiles.
const SHARED_VOCABULARY: &[(&str, &str)] = &[
    (
        "crate::error::DomainError",
        "the shared failure. A port is where failures from outside \
         arrive, and a refusal named in one side's private vocabulary \
         is one the other side cannot match on",
    ),
    (
        "crate::domain::value::AssetId",
        "what a reference *is* on both sides of the line. `Content` \
         wraps it so that nothing else in the model has to name it",
    ),
    // `crate::domain::value::PersonaId` was here, for "the tenancy
    // axis both sides carry". It is gone because the forge stopped
    // naming it: `boundary::Store` asks whether content exists, not
    // whose it is, and a line carries no owner for the answer to be
    // measured against. Removing it from this list is the point of
    // removing it from the code — what the lifted crate would have to
    // carry is one entry shorter.
    (
        "crate::domain::value::define_uuid_id",
        "how an id newtype is spelled. It shapes nothing, and it moves \
         with the split rather than before it",
    ),
    (
        "crate::domain::attribution::AttributionContext",
        "the write-side triple, named only where the forge asks who \
         somebody is — the one face that translates it into an actor. \
         A forge node records an `Actor`, never this",
    ),
];

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under the forge, by path relative to `src`.
fn forge_files() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in ["domain/forge", "application/forge"] {
        walk(&src().join(root), &mut found);
    }
    found
        .into_iter()
        .map(|path| path.strip_prefix(src()).expect("under src").to_path_buf())
        .collect()
}

fn walk(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("a forge directory") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() {
            walk(&path, into);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            into.push(path);
        }
    }
}

/// A file's code with its test module cut off.
///
/// Tests reach for whatever they need to build a fixture, and that is
/// not a statement about what the forge depends on.
fn without_tests(path: &Path) -> String {
    let text = fs::read_to_string(src().join(path)).expect("a readable file");
    match text.find("#[cfg(test)]") {
        Some(at) => text[..at].to_string(),
        None => text,
    }
}

/// Every path this file names outside the forge, one per `use`d item.
fn reaches_outside(code: &str) -> BTreeSet<String> {
    let mut named = BTreeSet::new();
    for line in code.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("use crate::") else {
            continue;
        };
        let rest = rest.trim_end_matches(';');
        if rest.contains("forge") {
            continue;
        }
        // `a::b::{C, D}` is two names; `a::b::C` is one.
        match rest.split_once("::{") {
            Some((prefix, items)) => {
                for item in items.trim_end_matches('}').split(',') {
                    named.insert(format!("crate::{}::{}", prefix, item.trim()));
                }
            }
            None => {
                named.insert(format!("crate::{rest}"));
            }
        }
    }
    named
}

#[test]
fn the_forge_names_nothing_outside_itself_but_the_shared_vocabulary() {
    let allowed: BTreeSet<&str> = SHARED_VOCABULARY.iter().map(|(path, _)| *path).collect();

    let mut unexpected = Vec::new();
    for file in forge_files() {
        let shown = file.to_string_lossy().replace('\\', "/");
        for named in reaches_outside(&without_tests(&file)) {
            if !allowed.contains(named.as_str()) {
                unexpected.push(format!("{shown}: {named}"));
            }
        }
    }

    assert!(
        unexpected.is_empty(),
        "the forge names something outside itself that no contract is written \
         in. Either it belongs to neither side — add it to `SHARED_VOCABULARY` \
         with the reason — or it is the coupling this guard exists to catch: \
         {unexpected:#?}"
    );
}

/// A reason is the work; the entry without one is a note that
/// something compiles.
#[test]
fn every_shared_word_says_why_it_is_shared() {
    for (path, reason) in SHARED_VOCABULARY {
        assert!(
            path.starts_with("crate::"),
            "an entry names a path from the crate root: {path}"
        );
        assert!(
            !reason.trim().is_empty(),
            "{path} is on the list without saying why the forge may name it"
        );
    }

    let named: BTreeSet<&str> = SHARED_VOCABULARY.iter().map(|(path, _)| *path).collect();
    assert_eq!(
        named.len(),
        SHARED_VOCABULARY.len(),
        "an entry is listed twice; two reasons for one import means one of \
         them is not the reason"
    );
}

/// Nothing under `domain/` may reach into `application/`, which is the
/// layering this codebase is built on and the one a module cannot
/// hold either.
#[test]
fn the_forges_domain_does_not_reach_up_into_the_application() {
    let mut reaching = Vec::new();
    for file in forge_files() {
        let shown = file.to_string_lossy().replace('\\', "/");
        if !shown.starts_with("domain/") {
            continue;
        }
        for named in reaches_outside(&without_tests(&file)) {
            if named.starts_with("crate::application") {
                reaching.push(format!("{shown}: {named}"));
            }
        }
    }

    assert!(
        reaching.is_empty(),
        "the forge's domain names something in the application layer: {reaching:#?}"
    );
}
