//! Integration test over the whole `card` subsystem, driven by a
//! committed dual-chunk PNG (`character-card-lyra.png`: `chara` = V2,
//! `ccv3` = V3, four-entry `character_book`).
//!
//! # Where the fixture comes from
//!
//! `scripts/gen-test-fixtures.py` writes it — pixels, both tEXt chunks
//! and the card JSON inside them. Regenerate with
//! `python3 scripts/gen-test-fixtures.py`.
//!
//! This test used to run against SillyTavern's `default_Seraphina.png`,
//! on the stated reasoning that a real-world card exercises the parser
//! more honestly than a hand-crafted one. Two things were wrong with
//! that: the file is AGPL-3.0 and was committed here despite this doc
//! claiming it was not, and "real-world" was doing no work the fixture
//! below does not — what the assertions check is the *composition* (one
//! Note per filled slot, greetings as Assistant messages, one grouping
//! key, unique locators), and a synthetic card fills all six V2 slots
//! deliberately rather than by luck.

use std::collections::BTreeMap;
use std::path::PathBuf;

use asterism_importer_sdk::card::{
    CCV3_KEYWORD, CHARA_KEYWORD, CardContext, CardParserRegistry, envelope_from_chunk,
};
use asterism_importer_sdk::{ChatRole, Footprint};
use chrono::Utc;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/character-card-lyra.png")
}

/// Minimal inline PNG tEXt chunk walker, independent of the one the
/// SDK uses.
///
/// It was originally here to keep a `pngmeta` dev-dep off the crate.
/// That reason expired — the crate has one now — and the walker stays
/// for the reason that outlived it: this test reads the shipped
/// fixture to say what the card parser *should* have found, so it must
/// not share a walker with the parser it is checking. Reaching for
/// `pngmeta` here would make a fault in the chunk walk invisible on
/// both sides of the assertion at once.
///
/// Returns a `BTreeMap<keyword, text>` for every `tEXt` chunk in the
/// file. tEXt text is Latin-1 per spec; character cards keep the
/// payload in the ASCII base64 subset so `String::from_utf8` succeeds.
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
        cursor = data_end + 4; // skip CRC
        if ctype == b"IEND" {
            break;
        }
    }
    chunks
}

fn load_fixture() -> Vec<u8> {
    let path = fixture_path();
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "fixture missing at {} ({e}). Regenerate with:\n\
             \x20\x20\x20\x20python3 scripts/gen-test-fixtures.py",
            path.display(),
        )
    })
}

fn ctx() -> CardContext<'static> {
    CardContext {
        source_kind: "chara",
        locator: "character-card-lyra.png",
        session_id: "sess-card",
        occurred_at: Utc::now(),
        platform: Some("SillyTavern"),
    }
}

#[test]
fn png_has_dual_chara_and_ccv3_chunks() {
    let png = load_fixture();
    let chunks = read_png_text_chunks(&png);
    assert!(
        chunks.contains_key(CHARA_KEYWORD),
        "card PNG must carry a chara chunk"
    );
    assert!(
        chunks.contains_key(CCV3_KEYWORD),
        "card PNG must carry a ccv3 chunk (V3 dual)"
    );
}

#[test]
fn v2_chunk_decodes_to_envelope() {
    let png = load_fixture();
    let chunks = read_png_text_chunks(&png);
    let env = envelope_from_chunk(&chunks[CHARA_KEYWORD]).expect("chara → envelope");
    assert_eq!(env.spec, "chara_card_v2");
    assert_eq!(env.spec_version, "2.0");
    assert_eq!(env.data.get("name").and_then(|v| v.as_str()), Some("Lyra"));
}

#[test]
fn v3_chunk_decodes_to_envelope() {
    let png = load_fixture();
    let chunks = read_png_text_chunks(&png);
    let env = envelope_from_chunk(&chunks[CCV3_KEYWORD]).expect("ccv3 → envelope");
    assert_eq!(env.spec, "chara_card_v3");
    assert_eq!(env.data.get("name").and_then(|v| v.as_str()), Some("Lyra"));
}

#[test]
fn v2_dispatch_produces_full_footprint_composition() {
    let png = load_fixture();
    let chunks = read_png_text_chunks(&png);
    let env = envelope_from_chunk(&chunks[CHARA_KEYWORD]).unwrap();
    let reg = CardParserRegistry::with_defaults();
    let out = reg.dispatch(&env, &ctx()).expect("V2Parser dispatches");

    // Shape counts — the fixture fills all six V2 text slots and
    // carries a 4-entry character_book (catalogue section 1).
    let notes = out
        .iter()
        .filter(|f| matches!(f, Footprint::Note(_)))
        .count();
    let msgs = out
        .iter()
        .filter(|f| matches!(f, Footprint::ChatMessage(_)))
        .count();
    let docs = out
        .iter()
        .filter(|f| matches!(f, Footprint::Doc(_)))
        .count();

    // Exact, not a floor. The old fixture came from upstream and could
    // gain a book entry between releases, so the counts were written as
    // `>=` — which passes just as happily when a slot silently stops
    // emitting. This repository owns the fixture now: 6 text slots + 4
    // book entries = 10 Notes, first_mes + 2 alternates = 3 messages,
    // creator_notes + mes_example = 2 docs. Change the card in
    // `gen-test-fixtures.py` and these numbers move with it.
    assert_eq!(notes, 10, "6 text slots + 4 book entries");
    assert_eq!(msgs, 3, "first_mes + 2 alternate_greetings");
    assert_eq!(docs, 2, "creator_notes + mes_example");

    // Every emitted ChatMessage is Assistant-role (character speaks).
    for f in &out {
        if let Footprint::ChatMessage(m) = f {
            assert_eq!(m.role, ChatRole::Assistant, "greetings are Assistant");
        }
    }
}

#[test]
fn v2_all_footprints_share_grouping_key() {
    let png = load_fixture();
    let chunks = read_png_text_chunks(&png);
    let env = envelope_from_chunk(&chunks[CHARA_KEYWORD]).unwrap();
    let reg = CardParserRegistry::with_defaults();
    let out = reg.dispatch(&env, &ctx()).unwrap();

    // Every footprint from one card shares one grouping key so
    // edge_rebuild draws `same-bundle` across siblings (catalogue
    // axiom 3). After the P3 session-model refactor the field the
    // key lands on depends on the variant: `external_session_key`
    // for the Dialog-only `ChatMessage`, `bundle_id` for everything
    // else. The underlying string stays constant.
    for f in &out {
        let key = match f {
            Footprint::Note(n) => n.bundle_id.as_deref(),
            Footprint::ChatMessage(m) => Some(m.external_session_key.as_str()),
            Footprint::Doc(d) => d.bundle_id.as_deref(),
            Footprint::Image(i) => i.bundle_id.as_deref(),
            Footprint::JournalEntry(j) => j.bundle_id.as_deref(),
            Footprint::Video(v) => v.bundle_id.as_deref(),
            Footprint::Audio(a) => a.bundle_id.as_deref(),
            Footprint::Tape(t) => t.bundle_id.as_deref(),
        };
        assert_eq!(key, Some("sess-card"));
    }
}

#[test]
fn v2_locators_are_unique_per_footprint() {
    let png = load_fixture();
    let chunks = read_png_text_chunks(&png);
    let env = envelope_from_chunk(&chunks[CHARA_KEYWORD]).unwrap();
    let reg = CardParserRegistry::with_defaults();
    let out = reg.dispatch(&env, &ctx()).unwrap();

    // Idempotency contract: `(source_kind, source_locator)` must be
    // unique per footprint — otherwise re-imports would collide.
    let mut seen = std::collections::HashSet::new();
    for f in &out {
        let locator = match f {
            Footprint::Note(n) => &n.source.locator,
            Footprint::ChatMessage(m) => &m.source.locator,
            Footprint::Doc(d) => &d.source.locator,
            Footprint::Image(i) => &i.source.locator,
            Footprint::JournalEntry(j) => &j.source.locator,
            Footprint::Video(v) => &v.source.locator,
            Footprint::Audio(a) => &a.source.locator,
            Footprint::Tape(t) => &t.source.locator,
        };
        assert!(seen.insert(locator.clone()), "duplicate locator: {locator}");
        assert!(
            locator.starts_with("character-card-lyra.png#"),
            "every locator must be a per-record suffix of the container: {locator}"
        );
    }
}

#[test]
fn v3_dispatch_extends_v2_composition() {
    let png = load_fixture();
    let chunks = read_png_text_chunks(&png);
    let env = envelope_from_chunk(&chunks[CCV3_KEYWORD]).unwrap();
    let reg = CardParserRegistry::with_defaults();
    let out = reg.dispatch(&env, &ctx()).expect("V3Parser dispatches");

    // V3 must produce at least as much as V2 (same slot base) —
    // and typically more when assets[] / creator_notes_multilingual /
    // group_only_greetings are populated.
    assert!(
        out.iter().any(|f| matches!(f, Footprint::Note(_))),
        "V3 emits Notes"
    );
    assert!(
        out.iter().any(|f| matches!(f, Footprint::ChatMessage(_))),
        "V3 emits ChatMessages"
    );

    // Every footprint carries the V3 spec label so downstream can
    // distinguish V3 output from V2 output on the same card.
    for f in &out {
        let labels: &[String] = match f {
            Footprint::Note(n) => &n.labels,
            Footprint::ChatMessage(m) => &m.labels,
            Footprint::Doc(d) => &d.labels,
            Footprint::Image(i) => &i.labels,
            Footprint::JournalEntry(j) => &j.labels,
            Footprint::Video(v) => &v.labels,
            Footprint::Audio(a) => &a.labels,
            Footprint::Tape(t) => &t.labels,
        };
        assert!(
            labels.iter().any(|l| l == "chara_card_v3"),
            "V3 label missing from footprint"
        );
    }
}
