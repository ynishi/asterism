//! Run:
//! ```sh
//! cargo run -p asterism-importer-sdk --example inspect_character_card
//! ```
//!
//! Reads the committed character-card fixture, decodes both `chara`
//! (V2) and `ccv3` (V3) tEXt chunks, dispatches each through
//! `CardParserRegistry::with_defaults()`, and prints a per-footprint
//! summary so a human can eyeball what the parser produced without
//! wading through JSON.

use std::collections::BTreeMap;
use std::path::PathBuf;

use asterism_importer_sdk::Footprint;
use asterism_importer_sdk::card::{
    CCV3_KEYWORD, CHARA_KEYWORD, CardContext, CardEnvelope, CardParserRegistry, envelope_from_chunk,
};
use chrono::Utc;

fn main() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/character-card-lyra.png");
    let bytes =
        std::fs::read(&fixture).unwrap_or_else(|e| panic!("read {}: {e}", fixture.display()));
    let chunks = read_png_text_chunks(&bytes);

    println!("== fixture ==");
    println!("path: {}", fixture.display());
    println!("size: {} bytes", bytes.len());
    println!("tEXt chunks: {:?}", chunks.keys().collect::<Vec<_>>());

    for keyword in [CHARA_KEYWORD, CCV3_KEYWORD] {
        let Some(raw) = chunks.get(keyword) else {
            continue;
        };
        let Some(env) = envelope_from_chunk(raw) else {
            eprintln!("(skipping {keyword}: envelope decode failed)");
            continue;
        };
        report(keyword, &env);
    }
}

fn report(keyword: &str, env: &CardEnvelope) {
    println!(
        "\n== {} chunk / spec={} v{} ==",
        keyword, env.spec, env.spec_version
    );
    let name = env.data.get("name").and_then(|v| v.as_str()).unwrap_or("?");
    println!("card.name: {name}");
    if let Some(book) = env.data.get("character_book").and_then(|v| v.as_object()) {
        let count = book
            .get("entries")
            .and_then(|v| v.as_array().map(|a| a.len()))
            .unwrap_or(0);
        println!("character_book.entries: {count}");
    }

    let ctx = CardContext {
        source_kind: "chara",
        locator: "character-card-lyra.png",
        session_id: "sess-card",
        occurred_at: Utc::now(),
        platform: Some("SillyTavern"),
    };
    let reg = CardParserRegistry::with_defaults();
    let out = reg.dispatch(env, &ctx).expect("registry claims this spec");

    let (mut n_note, mut n_msg, mut n_doc, mut n_img, mut n_jrn, mut n_vid, mut n_aud, mut n_tpe) =
        (0, 0, 0, 0, 0, 0, 0, 0);
    for f in &out {
        match f {
            Footprint::Note(_) => n_note += 1,
            Footprint::ChatMessage(_) => n_msg += 1,
            Footprint::Doc(_) => n_doc += 1,
            Footprint::Image(_) => n_img += 1,
            Footprint::JournalEntry(_) => n_jrn += 1,
            Footprint::Video(_) => n_vid += 1,
            Footprint::Audio(_) => n_aud += 1,
            Footprint::Tape(_) => n_tpe += 1,
        }
    }
    println!(
        "footprints: {} total ({} Note, {} ChatMessage, {} Doc, {} Image, {} JournalEntry, {} Video, {} Audio, {} Tape)",
        out.len(),
        n_note,
        n_msg,
        n_doc,
        n_img,
        n_jrn,
        n_vid,
        n_aud,
        n_tpe
    );

    println!("--- per-footprint ---");
    for f in &out {
        let (kind, locator, first_label, preview) = match f {
            Footprint::Note(n) => (
                "Note",
                n.source.locator.as_str(),
                n.labels.get(1).cloned().unwrap_or_default(),
                preview(&n.body),
            ),
            Footprint::ChatMessage(m) => (
                "ChatMessage",
                m.source.locator.as_str(),
                m.labels.get(1).cloned().unwrap_or_default(),
                preview(&m.body),
            ),
            Footprint::Doc(d) => (
                "Doc",
                d.source.locator.as_str(),
                d.labels.get(1).cloned().unwrap_or_default(),
                preview(&d.excerpt),
            ),
            Footprint::Image(i) => (
                "Image",
                i.source.locator.as_str(),
                i.labels.get(1).cloned().unwrap_or_default(),
                i.alt.clone().unwrap_or_default(),
            ),
            Footprint::JournalEntry(j) => (
                "JournalEntry",
                j.source.locator.as_str(),
                j.labels.first().cloned().unwrap_or_default(),
                preview(&j.body),
            ),
            Footprint::Video(v) => (
                "Video",
                v.source.locator.as_str(),
                v.labels.first().cloned().unwrap_or_default(),
                v.alt.clone().unwrap_or_default(),
            ),
            Footprint::Audio(a) => (
                "Audio",
                a.source.locator.as_str(),
                a.labels.first().cloned().unwrap_or_default(),
                a.alt.clone().unwrap_or_default(),
            ),
            Footprint::Tape(t) => (
                "Tape",
                t.source.locator.as_str(),
                t.labels.first().cloned().unwrap_or_default(),
                preview(&t.excerpt),
            ),
        };
        // Strip the common container prefix so the table is scannable.
        let suffix = locator
            .strip_prefix("character-card-lyra.png#")
            .unwrap_or(locator);
        println!("  [{kind:12}] {suffix:44} {first_label:20}  {preview}");
    }
}

fn preview(text: &str) -> String {
    let one_line = text.replace('\n', " ").trim().to_string();
    if one_line.chars().count() > 60 {
        one_line.chars().take(60).collect::<String>() + "…"
    } else {
        one_line
    }
}

/// Inline PNG tEXt chunk walker — duplicated from
/// `tests/character_card.rs` so this example stays self-contained.
fn read_png_text_chunks(bytes: &[u8]) -> BTreeMap<String, String> {
    const SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let mut chunks = BTreeMap::new();
    if bytes.len() < SIG.len() || &bytes[..8] != SIG {
        return chunks;
    }
    let mut cursor = 8usize;
    while cursor + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        let ctype = &bytes[cursor + 4..cursor + 8];
        let data_start = cursor + 8;
        let data_end = data_start + len;
        if data_end + 4 > bytes.len() {
            break;
        }
        if ctype == b"tEXt" {
            let data = &bytes[data_start..data_end];
            if let Some(nul) = data.iter().position(|&b| b == 0) {
                let keyword = String::from_utf8_lossy(&data[..nul]).into_owned();
                let text = String::from_utf8_lossy(&data[nul + 1..]).into_owned();
                chunks.insert(keyword, text);
            }
        }
        cursor = data_end + 4;
        if ctype == b"IEND" {
            break;
        }
    }
    chunks
}
