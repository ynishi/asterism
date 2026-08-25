//! Counts the desktop write surface, from inside the crate that owns it.
//!
//! This guard lived in `asterism-core`'s `attribution_guards.rs` until
//! #159: it reads this crate's `src/commands.rs`, but the `-changed`
//! gates run a crate's tests only when that crate changes, so a pull
//! request that grew the surface never ran the guard, and the failure
//! surfaced one merge later on `main`'s run (#154). A test that reads a
//! file belongs to the crate that owns the file — sitting here, it runs
//! in the same pull request that moves its subject.

use std::fs;
use std::path::Path;

/// How many `#[tauri::command]` functions call a service mutation.
///
/// The desktop surface has no single point every write passes through
/// — the commands are a flat list, each calling its service directly.
/// The number is what stands in for that missing point: it does not
/// prevent anything, it makes a change in the size of the write
/// surface something somebody had to type.
///
/// Counted from the source, so it moves when a mutation command is
/// added or removed and not otherwise. Adding a read command leaves it
/// alone.
const TAURI_MUTATION_COMMANDS: usize = 103;

#[test]
fn the_tauri_mutation_surface_is_the_size_it_records() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands.rs");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect();

    let mut total = 0usize;
    let mut mutations = 0usize;
    let mut index = 0usize;
    while index < lines.len() {
        if lines[index].trim() != "#[tauri::command]" {
            index += 1;
            continue;
        }
        total += 1;
        let mut body = String::new();
        let mut cursor = index + 1;
        while cursor < lines.len() {
            let text = lines[cursor];
            body.push_str(text);
            body.push('\n');
            if text == "}" {
                break;
            }
            cursor += 1;
        }
        if body.contains("AttributionContext::owner_surface()") {
            mutations += 1;
        }
        index = cursor + 1;
    }

    assert!(
        total > 0,
        "no `#[tauri::command]` found — the scan is not reading commands.rs"
    );
    assert_eq!(
        mutations, TAURI_MUTATION_COMMANDS,
        "the desktop write surface changed size: {mutations} of {total} \
         commands now name the owner's surface. If a mutation was added \
         or removed, update TAURI_MUTATION_COMMANDS in the same diff. If \
         the count fell because a mutation stopped passing a context, \
         that is the change to look at"
    );
}
