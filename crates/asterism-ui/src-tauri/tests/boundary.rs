//! The frontend has one vocabulary, and this is what keeps it that
//! way.
//!
//! # The layer this guards
//!
//! A fact reaching a screen from the team plane crosses two
//! boundaries, and they speak different languages:
//!
//! ```text
//!   teams-server ──HTTP── asterism-teams-wire ──┐
//!                                               │  the wire's language
//!                            commands.rs  ──────┤  ← the only bilingual site
//!                                               │  the contract's language
//!   frontend ──Tauri IPC── asterism-contract ───┘
//! ```
//!
//! `bindings.ts` is a projection of `asterism-contract` alone, which is
//! what gives every screen one set of types to know. A command whose
//! signature named a wire type would put the second language on the far
//! side of the IPC boundary — and the frontend would then hold shapes
//! that arrived from a server it never talks to.
//!
//! # Why a test rather than a convention
//!
//! Because the convention was already there and did not hold. A ledger
//! surface landed with `team_ledger_page` returning
//! `asterism_teams_wire::dto::LedgerPageDto` and the wire crate added
//! to both dependency tables; `bindings.ts` grew a second source, and
//! the screen imported from it. `export_parity` refused the export —
//! correctly — but it answers about the *list*, so it could only say
//! that three names were not the contract's, not that a boundary had
//! moved. Nothing said the thing that had actually gone wrong.
//!
//! The order this enforces is the one the layers already imply: a
//! backend verb reachable over HTTP is not reachable from a screen
//! until a contract DTO and a command exist for it. Skipping the middle
//! is what this catches.
//!
//! # What it does not catch
//!
//! A command that maps a wire shape into a *wrong* contract shape, or
//! one whose contract type nothing on the wire supports. Both are
//! ordinary review; what is mechanical here is only that the language
//! does not cross.

use std::fs;
use std::path::{Path, PathBuf};

/// This crate's own directory.
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under a directory, recursively.
fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
    found
}

#[test]
fn the_wire_vocabulary_does_not_cross_into_this_crate() {
    let root = crate_dir();
    let mut offenders = Vec::new();

    // The two manifests. Naming the crate in either is what makes the
    // types spellable at all, so the check starts there rather than at
    // the use site.
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    for (number, line) in manifest.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("asterism-teams-wire") {
            offenders.push(format!("Cargo.toml:{}: {}", number + 1, line.trim()));
        }
    }

    // The sources, including the build script, which is where an
    // export list would name one.
    let mut sources = rust_sources(&root.join("src"));
    sources.push(root.join("build.rs"));
    for path in sources {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            // The path form is what a signature or an import uses; a
            // comment naming the crate in prose is not a crossing, and
            // this file's own header is an example of why that
            // distinction has to be drawn.
            if line.contains("asterism_teams_wire::") {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the teams wire is named inside the app's own boundary:\n{}\n\n\
         `bindings.ts` is a projection of `asterism-contract` alone, so a \
         command handing a screen a wire type gives the frontend a second \
         vocabulary — one belonging to a server it never talks to. Add the \
         shape to `asterism-contract::teams` and map to it in the command \
         that fetches it; `asterism-teams-client` stays the only crate here \
         that speaks both.",
        offenders
            .iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
