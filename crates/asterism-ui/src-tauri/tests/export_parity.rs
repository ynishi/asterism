//! Holds the binding list to the contract it projects.
//!
//! The rule is in `build.rs`, beside the list it constrains: what
//! reaches TypeScript is a projection of the contract rather than a
//! subset a screen asked for, so an absence needs a reason about reach.
//! Nothing compared the two sides until this file existed, which is how
//! that doc came to describe three omissions while the contract had
//! grown many more.
//!
//! What this site adds is the pairing and a record of it.
//! `NOT_PROJECTED` holds what stays out with a reason each. And
//! `exported-types.txt`, beside this file and tracked in git, is written
//! from the same two scans — so a shape changing sides arrives as a diff
//! to read rather than as a number somebody has to re-derive and keep in
//! prose. That is the failure this whole change is about, and a count
//! stated here would be the next instance of it.
//!
//! Both sides are read as syntax rather than as text, the way
//! `asterism-core`'s `forge_boundary` reads the forge's imports and
//! `asterism-server`'s `transport_parity` reads the two transports.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use syn::visit::Visit;

/// Contract types that derive `SchemaBridge` and deliberately do not
/// reach TypeScript, each with the reason.
///
/// The reason is what makes an absence a decision. Every entry here is a
/// statement that no surface the app has will ever name this shape — not
/// that none does yet, which is a sentence about today that nothing
/// maintains.
const NOT_PROJECTED: &[(&str, &str)] = &[
    // The diagnostics, job-log and perf reads. Each is a route with no
    // Tauri command — `asterism-server`'s `transport_parity` records
    // them as differences a person never invokes through the app — so
    // there is no IPC path for a TypeScript caller to reach, and a name
    // here would describe a call that cannot be made. The *write* side
    // is exported: `build.rs` names `RecordDiagCommand`, because the
    // webview is itself a diagnostic source (`lib/diag.ts`).
    (
        "DiagDto",
        "read over HTTP only — no Tauri command serves it",
    ),
    (
        "ListDiagQuery",
        "read over HTTP only — no Tauri command serves it",
    ),
    (
        "JobLogDto",
        "read over HTTP only — no Tauri command serves it",
    ),
    (
        "ListJobLogQuery",
        "read over HTTP only — no Tauri command serves it",
    ),
    (
        "PerfDto",
        "read over HTTP only — no Tauri command serves it",
    ),
    (
        "ListPerfQuery",
        "read over HTTP only — no Tauri command serves it",
    ),
    // Severity, and the one entry here that is not about reach at all:
    // no wire shape carries this type in either direction. `DiagDto` and
    // `ListDiagQuery` spell the level as a `String`, the route that
    // publishes the vocabulary sends `&'static str`, and
    // `RecordDiagCommand::level` is a `String` on purpose — "so the
    // bindings stay flat; an unknown value is a validation error, not a
    // guessed level". `DiagLevel::parse` turns one into the closed type
    // at the boundary and it never crosses.
    (
        "DiagLevel",
        "no wire shape carries it — both directions spell severity as a `String`",
    ),
];

/// The workspace root, from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("crates/asterism-ui/src-tauri sits three levels below the root")
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

/// Every type in `asterism-contract` deriving `SchemaBridge`.
fn schema_bridge_types(root: &Path) -> BTreeSet<String> {
    let dir = root.join("crates/asterism-contract/src");
    let mut found = Derived(BTreeSet::new());
    let mut files = Vec::new();
    walk(&dir, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no sources under {} — the scan is not reading the contract",
        dir.display()
    );
    for path in &files {
        found.visit_file(&parsed(path));
    }
    assert!(
        !found.0.is_empty(),
        "no `SchemaBridge` derive found under {} — the scan is reading \
         sources but not seeing derives",
        dir.display()
    );
    found.0
}

/// Every `.rs` file under a directory, at any depth.
///
/// Recursive rather than a single `read_dir`, because a module that
/// becomes a directory would otherwise take its types out of the scan
/// silently — and a type that leaves the derived set stops being
/// something this file can fail on. `asterism-core`'s `forge_boundary`
/// walks for the same reason.
fn walk(dir: &Path, into: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("read an entry under {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            walk(&path, into);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            into.push(path);
        }
    }
}

/// Collects the name of every item deriving `SchemaBridge`.
struct Derived(BTreeSet<String>);

impl<'ast> Visit<'ast> for Derived {
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        if derives_schema_bridge(&item.attrs) {
            self.0.insert(item.ident.to_string());
        }
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        if derives_schema_bridge(&item.attrs) {
            self.0.insert(item.ident.to_string());
        }
        syn::visit::visit_item_enum(self, item);
    }
}

/// Does this item's attribute list carry `#[derive(… SchemaBridge …)]`?
///
/// Matched on the last path segment, so a `SchemaBridge` reached through
/// a `use` and one written out in full both answer yes.
fn derives_schema_bridge(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("derive"))
        .any(|attr| {
            attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
            )
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(|path| path.segments.last())
                    .any(|segment| segment.ident == "SchemaBridge")
            })
            .unwrap_or(false)
        })
}

/// Every type named in `build.rs`'s `export_types!` invocation.
///
/// **A name this cannot read stops the run.** The whole question is which
/// types reach TypeScript, and a scan that silently drops one from the
/// exported set reports it as an unexplained omission — a failure about
/// the wrong file, pointing at a type that is in fact exported.
fn exported_types(root: &Path) -> BTreeSet<String> {
    let path = root.join("crates/asterism-ui/src-tauri/build.rs");
    let mut found = Exported {
        names: BTreeSet::new(),
        path: path.clone(),
        seen_macro: false,
    };
    found.visit_file(&parsed(&path));
    assert!(
        found.seen_macro,
        "no `export_types!` invocation in {} — the scan is not reading the \
         binding list",
        path.display()
    );
    assert!(
        !found.names.is_empty(),
        "`export_types!` in {} names no type — the scan reached the macro \
         and read nothing out of it",
        path.display()
    );
    found.names
}

/// Collects the type names passed to `export_types!`.
struct Exported {
    names: BTreeSet<String>,
    path: PathBuf,
    seen_macro: bool,
}

impl Exported {
    /// Reads the macro's arguments. The first is the output path as a
    /// string literal; the rest name types.
    fn take(&mut self, mac: &syn::Macro) {
        if !mac
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == "export_types")
        {
            return;
        }
        self.seen_macro = true;
        let args = mac
            .parse_body_with(
                syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
            )
            .unwrap_or_else(|err| {
                panic!(
                    "`export_types!` in {} does not parse as a comma-separated \
                     list: {err}",
                    self.path.display()
                )
            });
        for arg in &args {
            match arg {
                // The destination path.
                syn::Expr::Lit(_) => {}
                syn::Expr::Path(named) => {
                    let last = named
                        .path
                        .segments
                        .last()
                        .expect("a path has at least one segment");
                    self.names.insert(last.ident.to_string());
                }
                _ => panic!(
                    "`export_types!` in {} names something this scan cannot \
                     read as a type. Teach it that form — skipping it would \
                     report an exported type as an unexplained omission",
                    self.path.display()
                ),
            }
        }
    }
}

impl<'ast> Visit<'ast> for Exported {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.take(mac);
        syn::visit::visit_macro(self, mac);
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
fn every_contract_type_is_projected_or_has_a_recorded_reason() {
    let root = workspace_root();
    let derived = schema_bridge_types(&root);
    let exported = exported_types(&root);

    let absent: BTreeSet<String> = derived.difference(&exported).cloned().collect();
    let allowed: BTreeSet<String> = NOT_PROJECTED
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();

    let undeclared: BTreeSet<String> = absent.difference(&allowed).cloned().collect();
    assert!(
        undeclared.is_empty(),
        "these contract types derive `SchemaBridge` and reach no \
         TypeScript caller:{}\n\n\
         `bindings.ts` is a projection of the contract, not a subset a \
         screen asks for. Add the type to `export_types!` in `build.rs`, \
         or, if no surface this app has will ever name it, add it to \
         NOT_PROJECTED with that reason.",
        listed(&undeclared)
    );

    let stale: BTreeSet<String> = allowed.difference(&absent).cloned().collect();
    assert!(
        stale.is_empty(),
        "NOT_PROJECTED names types that are no longer absent:{}\n\n\
         Either the type is exported now and the entry should go, or it \
         left the contract — remove them in the change that did it.",
        listed(&stale)
    );
}

#[test]
fn every_exported_name_is_a_contract_type() {
    let root = workspace_root();
    let derived = schema_bridge_types(&root);
    let exported = exported_types(&root);

    let unknown: BTreeSet<String> = exported.difference(&derived).cloned().collect();
    assert!(
        unknown.is_empty(),
        "`export_types!` names types that do not derive `SchemaBridge` in \
         `asterism-contract`:{}\n\n\
         A name here that the contract does not carry is a build failure \
         waiting for the next `cargo check`, or a type that moved and left \
         its name behind.",
        listed(&unknown)
    );
}

/// Renders the two sides as the tracked record beside this file.
///
/// Sorted, one type per line, each marked with which side it is on. The
/// header carries the sizes — this file holds the lists, so it is the
/// one place a count of them does not rot.
fn record(derived: &BTreeSet<String>, exported: &BTreeSet<String>) -> String {
    let mut out = String::from(
        "# Which of asterism-contract's SchemaBridge types reach TypeScript.\n\
         #\n\
         # Written by tests/export_parity.rs and tracked, so a shape changing\n\
         # sides is a diff to read. Regenerate with:\n\
         #\n\
         #     just rust-test-one asterism-ui --test export_parity\n\
         #\n\
         # `withheld` means the type reaches no TypeScript caller on purpose;\n\
         # NOT_PROJECTED in that test carries the reason for each.\n\
         #\n",
    );
    out.push_str(&format!("# derived: {}\n", derived.len()));
    out.push_str(&format!("# exported: {}\n", exported.len()));
    out.push_str(&format!(
        "# withheld: {}\n\n",
        derived.difference(exported).count()
    ));
    for name in derived {
        let side = if exported.contains(name) {
            "exported"
        } else {
            "withheld"
        };
        out.push_str(&format!("{side:<9} {name}\n"));
    }
    out
}

/// The record's path.
fn record_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/exported-types.txt")
}

#[test]
fn the_record_beside_this_file_says_what_the_tree_says() {
    let root = workspace_root();
    let want = record(&schema_bridge_types(&root), &exported_types(&root));
    let path = record_path();

    let have = fs::read_to_string(&path).unwrap_or_default();
    if have == want {
        return;
    }

    // Written rather than only reported, because the alternative is a
    // panic carrying two hundred lines for a reader to diff by eye. The
    // gate is `git`: the file is tracked, so an unintended move arrives
    // as a diff in the change that caused it, and a run on a clean tree
    // leaves nothing behind.
    fs::write(&path, &want).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    panic!(
        "{} did not match the tree and has been rewritten. Read the diff: \
         if a type changed sides on purpose, commit it with the change \
         that moved it.",
        path.display()
    );
}

#[test]
fn every_recorded_omission_carries_a_reason() {
    for (name, reason) in NOT_PROJECTED {
        assert!(
            !reason.trim().is_empty(),
            "`{name}` is recorded as not projected with no reason. An \
             absence with a reason is a decision; one without is what this \
             list exists to fail on"
        );
    }
}
