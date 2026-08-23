//! A repository says what storage did. It does not say what that means
//! to a caller.
//!
//! `DomainError`'s `Conflict` carries a `ConflictKind`, which is advice
//! a client acts on — whether asking again is worth anything. A SQLite
//! repository cannot answer that. It knows a unique index rejected the
//! row, or a predicate matched nothing; whether the caller should retry
//! depends on what the request meant, which is a layer up.
//!
//! # What this is here to stop happening again
//!
//! It had happened. `DomainError`'s four shared variants had no written
//! rule for which one a refusal belongs to, so the choice was made at
//! each site that raised one — fifty-eight, thirty-nine of them in this
//! crate. Several were wrong in ways that reached a client: one
//! situation answered `400` through a service and `409` through the
//! port beneath it, while the port's own doc said the two were the same
//! refusal.
//!
//! The fix was to give this crate its own vocabulary — `crate::fault`'s
//! `StoreFault` — and one hand-written conversion whose RustDoc is the
//! specification. That only holds while the conversion is the sole way
//! through, and nothing about a `use` statement makes it so. This is
//! what makes it so.
//!
//! # What this covers, and what it does not
//!
//! `Conflict` only. `Validation`, `NotFound` and `Infra` are still
//! named directly all over this crate, and deliberately: a repository
//! reporting that a row is absent, or that a stored value would not
//! decode, is not answering a question about retry advice. What made
//! `Conflict` different is [`ConflictKind`] — the one variant carrying
//! a promise a client acts on.
//!
//! # Why it parses
//!
//! The first version of this cut each file at its first `#[cfg(test)]`
//! and read what came before. That is wrong here, and badly: several
//! files in this crate put a `#[cfg(test)] const` near the top, so
//! `sqlite/repo/asset.rs` was read to line 618 of 15,383 and the
//! repository itself never scanned. Measured across the crate, it saw
//! 32,324 of 75,247 lines and reported green.
//!
//! It is the same defect `asterism-core`'s `forge_boundary.rs` had and
//! for the same reason — a guard deciding what is test code by looking
//! at text — and it took the same fix. Which items are test-only is a
//! question about syntax, and `syn` answers it.

use std::fs;
use std::path::{Path, PathBuf};

use syn::{Item, visit::Visit};

/// The ways a caller-facing conflict can be spelled.
///
/// `Conflict` itself, and the four constructors that build one. A
/// repository reaching for any of them is answering the question this
/// crate does not get to answer.
const NOT_THIS_CRATE_S_TO_SAY: &[&str] = &[
    "Conflict", "conflict", "raced", "blocked", "settled", "clashes",
];

/// Where the conversion itself lives, which is the one file that must
/// name them.
const THE_CONVERSION: &str = "fault.rs";

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn walk(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("a readable directory") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() {
            walk(&path, into);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            into.push(path);
        }
    }
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
        // `Item` is `#[non_exhaustive]`. Answering "no attributes"
        // means an unrecognised item is read as production and its
        // paths are checked, which is the direction to fail in.
        _ => &[],
    }
}

/// Every `DomainError::…` this file names in production code.
///
/// One rule at every node: an item holds items, a function body holds
/// items, and `#[cfg(test)]` sits on any of them. Reading only the top
/// level would miss a construction inside a `mod`, and cutting at the
/// first attribute misses everything after a `#[cfg(test)] const`.
struct Naming<'a> {
    found: &'a mut Vec<String>,
    file: &'a str,
}

impl<'ast> Visit<'ast> for Naming<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if is_test_only(item_attrs(item)) {
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
        // `DomainError::X` and `X` where the enum was imported by
        // variant. The first segment being the type is the ordinary
        // spelling; the last segment carries the answer either way.
        if let Some(last) = named.last()
            && named.len() >= 2
            && named[named.len() - 2] == "DomainError"
            && NOT_THIS_CRATE_S_TO_SAY.contains(&last.as_str())
        {
            self.found
                .push(format!("{}: DomainError::{last}", self.file));
        }
        syn::visit::visit_path(self, path);
    }
}

#[test]
fn no_repository_decides_what_a_conflict_means() {
    let mut files = Vec::new();
    walk(&src(), &mut files);

    let mut saying = Vec::new();
    for file in files {
        let shown = file
            .strip_prefix(src())
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        if shown.ends_with(THE_CONVERSION) {
            continue;
        }
        let text = fs::read_to_string(&file).expect("a readable file");
        let parsed =
            syn::parse_file(&text).unwrap_or_else(|err| panic!("{shown} does not parse: {err}"));
        let mut naming = Naming {
            found: &mut saying,
            file: &shown,
        };
        for item in &parsed.items {
            naming.visit_item(item);
        }
    }

    assert!(
        saying.is_empty(),
        "a repository is deciding what a conflict means to a caller, which \
         is `crate::fault`'s table to decide. Raise the `StoreFault` that \
         says what storage did and let the conversion answer: {saying:#?}"
    );
}

/// The conversion is where they are named, so the check above is
/// checking something.
#[test]
fn the_conversion_still_names_them() {
    let text = fs::read_to_string(src().join(THE_CONVERSION)).expect("the conversion");

    for spelling in ["DomainError::conflict", "DomainError::Validation"] {
        assert!(
            text.contains(spelling),
            "{THE_CONVERSION} no longer names {spelling}, so the exemption \
             above covers a file that does not need it"
        );
    }
}

/// The scan reaches the whole of a file, not the part before its first
/// `#[cfg(test)]`.
///
/// This is the assertion the first version of this guard would have
/// failed. `sqlite/repo/asset.rs` carries a `#[cfg(test)] const` near
/// the top and fifteen thousand lines after it, so a guard that cut
/// there read four per cent of the file and called it clean — and no
/// ablation in a file without such a `const` could show it.
#[test]
fn a_cfg_test_const_does_not_hide_the_rest_of_a_file() {
    let asset = src().join("sqlite/repo/asset.rs");
    let text = fs::read_to_string(&asset).expect("the asset repository");
    let parsed = syn::parse_file(&text).expect("it parses");

    let cut = text
        .find("#[cfg(test)]")
        .expect("the file has a #[cfg(test)] to be cut at");
    let before = text[..cut].lines().count();
    assert!(
        before < text.lines().count() / 2,
        "this file no longer has an early `#[cfg(test)]`, so it cannot show \
         what the text-cutting version missed — point this at one that does"
    );

    // What the parse actually reaches: the repository impl, which lives
    // far past that point.
    let mut items = 0usize;
    for item in &parsed.items {
        if !is_test_only(item_attrs(item)) {
            items += 1;
        }
    }
    assert!(
        items > 1,
        "the parse sees more than what precedes the first `#[cfg(test)]`"
    );
    assert!(
        parsed.items.iter().any(|item| matches!(
            item,
            Item::Impl(i) if !is_test_only(&i.attrs)
                && i.trait_.as_ref().is_some_and(|(_, path, _)| {
                    path.segments.last().is_some_and(|s| s.ident == "AssetRepository")
                })
        )),
        "the repository impl itself is scanned"
    );
}
