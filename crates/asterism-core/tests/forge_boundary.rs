//! What the forge is allowed to name outside itself.
//!
//! The forge is meant to be liftable: a crate of its own, depending on
//! a small shared vocabulary and on nothing else in this one. Today it
//! is a module, and a module holds nothing — `use crate::domain::asset`
//! in a forge file compiles, and the only thing between here and there
//! is somebody noticing.
//!
//! So this is the thing that notices. It reads every forge file that is
//! not a test, collects what it names outside the forge, and refuses
//! anything not on the list below with a reason beside it.
//!
//! # Why this reads syntax rather than lines
//!
//! It used to match text: a line starting `use crate::`, minus any line
//! containing the word "forge". Four shapes went through it, all of
//! them ordinary Rust, and each was demonstrated against the tree
//! before this was written.
//!
//! - `use crate::application::forge::…` — dropped by the filter for the
//!   word "forge", which is in every application-side forge path. The
//!   filter was there to skip the forge naming itself, and it could not
//!   tell that from the forge's own service being named by its model.
//!   [`the_forges_domain_does_not_reach_up_into_the_application`] still
//!   saw `crate::application::asset_service::…`; what it could not see
//!   was the forge-named half, which is the half somebody actually
//!   writes.
//! - `pub use crate::…` — the line does not start with `use`.
//! - a `use` rustfmt had wrapped over several lines — only the first
//!   line was read, and what it recorded was the prefix with the items
//!   missing.
//! - `use crate::x::Y as Z` — recorded as `crate::x::Y as Z`, which
//!   matches nothing on the list, so an allowed word written with a
//!   rename would have been reported as coupling.
//!
//! Not one of the four is about what the code means. Three are `use`
//! trees the scan could not walk and one is a line break; the syntax
//! has them all in hand before the question is even asked. Reading the
//! syntax is not a stricter version of reading the text — it is the
//! difference between answering the question and answering a question
//! about formatting.
//!
//! # This is the list, and the `SHARED VOCABULARY` comments are not
//!
//! That comment marks the same edges at the point of use — a word
//! neither side owns, which a contract may therefore be written in —
//! and it is worth reading where it sits. It cannot be the authority:
//! an import written without one is invisible to a grep, which is
//! exactly the case that matters. The list here is checked whether
//! anybody remembered or not.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use syn::visit::Visit;
use syn::{File, Item, UseTree};

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

/// Where the forge's own code lives, as path prefixes from the crate
/// root. A name under one of these is the forge talking to itself.
const FORGE_ROOTS: &[&str] = &["crate::domain::forge", "crate::application::forge"];

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

fn parsed(path: &Path) -> File {
    let text = fs::read_to_string(src().join(path)).expect("a readable file");
    syn::parse_file(&text).unwrap_or_else(|err| panic!("{} does not parse: {err}", path.display()))
}

/// Does this item carry `#[cfg(test)]`?
fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Meta>()
                .is_ok_and(|meta| meta.path().is_ident("test"))
    })
}

/// The forge files that are test code in their entirety.
///
/// A `#[cfg(test)]` inside a file is an item, and skipping it is easy.
/// A whole file of tests carries no such attribute anywhere in itself —
/// the `#[cfg(test)] mod tests;` that brings it in is in the *parent*,
/// which is why the previous version read
/// `domain/forge/strategies/tests.rs` as production code and measured
/// what its fixtures reach for.
///
/// Only the top level of each file is read for these declarations. A
/// `#[cfg(test)] mod tests;` nested inside an inline module would be
/// missed and its file read as production — which fails closed: the
/// file is checked when it need not have been, and somebody deletes a
/// line from the allow-list rather than a coupling going unseen.
fn test_files() -> BTreeSet<PathBuf> {
    let mut declared = BTreeSet::new();
    for file in forge_files() {
        let dir = match file.file_name().and_then(|n| n.to_str()) {
            // `foo/mod.rs` declares into `foo/`, and so does the crate
            // root of a directory module.
            Some("mod.rs") => file.parent().expect("a parent").to_path_buf(),
            // `foo.rs` declares into `foo/`.
            _ => file.with_extension(""),
        };
        for item in &parsed(&file).items {
            let Item::Mod(module) = item else { continue };
            // A module with a body is skipped where it is met. Only a
            // file module sends us looking for another file.
            if module.content.is_some() || !is_test_only(&module.attrs) {
                continue;
            }
            let name = module.ident.to_string();
            declared.insert(dir.join(format!("{name}.rs")));
            declared.insert(dir.join(&name).join("mod.rs"));
        }
    }
    declared
}

/// Every path this file names from the crate root, whether it was
/// written as an import or spelled out where it is used.
///
/// A `use` and a `crate::…` in the body of a function are the same
/// coupling; one of them is just tidier. Both arrive here.
///
/// **One rule at every node, and the depth of a thing is not part of
/// it.** A file is a tree: an item holds items, a function body holds
/// items, and `#[cfg(test)]` can sit on any of them. Reading imports
/// at the top level while reading paths everywhere would answer two
/// different questions of one file — an import inside `mod inner`
/// would go unseen, and a fixture inside a nested `#[cfg(test)]` would
/// be measured as production. So the whole traversal is the visitor's,
/// and the visitor decides both things at each node it reaches.
fn names_from_crate_root(file: &File) -> BTreeSet<String> {
    let mut named = BTreeSet::new();
    let mut spelled = Spelled(&mut named);
    for item in &file.items {
        spelled.visit_item(item);
    }
    named
}

fn item_attrs(item: &Item) -> &[syn::Attribute] {
    match item {
        Item::Const(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::ExternCrate(i) => &i.attrs,
        Item::Fn(i) => &i.attrs,
        Item::ForeignMod(i) => &i.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Macro(i) => &i.attrs,
        Item::Mod(i) => &i.attrs,
        Item::Static(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::TraitAlias(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        Item::Union(i) => &i.attrs,
        Item::Use(i) => &i.attrs,
        // `Item` is `#[non_exhaustive]`, so this arm is required. What
        // reaches it is `Item::Verbatim` — tokens `syn` parsed as an
        // item without recognising the form — and whatever a later
        // release adds. Answering "no attributes" means such an item is
        // never treated as test-only, so it is read as production and
        // what it names is checked. That is the direction to fail in:
        // an unrecognised item is examined rather than waved through.
        _ => &[],
    }
}

/// One `use` tree, flattened into the names it brings in.
///
/// `pub use` is an `ItemUse` like any other and arrives here the same
/// way. A rename is recorded under the name it renames, because what
/// the forge reached for is the question and what it called the result
/// is not. A glob is recorded as the glob: it names whatever is behind
/// it, which is more than a list can hold.
fn expand_use(tree: &UseTree, prefix: &mut Vec<String>, into: &mut BTreeSet<String>) {
    match tree {
        UseTree::Path(node) => {
            prefix.push(node.ident.to_string());
            expand_use(&node.tree, prefix, into);
            prefix.pop();
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                expand_use(tree, prefix, into);
            }
        }
        UseTree::Name(node) => keep(prefix, &node.ident.to_string(), into),
        UseTree::Rename(node) => keep(prefix, &node.ident.to_string(), into),
        UseTree::Glob(_) => keep(prefix, "*", into),
    }
}

fn keep(prefix: &[String], last: &str, into: &mut BTreeSet<String>) {
    if prefix.first().is_none_or(|first| first != "crate") {
        return;
    }
    into.insert(format!("{}::{last}", prefix.join("::")));
}

/// What a file names from the crate root, gathered at every depth.
struct Spelled<'a>(&'a mut BTreeSet<String>);

impl<'ast> Visit<'ast> for Spelled<'_> {
    /// Every item, wherever it sits — at the file's top level, inside
    /// a `mod`, or inside a function body, which are the same thing to
    /// `syn` and should be the same thing here.
    fn visit_item(&mut self, item: &'ast Item) {
        if is_test_only(item_attrs(item)) {
            return;
        }
        if let Item::Use(item) = item {
            // A `use` tree holds no `syn::Path`, so there is nothing
            // below this to descend into.
            expand_use(&item.tree, &mut Vec::new(), self.0);
            return;
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let named: Vec<String> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        if named.first().is_some_and(|first| first == "crate") && named.len() > 1 {
            self.0.insert(named.join("::"));
        }
        syn::visit::visit_path(self, path);
    }
}

/// Is this name the forge talking to itself?
fn inside_the_forge(named: &str) -> bool {
    FORGE_ROOTS
        .iter()
        .any(|root| named == *root || named.starts_with(&format!("{root}::")))
}

/// Every name each production forge file reaches for outside the forge.
fn reaching_outside() -> BTreeMap<String, BTreeSet<String>> {
    let tests = test_files();
    let mut found = BTreeMap::new();
    for file in forge_files() {
        if tests.contains(&file) {
            continue;
        }
        let outside: BTreeSet<String> = names_from_crate_root(&parsed(&file))
            .into_iter()
            .filter(|named| !inside_the_forge(named))
            .collect();
        // Every file that was read gets a key, empty set and all. A
        // missing key then means "not measured" rather than "measured
        // and reaches for nothing", which is the difference
        // [`a_whole_file_of_tests_is_not_read_as_production_code`]
        // turns on — and without it that assertion would hold whether
        // the exclusion worked or not.
        found.insert(file.to_string_lossy().replace('\\', "/"), outside);
    }
    found
}

#[test]
fn the_forge_names_nothing_outside_itself_but_the_shared_vocabulary() {
    let allowed: BTreeSet<&str> = SHARED_VOCABULARY.iter().map(|(path, _)| *path).collect();

    let mut unexpected = Vec::new();
    for (file, named) in reaching_outside() {
        for named in named {
            if !allowed.contains(named.as_str()) {
                unexpected.push(format!("{file}: {named}"));
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
///
/// `crate::application::forge` is the case worth stating, because it is
/// the one that reads as harmless: the forge's own service, named from
/// the forge's own model. It is the layering violation exactly like any
/// other, and the version of this file that matched text could not see
/// it.
#[test]
fn the_forges_domain_does_not_reach_up_into_the_application() {
    let tests = test_files();
    let mut reaching = Vec::new();
    for file in forge_files() {
        let shown = file.to_string_lossy().replace('\\', "/");
        if tests.contains(&file) || !shown.starts_with("domain/") {
            continue;
        }
        for named in names_from_crate_root(&parsed(&file)) {
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

/// The file whose fixtures were being measured as production code.
///
/// `domain/forge/strategies/tests.rs` is a whole file of tests, brought
/// in by a `#[cfg(test)] mod tests;` one directory up. Nothing in the
/// file itself says so, so this asks whether the parent was read.
///
/// **What it does not claim.** That file reaches for nothing outside
/// the forge today, so excluding it changes no answer this guard
/// currently gives — it is a fixture that happens to be built out of
/// forge parts. Asserting "the exclusion caught something" would be
/// asserting the contents of a test file, which is free to change.
/// What is asserted is the mechanism: the file is known to be a test,
/// and it is not among the files measured.
#[test]
fn a_whole_file_of_tests_is_not_read_as_production_code() {
    let whole_file = Path::new("domain/forge/strategies/tests.rs");

    let tests = test_files();
    assert!(
        tests.contains(whole_file),
        "the parent's `#[cfg(test)] mod tests;` was not followed: {tests:#?}"
    );

    let measured = reaching_outside();
    assert!(
        !measured.contains_key(&whole_file.to_string_lossy().to_string()),
        "a test file was measured for what the forge depends on"
    );
    // The key is absent because the file was skipped, and not because
    // `reaching_outside` drops files that reach for nothing: it keys
    // every file it read. Counting says so — one key per forge file
    // that is not a test, whether it names anything outside or not.
    assert_eq!(
        measured.len(),
        forge_files().len()
            - tests
                .iter()
                .filter(|path| src().join(path).exists())
                .count(),
        "every forge file that is not a test is measured"
    );
}
