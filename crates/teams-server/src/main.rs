//! `teams-server` — the hosted Team plane's binary (stub).
//!
//! This slice scaffolds the crate and its dependency edges only. The
//! real composition root — axum `/teams/*` + rmcp on one router, the
//! session→membership gate middleware, and the `serve` / `backup` /
//! `bootstrap` CLI — is #83 §4/§5 follow-up work.

fn main() {
    // A placeholder, not a server: exiting immediately is the honest
    // behavior until the first route exists.
    println!(
        "teams-server: scaffold only — the serve/backup/bootstrap CLI arrives \
         with the #83 infra and API slices"
    );
}
