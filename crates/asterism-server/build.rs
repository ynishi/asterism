//! Build identity for the serving process.
//!
//! Embeds the workspace HEAD commit as `ASTERISM_GIT_SHA` so
//! `/asterism/health` can answer "which build is actually serving this
//! port". That question is the whole restart story: after a rebuild the
//! only way to know the new binary took over is to compare the served
//! sha against the repo — a relaunch that silently leaves the old
//! process bound to the port (the 2026-07-31 dogfood incident) then
//! reads as a mismatch instead of as success.
//!
//! Known limit: the sha names the commit the build was made from, not
//! the working tree — two dirty builds off the same HEAD share a sha.
//! `/asterism/health` therefore also reports `pid` / `started_at_ms`,
//! which is what proves "a different process answered".

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let sha = git(&manifest_dir, &["rev-parse", "--short=12", "HEAD"])
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=ASTERISM_GIT_SHA={sha}");

    for path in watch_set(&manifest_dir) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// Run `git` inside the package and return trimmed stdout, or `None` for
/// any failure — no git, no repo, a tarball build. Every caller treats
/// that as "cannot know", never as an error: a build that cannot name
/// its commit still has to produce a binary.
fn git(manifest_dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", manifest_dir])
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// The files whose mtime moving means `HEAD` now resolves to a different
/// commit. Getting this set wrong does not fail loudly — it bakes a
/// stale sha into a fresh binary, which is worse than no sha at all
/// because the value exists to *detect* staleness.
///
/// # Why the common dir, and not just `--git-dir`
///
/// In a linked worktree `--git-dir` is `.git/worktrees/<name>`, and
/// none of the commit-bearing state lives there (observed in a linked
/// worktree, 2026-08-06):
///
/// - `.git/worktrees/<name>/HEAD` is a symref (`ref: refs/heads/...`).
///   Its *content* is what a branch switch rewrites; a commit leaves it
///   byte-identical.
/// - `.git/worktrees/<name>/refs` is empty.
/// - The file a commit actually rewrites is
///   `.git/refs/heads/<branch>` — on the **common** dir side.
///
/// So the old watch set never fired in a worktree and the first build's
/// sha stayed burned in for the life of the branch. It fired in the
/// main checkout only because there `--git-dir` and the common dir are
/// the same path, which is exactly why the hole stayed invisible: the
/// coding pipeline runs in worktrees, so the broken case is the common
/// one.
///
/// Both are kept. The per-worktree `HEAD` is what changes on a branch
/// switch (and holds the raw sha when detached); the common `refs` tree
/// is what changes on a commit.
///
/// # Why only paths that exist
///
/// Cargo documents mtime as the whole mechanism and says nothing about
/// what a missing path means, so emitting one is a bet on unspecified
/// behaviour. `packed-refs` is the case that matters: it is absent
/// until something packs refs. Dropping it while absent is safe because
/// packing *deletes the loose ref*, and that deletion changes the
/// `refs` tree we are already scanning — the next run then sees
/// `packed-refs` and starts watching it. The set repairs itself one
/// build later rather than depending on a rule cargo has not written
/// down.
///
/// An empty set is also a deliberate outcome (no git at all): emitting
/// no `rerun-if-changed` puts cargo back on its conservative default of
/// re-running whenever any file in the package changes.
fn watch_set(manifest_dir: &str) -> Vec<PathBuf> {
    let git_dir = git(manifest_dir, &["rev-parse", "--absolute-git-dir"]).map(PathBuf::from);
    // `--git-common-dir` alone answers relative to the process CWD
    // (`../../.git` from this package). `--path-format=absolute` pins
    // it; on a git too old to know that flag we fall back to the git
    // dir, which is correct everywhere except the worktree case this
    // function exists for.
    let common_dir = git(
        manifest_dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(PathBuf::from)
    .or_else(|| git_dir.clone());

    let mut paths = Vec::new();
    if let Some(dir) = &git_dir {
        paths.push(dir.join("HEAD"));
    }
    if let Some(dir) = &common_dir {
        paths.push(dir.join("HEAD"));
        paths.push(dir.join("refs"));
        paths.push(dir.join("packed-refs"));
    }
    paths.sort();
    paths.dedup();
    paths.retain(|p| p.exists());
    paths
}
