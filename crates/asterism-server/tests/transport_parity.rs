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
//! # Read as syntax, not as text
//!
//! Both sides are parsed with `syn` and walked, which is how
//! `asterism-core`'s `forge_boundary` reads the forge's imports. A
//! string scan was the first attempt here and it is the wrong tool for
//! this question, in the direction that costs: a `get(` inside a block
//! comment or a path literal is a route that does not exist, and a call
//! rustfmt wrapped across lines is a route that does and cannot be
//! seen. `forge_boundary`'s own doc says why that asymmetry decides it
//! — "an import written without one is invisible to a grep, which is
//! exactly the case that matters".
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

use syn::visit::Visit;

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
    ("create_team", "talks to a team, not to this process"),
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
    ("team_ledger_page", "talks to a team, not to this process"),
    ("team_roster", "talks to a team, not to this process"),
    (
        "team_server_session",
        "talks to a team, not to this process",
    ),
    (
        "close_shared_pursuit",
        "talks to a team, not to this process",
    ),
    ("my_teams", "talks to a team, not to this process"),
    (
        "open_shared_pursuit",
        "talks to a team, not to this process",
    ),
    (
        "promote_asset_to_team",
        "talks to a team, not to this process",
    ),
    ("push_shared_round", "talks to a team, not to this process"),
    (
        "shared_line_pursuits",
        "talks to a team, not to this process",
    ),
    ("shared_pursuit", "talks to a team, not to this process"),
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

/// Parses a source file, naming it when it will not parse.
fn parsed(path: &Path) -> syn::File {
    let text = read(path);
    syn::parse_file(&text).unwrap_or_else(|err| panic!("{} does not parse: {err}", path.display()))
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
    let mut found = Commands(BTreeSet::new());
    found.visit_file(&parsed(&path));
    assert!(
        !found.0.is_empty(),
        "no `#[tauri::command]` found in {} — the scan is not reading the \
         command module",
        path.display()
    );
    found.0
}

/// Collects the name of every function carrying `#[tauri::command]`.
struct Commands(BTreeSet<String>);

impl<'ast> Visit<'ast> for Commands {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if item.attrs.iter().any(is_tauri_command) {
            self.0.insert(item.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, item);
    }
}

/// Is this the attribute that makes a function reachable from the
/// window? Matched on the last path segment, so `#[tauri::command]` and
/// a `#[command]` reached through a `use` both answer yes.
fn is_tauri_command(attr: &syn::Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "command")
}

/// Every handler named by a method router inside `http::router`.
///
/// Method-scoped rather than "every call in the body": the router also
/// calls `nest_service` and `with_state`, which route nothing and name
/// no handler.
///
/// **An argument this cannot resolve panics rather than being skipped.**
/// A route the scan does not see is a route with no command, and both
/// pairings below would pass over it — fail-open in the one direction
/// #136 says the drift comes from. The forms resolved are what the tree
/// writes; the next one that is not is a failure asking to be taught.
fn routed_handlers(root: &Path) -> BTreeSet<String> {
    let path = root.join("crates/asterism-server/src/http.rs");
    let file = parsed(&path);
    let router = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(item) if item.sig.ident == "router" => Some(item),
            _ => None,
        })
        .expect("http.rs declares `fn router`");

    let mut found = Handlers {
        names: BTreeSet::new(),
        path: path.clone(),
    };
    found.visit_block(&router.block);
    assert!(
        !found.names.is_empty(),
        "no routed handler found in {} — the scan is not reading the router",
        path.display()
    );
    found.names
}

/// The method routers a handler can be named by. `axum::routing` has
/// more; these are what this router uses, and one it starts using
/// reaches the panic below rather than going unread.
const METHOD_ROUTERS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options"];

/// Collects the handler named by each method router in a router body.
struct Handlers {
    names: BTreeSet<String>,
    path: PathBuf,
}

impl<'ast> Visit<'ast> for Handlers {
    /// A method router written as a free function: `get(handler)`.
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(func) = &*call.func
            && let Some(named) = func.path.segments.last()
            && is_method_router(&named.ident)
        {
            self.take(&named.ident, call.args.first());
        }
        syn::visit::visit_expr_call(self, call);
    }

    /// The same router reached as a method on one already built:
    /// `get(read).post(write)`. Both forms are `axum::routing`, and the
    /// second is how every route carrying two verbs is written here —
    /// missing it drops the *second* handler of a pair while the first
    /// still pairs, which reads as a deliberate one-sided route rather
    /// than as a scan that stopped early.
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if is_method_router(&call.method) {
            self.take(&call.method, call.args.first());
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

/// Is this identifier one of `axum::routing`'s method routers?
fn is_method_router(ident: &syn::Ident) -> bool {
    METHOD_ROUTERS.iter().any(|name| ident == name)
}

impl Handlers {
    /// Records the handler a method router names, or refuses the whole
    /// run when it names something this cannot read.
    fn take(&mut self, method: &syn::Ident, arg: Option<&syn::Expr>) {
        match arg {
            // A closure routes a body written in place and names no
            // handler to pair.
            Some(syn::Expr::Closure(_)) => {}
            Some(syn::Expr::Path(handler)) => {
                let named = handler
                    .path
                    .segments
                    .last()
                    .expect("a path has at least one segment");
                self.names.insert(named.ident.to_string());
            }
            other => panic!(
                "a `{}` in {}'s router names something this scan cannot \
                 read as a handler: {}. Teach it that form — skipping it \
                 would hide a route from the pairing below",
                method,
                self.path.display(),
                match other {
                    Some(expr) => quote_shape(expr),
                    None => "no argument at all",
                }
            ),
        }
    }
}

/// Names the shape of an expression for a panic message, since `syn`
/// carries no source text to quote back.
fn quote_shape(expr: &syn::Expr) -> &'static str {
    match expr {
        syn::Expr::Call(_) => "a nested call",
        syn::Expr::MethodCall(_) => "a method call",
        syn::Expr::Macro(_) => "a macro invocation",
        syn::Expr::Reference(_) => "a reference",
        _ => "an expression of a kind this scan has not met",
    }
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
