//! Holds the two transports to the obligation their module docs state.
//!
//! The rule is in this crate's `http` module doc: HTTP and Tauri owe
//! each other every verb a person invokes, in the same change. It was
//! enforced by a person reading two files, and by a count in that prose
//! saying what they had found — which went stale each time somebody
//! moved a verb without re-reading it, and that doc records how often.
//!
//! This file is what took the counting over, and it states no count.
//! The lists below are the answer; their sizes are not written anywhere,
//! which is the second branch of CONTRIBUTING's third documentation rule
//! — a number describing a list belongs in the file holding the list, or
//! nowhere.
//!
//! # Which direction this covers, and which it does not
//!
//! It reads a file from each crate, so it cannot sit in "the crate that
//! owns the file" the way `src-tauri`'s `mutation_surface.rs` does
//! (#159). `changed-packages` maps a changed path to the member
//! directory containing it, so a branch touching only `commands.rs`
//! selects `asterism-ui` and this test does not run on it.
//!
//! Sitting here is the side that matters, and #136 is why: every one of
//! the sixteen verbs it counted "landed in a change whose scope said
//! `routes`, and nothing counted the other side afterwards". The drift
//! that has actually happened is a route arriving without its command,
//! and that branch touches `http.rs` and selects this crate. The
//! opposite gap is real and left open on purpose — a command arriving
//! without its route is caught one merge later, on `main`'s full run,
//! which is the same trade `changed-packages` documents for every
//! dependent it does not rebuild.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Routed handlers with no Tauri command of the same name, each with the
/// reason it is not one.
///
/// The reasons are the point rather than the names: a difference with a
/// reason is a decision, and one without is the defect this file exists
/// to fail on. Grouped the way `http`'s module doc groups them, because
/// that prose is the statement of the rule and this is its list.
const ROUTES_WITHOUT_COMMAND: &[(&str, &str)] = &[
    // The same job under another name. Renaming for symmetry is churn
    // (#136 ruled it out of scope), so the pairing is recorded here
    // instead.
    (
        "declare_asset_album_meta",
        "command is `asset_declare_meta`",
    ),
    (
        "declare_asset_provenance",
        "command is `asset_declare_provenance`",
    ),
    (
        "declare_asset_source_type",
        "command is `asset_declare_source_type`",
    ),
    ("delete_message", "command is `delete_thread_message`"),
    ("get_thumb", "command is `get_asset_thumb`"),
    ("list_threads_by_anchor", "command is `list_threads`"),
    // One command answers all four: an anchor has four variants of three
    // arities, which a router cannot express in one path without
    // accepting combinations that have no answer.
    (
        "threads_about_change",
        "one of four routes behind `list_forge_threads_about`",
    ),
    (
        "threads_about_entry",
        "one of four routes behind `list_forge_threads_about`",
    ),
    (
        "threads_about_pursuit",
        "one of four routes behind `list_forge_threads_about`",
    ),
    (
        "threads_about_round",
        "one of four routes behind `list_forge_threads_about`",
    ),
    // What a person never invokes.
    ("health", "the process's own control, not a person's verb"),
    (
        "shutdown_process",
        "the process's own control, not a person's verb",
    ),
    (
        "get_asset_file",
        "serves bytes the app reaches through Tauri's asset protocol",
    ),
    (
        "put_thumb",
        "serves bytes the app reaches through Tauri's asset protocol",
    ),
    ("jobs_depth", "diagnostics a socket client reads"),
    ("list_diag", "diagnostics a socket client reads"),
    ("list_diag_levels", "diagnostics a socket client reads"),
    ("list_job_log", "diagnostics a socket client reads"),
    ("list_perf", "diagnostics a socket client reads"),
    // A shape the desktop already has (#136's decision): `list_settings`
    // returns every registry key fully resolved, and the two writes
    // return the resolved row, so a single-key IPC read would be a
    // second way to ask a question the app has already answered.
    (
        "get_setting",
        "the desktop reads settings through `list_settings`",
    ),
];

/// Tauri commands with no route of the same name, each with the reason.
///
/// The other side of the same question, checked for the same reason: a
/// name leaving this list silently is how a pair of twins would drift
/// apart without either half looking wrong.
const COMMANDS_WITHOUT_ROUTE: &[(&str, &str)] = &[
    // The command-side names of the twins above.
    ("asset_declare_meta", "route is `declare_asset_album_meta`"),
    (
        "asset_declare_provenance",
        "route is `declare_asset_provenance`",
    ),
    (
        "asset_declare_source_type",
        "route is `declare_asset_source_type`",
    ),
    ("delete_thread_message", "route is `delete_message`"),
    ("get_asset_thumb", "route is `get_thumb`"),
    ("list_threads", "route is `list_threads_by_anchor`"),
    (
        "list_forge_threads_about",
        "answers the four `threads_about_*` routes",
    ),
    // Desktop facts a socket client does not have.
    (
        "paste_image_import",
        "stages clipboard material, which a socket client has none of",
    ),
    (
        "rehome_dropped_path",
        "stages drag-drop material, which a socket client has none of",
    ),
    // A second shape over a route that already exists.
    (
        "get_asset_thumbs",
        "a batch second command over the single-thumb route",
    ),
    // Desktop chrome.
    (
        "active_profile",
        "desktop chrome: which local data profile this process opened",
    ),
    // Deliberately one-sided, and #153 argues why where the obligation
    // itself is stated: a verb against somebody else's server is not a
    // verb against this one.
    ("clone_shared_entry", "talks to a team, not to this process"),
    (
        "connect_team_server",
        "talks to a team, not to this process",
    ),
    (
        "disconnect_team_server",
        "talks to a team, not to this process",
    ),
    ("list_shared_lines", "talks to a team, not to this process"),
    (
        "publish_line_to_team",
        "talks to a team, not to this process",
    ),
    (
        "shared_line_history",
        "talks to a team, not to this process",
    ),
    ("shared_line_states", "talks to a team, not to this process"),
    (
        "team_server_session",
        "talks to a team, not to this process",
    ),
];

/// The workspace root, from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<member> sits two levels below the workspace root")
        .to_path_buf()
}

/// Reads a file, naming it when it is not there — a moved source is a
/// scan reading nothing, which would otherwise pass as "no drift".
fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Source lines with `//` comments dropped, so a name inside prose is
/// not mistaken for a declaration. The same filter `mutation_surface.rs`
/// applies, and for the same reason.
fn code_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect()
}

/// Every `#[tauri::command]` function name in the desktop's command
/// module.
///
/// Declarations, not registrations: what the window can invoke is the
/// `generate_handler!` list in `lib.rs`, and a command declared without
/// being registered would satisfy the pairing while being unreachable.
/// The two agree today, and holding them to each other is a different
/// question from this one — a verb the socket has and the desktop does
/// not.
fn tauri_commands(root: &Path) -> BTreeSet<String> {
    let path = root.join("crates/asterism-ui/src-tauri/src/commands.rs");
    let text = read(&path);
    let lines = code_lines(&text);

    let mut names = BTreeSet::new();
    for (index, line) in lines.iter().enumerate() {
        if line.trim() != "#[tauri::command]" {
            continue;
        }
        // The attribute sits directly above the signature, but a
        // `#[allow(...)]` or a second attribute can come between.
        let name = lines[index + 1..]
            .iter()
            .take(5)
            .find_map(|candidate| fn_name(candidate))
            .unwrap_or_else(|| {
                panic!(
                    "`#[tauri::command]` on line {} names no function",
                    index + 1
                )
            });
        names.insert(name);
    }
    assert!(
        !names.is_empty(),
        "no `#[tauri::command]` found in {} — the scan is not reading the \
         command module",
        path.display()
    );
    names
}

/// The function name declared by a signature line, if it declares one.
fn fn_name(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("pub ").unwrap_or(line.trim());
    let rest = rest.strip_prefix("async ").unwrap_or(rest);
    let rest = rest.strip_prefix("fn ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Every handler named by a method call inside `http::router`.
///
/// Method-scoped rather than "every identifier in a call": the router
/// body calls things that route nothing, and only a method router names
/// a handler.
///
/// **A `method(` this cannot resolve panics rather than being skipped.**
/// The command side already fails that way, and a scan that quietly
/// drops what it does not recognise is fail-open in the one direction
/// #136 says the drift comes from — an unread route is a route with no
/// command, and both tests would pass over it. The forms it resolves
/// are what the tree writes today; the next one that is not is a
/// failure asking to be taught, not a silent gap.
fn routed_handlers(root: &Path) -> BTreeSet<String> {
    let path = root.join("crates/asterism-server/src/http.rs");
    let text = read(&path);
    let lines = code_lines(&text);

    let start = lines
        .iter()
        .position(|line| line.starts_with("pub fn router("))
        .expect("http.rs declares `pub fn router(`");
    // The router is one expression ending at the first column-zero
    // brace after it.
    let end = lines[start + 1..]
        .iter()
        .position(|line| *line == "}")
        .map(|offset| start + 1 + offset)
        .expect("`router` has a closing brace at column zero");
    // Joined, so a call rustfmt wrapped across lines still reads as one
    // call — the width at which it wraps is not something this should
    // depend on.
    let body = lines[start..end].join(" ");

    let mut names = BTreeSet::new();
    for method in ["get(", "post(", "put(", "patch(", "delete("] {
        let mut rest = body.as_str();
        while let Some(at) = rest.find(method) {
            // `.get(` and `get(` both reach here; `forget(` must not.
            let preceded_by_ident = rest[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            rest = &rest[at + method.len()..];
            if preceded_by_ident {
                continue;
            }
            let arg = rest.trim_start();
            // A closure routes a body written in place and names no
            // handler to pair.
            if arg.starts_with('|') {
                continue;
            }
            let name = handler_name(arg).unwrap_or_else(|| {
                panic!(
                    "a `{method}` in {}'s router names something this scan \
                     cannot read as a handler: `{}`. Teach it that form — \
                     skipping it would hide a route from the pairing below",
                    path.display(),
                    arg.chars().take(40).collect::<String>()
                )
            });
            names.insert(name);
        }
    }
    assert!(
        !names.is_empty(),
        "no routed handler found in {} — the scan is not reading the router",
        path.display()
    );
    names
}

/// The handler a method router's argument names, if it names one.
///
/// Takes the last segment of a path, so `handlers::foo` and a turbofish
/// both answer `foo` — the pairing is by name, and a route reached
/// through a module is the same verb as one written bare.
fn handler_name(arg: &str) -> Option<String> {
    let path: String = arg
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
        .collect();
    let last = path.rsplit("::").find(|segment| !segment.is_empty())?;
    Some(last.to_string())
}

/// Renders a difference as sorted lines, for a message a reader can act
/// on without running anything.
fn listed(names: &BTreeSet<String>) -> String {
    names
        .iter()
        .map(|name| format!("\n  {name}"))
        .collect::<String>()
}

#[test]
fn every_routed_handler_has_a_command_or_a_recorded_reason() {
    let root = workspace_root();
    let commands = tauri_commands(&root);
    let routes = routed_handlers(&root);

    let unpaired: BTreeSet<String> = routes.difference(&commands).cloned().collect();
    let allowed: BTreeSet<String> = ROUTES_WITHOUT_COMMAND
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();

    let undeclared: BTreeSet<String> = unpaired.difference(&allowed).cloned().collect();
    assert!(
        undeclared.is_empty(),
        "a route landed with no Tauri command of the same name:{}\n\n\
         The two surfaces owe each other every verb a person invokes, in \
         the same change — the rule is in this crate's `http` module doc, \
         which is also where the grounds a difference can stand on are \
         stated. Add the command, or add the name to ROUTES_WITHOUT_COMMAND \
         under the grouping it belongs to, with its reason.",
        listed(&undeclared)
    );

    let stale: BTreeSet<String> = allowed.difference(&unpaired).cloned().collect();
    assert!(
        stale.is_empty(),
        "ROUTES_WITHOUT_COMMAND names entries that are no longer \
         unpaired:{}\n\nEither the command arrived and the entry should go, \
         or the route did — remove them in the change that paired them.",
        listed(&stale)
    );
}

#[test]
fn every_command_has_a_route_or_a_recorded_reason() {
    let root = workspace_root();
    let commands = tauri_commands(&root);
    let routes = routed_handlers(&root);

    let unpaired: BTreeSet<String> = commands.difference(&routes).cloned().collect();
    let allowed: BTreeSet<String> = COMMANDS_WITHOUT_ROUTE
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();

    let undeclared: BTreeSet<String> = unpaired.difference(&allowed).cloned().collect();
    assert!(
        undeclared.is_empty(),
        "a Tauri command landed with no route of the same name:{}\n\n\
         The two surfaces owe each other every verb a person invokes, in \
         the same change — the rule is in this crate's `http` module doc, \
         which is also where the grounds a difference can stand on are \
         stated. Add the route, or add the name to COMMANDS_WITHOUT_ROUTE \
         under the grouping it belongs to, with its reason.",
        listed(&undeclared)
    );

    let stale: BTreeSet<String> = allowed.difference(&unpaired).cloned().collect();
    assert!(
        stale.is_empty(),
        "COMMANDS_WITHOUT_ROUTE names entries that are no longer \
         unpaired:{}\n\nEither the route arrived and the entry should go, \
         or the command did — remove them in the change that paired them.",
        listed(&stale)
    );
}

/// Pins the argument forms the router scan resolves, and the one it
/// refuses.
///
/// These are what `routed_handlers` panics or does not panic on, and
/// the panic is the point: a form this cannot read has to stop the
/// suite rather than drop a route out of the pairing. The bare name is
/// what the tree writes today; the other two are here so that adopting
/// either is not also a silent change in what gets checked.
#[test]
fn a_handler_is_named_by_the_last_segment_of_its_path() {
    assert_eq!(
        handler_name("list_forge_lines)").as_deref(),
        Some("list_forge_lines")
    );
    assert_eq!(
        handler_name("handlers::list_forge_lines)").as_deref(),
        Some("list_forge_lines"),
        "a route reached through a module is the same verb as a bare one"
    );
    assert_eq!(
        handler_name("list_forge_lines::<Body>)").as_deref(),
        Some("list_forge_lines"),
        "a turbofish names the same handler"
    );
    assert_eq!(
        handler_name("(something_nested())"),
        None,
        "what this cannot read must reach the panic rather than be skipped"
    );
}

#[test]
fn every_recorded_exception_carries_a_reason() {
    for (name, reason) in ROUTES_WITHOUT_COMMAND.iter().chain(COMMANDS_WITHOUT_ROUTE) {
        assert!(
            !reason.trim().is_empty(),
            "`{name}` is on an exception list with no reason. A difference \
             with a reason is a decision; one without is the defect these \
             lists exist to fail on"
        );
    }
}
