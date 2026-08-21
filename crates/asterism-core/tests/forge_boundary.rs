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
//! # This is the list, and the `SHARED KERNEL` comments are not
//!
//! Those comments mark the same edges at the point of use, and they
//! are worth reading, but they cannot be the authority: a new import
//! without one is invisible to a grep, which is exactly the case that
//! matters. The list here is checked whether anybody remembers or not.
//!
//! # Two populations, and the second is shrinking
//!
//! The model this branch replaces still has files under `forge`, and
//! they reach further — into the raw layer's repositories, into the
//! application's parsing helpers. They are exempt by name rather than
//! by rule, because what they name is not a decision anybody is
//! defending; it is what a deletion will take with it. The exemption
//! list is a measure of how much is left.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// What the forge may name outside itself, and why each one is not a
/// leak.
///
/// Adding to this list is a decision about what the forge crate would
/// have to depend on. It is not a place to record that something
/// compiles.
const LIFT_SURFACE: &[(&str, &str)] = &[
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
    (
        "crate::domain::value::PersonaId",
        "the tenancy axis both sides carry. The forge does not decide \
         what ownership means; it asks, and asking without saying \
         whose is asking a question with two right answers",
    ),
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

/// Files still serving the model this branch replaces.
///
/// Exempt as a whole, because what they reach for goes when they do.
/// The list shrinks; it does not grow.
const LEAVING: &[&str] = &[
    "domain/forge/line.rs",
    "domain/forge/project.rs",
    "domain/forge/pursuit.rs",
    "domain/forge/repository.rs",
    "domain/forge/tx.rs",
    "domain/forge/value.rs",
    "application/forge/legacy_pursuit_service.rs",
    "application/forge/project_service.rs",
    "application/forge/mapping.rs",
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
fn the_forge_names_nothing_outside_itself_that_is_not_on_the_list() {
    let allowed: BTreeSet<&str> = LIFT_SURFACE.iter().map(|(path, _)| *path).collect();
    let leaving: BTreeSet<&str> = LEAVING.iter().copied().collect();

    let mut unexpected = Vec::new();
    for file in forge_files() {
        let shown = file.to_string_lossy().replace('\\', "/");
        if leaving.contains(shown.as_str()) {
            continue;
        }
        for named in reaches_outside(&without_tests(&file)) {
            if !allowed.contains(named.as_str()) {
                unexpected.push(format!("{shown}: {named}"));
            }
        }
    }

    assert!(
        unexpected.is_empty(),
        "the forge reaches for something that is not on its lift surface. Either \
         it belongs there — add it to `LIFT_SURFACE` with the reason it is \
         shared rather than borrowed — or it is the coupling this guard exists \
         to catch: {unexpected:#?}"
    );
}

/// A reason is the work; the entry without one is a note that
/// something compiles.
#[test]
fn every_entry_on_the_lift_surface_says_why_it_is_there() {
    for (path, reason) in LIFT_SURFACE {
        assert!(
            path.starts_with("crate::"),
            "an entry names a path from the crate root: {path}"
        );
        assert!(
            !reason.trim().is_empty(),
            "{path} is on the list without saying why the forge may name it"
        );
    }

    let named: BTreeSet<&str> = LIFT_SURFACE.iter().map(|(path, _)| *path).collect();
    assert_eq!(
        named.len(),
        LIFT_SURFACE.len(),
        "an entry is listed twice; two reasons for one import means one of \
         them is not the reason"
    );
}

/// The exemption list measures what is left of the model being
/// replaced. A name that matches nothing is a file already deleted,
/// and leaving it there would hide the next one.
#[test]
fn every_exempt_file_still_exists() {
    let present: BTreeSet<String> = forge_files()
        .iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();

    let stale: Vec<&&str> = LEAVING
        .iter()
        .filter(|file| !present.contains(**file))
        .collect();

    assert!(
        stale.is_empty(),
        "these files are exempt and gone. The exemption list is how much of the \
         replaced model is left, so a stale name overstates it: {stale:#?}"
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
