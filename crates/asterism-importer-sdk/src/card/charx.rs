//! `.charx` — a V3 card and its assets inside one ZIP.
//!
//! The third container for the same card, beside the PNG tEXt chunk and
//! the standalone `.json`. What it adds is that the assets travel with
//! it: `card.json` at the root, and the pictures the card refers to
//! under `assets/<type>/<category>/<file>` (the layout is written down
//! in [`crate::catalogue`], V3 section).
//!
//! **The entries are addressed, not extracted.** This module reads
//! `card.json` out of the archive and lists what else is in there; the
//! footprints it feeds carry `<container>#<entry>` locators, which the
//! domain reads back as a
//! `SourceLocator::Record` — a container plus an address its reader
//! resolves. Nothing is written to disk, so there is no second copy to
//! keep in step with the first.
//!
//! Only the card route reaches this. An archive is not opened because
//! it is an archive: a plain `.zip` is one asset that states its
//! format, and nothing looks inside it.

use std::io::{Cursor, Read};

use super::envelope::CardEnvelope;

/// Where a `.charx` keeps the card itself. Fixed by the V3 spec.
pub const CARD_JSON: &str = "card.json";

/// The scheme a V3 `assets[].uri` uses for something inside the
/// container. Spelled with one `d` in the spec, which is what emitters
/// write, so it is what this reads.
pub const EMBEDDED_SCHEME: &str = "embeded://";

/// Local file header magic — the first four bytes of every non-empty
/// ZIP. An empty archive (`PK\x05\x06`) holds no `card.json` and so is
/// not a card either way, which is why the one signature is enough to
/// route on.
const LOCAL_FILE_HEADER: &[u8; 4] = b"PK\x03\x04";

/// Whether a payload opens like a ZIP.
///
/// A cheap route decision, not a validity claim: [`read`] is what says
/// whether the archive holds a card.
pub fn is_zip(payload: &[u8]) -> bool {
    payload.starts_with(LOCAL_FILE_HEADER)
}

/// What one `.charx` holds: the card, and the names of everything else
/// inside it.
#[derive(Debug, Clone)]
pub struct Charx {
    /// The parsed `card.json`.
    pub envelope: CardEnvelope,
    /// Every other entry's name, in archive order, directories dropped.
    ///
    /// Names rather than bytes: an `assets[]` URI is resolved against
    /// this list so that a footprint is only ever addressed at an entry
    /// the archive really holds, and the bytes are read later by
    /// whoever needs them.
    pub entries: Vec<String>,
}

/// Read a `.charx`: the card at [`CARD_JSON`], and the names of the
/// other entries.
///
/// `None` when the payload is not a ZIP, holds no `card.json`, or holds
/// one that is not a card envelope — the same "unrecognised shape, skip
/// it" answer the PNG and JSON routes give, rather than an error that
/// would fail the batch around it.
pub fn read(payload: &[u8]) -> Option<Charx> {
    if !is_zip(payload) {
        return None;
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(payload)).ok()?;

    let mut card = String::new();
    archive
        .by_name(CARD_JSON)
        .ok()?
        .read_to_string(&mut card)
        .ok()?;
    let envelope = CardEnvelope::from_json(serde_json::from_str(&card).ok()?)?;

    let entries = archive
        .file_names()
        .filter(|name| *name != CARD_JSON && !name.ends_with('/'))
        .map(str::to_string)
        .collect();

    Some(Charx { envelope, entries })
}

/// The entry an `assets[]` URI names, when it names one inside this
/// container.
///
/// Returns the entry name as the archive spells it, so a caller can use
/// it as a record address directly. `None` for a URI that points
/// somewhere else (`http://`, a bare path, a `ccdefault:` sentinel) and
/// for one that names an entry this archive does not hold — a card may
/// refer to a picture that was never packed, and addressing that would
/// mint a locator with nothing behind it.
pub fn entry_for_uri<'a>(uri: &str, entries: &'a [String]) -> Option<&'a str> {
    let path = uri.strip_prefix(EMBEDDED_SCHEME)?;
    entries
        .iter()
        .find(|entry| entry.as_str() == path)
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    /// Builds a `.charx` in memory: `card.json` plus whatever else is
    /// asked for. The tests below are about what comes back out, so the
    /// archive has to be real rather than a fixture nobody can read.
    fn charx(card: &serde_json::Value, files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default();
            zip.start_file(CARD_JSON, opts).unwrap();
            zip.write_all(card.to_string().as_bytes()).unwrap();
            for (name, bytes) in files {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
        }
        buf
    }

    fn v3_card() -> serde_json::Value {
        json!({
            "spec": "chara_card_v3",
            "spec_version": "3.0",
            "data": { "name": "Alice", "first_mes": "hi" }
        })
    }

    #[test]
    fn a_png_is_not_routed_here() {
        assert!(!is_zip(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_zip(b"{\"spec\":\"chara_card_v3\"}"));
        assert!(is_zip(b"PK\x03\x04rest"));
    }

    #[test]
    fn the_card_comes_out_and_the_assets_are_listed_beside_it() {
        let bytes = charx(
            &v3_card(),
            &[
                ("assets/icon/images/main.png", b"\x89PNG"),
                ("assets/emotion/images/joy.png", b"\x89PNG"),
            ],
        );

        let charx = read(&bytes).expect("a card.json makes this a card");
        assert_eq!(charx.envelope.spec, "chara_card_v3");
        assert_eq!(
            charx.entries,
            vec![
                "assets/icon/images/main.png".to_string(),
                "assets/emotion/images/joy.png".to_string(),
            ],
            "card.json is the card, not one of the things beside it"
        );
    }

    #[test]
    fn an_archive_without_a_card_is_not_a_card() {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
            zip.start_file("notes.txt", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"nothing to do with cards").unwrap();
            zip.finish().unwrap();
        }
        assert!(read(&buf).is_none());
    }

    /// The one that keeps a locator honest: a card is free to name a
    /// picture the packer left out, and a footprint addressed at it
    /// would point into empty space.
    #[test]
    fn a_uri_naming_an_entry_that_was_never_packed_resolves_to_nothing() {
        let entries = vec!["assets/icon/images/main.png".to_string()];

        assert_eq!(
            entry_for_uri("embeded://assets/icon/images/main.png", &entries),
            Some("assets/icon/images/main.png")
        );
        assert_eq!(
            entry_for_uri("embeded://assets/emotion/images/joy.png", &entries),
            None,
            "the card refers to it; the archive does not hold it"
        );
        assert_eq!(
            entry_for_uri("https://example.com/main.png", &entries),
            None,
            "a URI that points outside the container is not an entry"
        );
        assert_eq!(entry_for_uri("ccdefault:", &entries), None);
    }
}
