//! The bytes a locator addresses, when they are on this machine.
//!
//! Every job that decodes, measures or hashes an original asks the same
//! two questions in the same order, and the answers are not the same
//! kind of thing: *are there bytes here at all* decides whether the row
//! is retired, and *could they be read just now* decides whether it is
//! left for a later pass. Recording a temporary failure permanently is
//! the dims-walk mistake those jobs each carry a comment about.
//!
//! [`read`] answers both in one value, so the two stay in the order
//! that keeps them apart:
//!
//! - `None` — nothing here will ever have bytes. A remote locator, a
//!   caller-minted name, or a record inside a container nothing opens.
//! - `Some(Err(_))` — bytes that should be here and were not readable
//!   this time.
//! - `Some(Ok(bytes))` — the bytes.
//!
//! ## Which containers open, and which of their records
//!
//! A [`Record`](SourceLocator::Record) is a container plus an address
//! inside it, and most such records have no bytes of their own.
//! `ContainerRecord::holds_its_own_bytes` is the question, and its
//! docstring says which shapes answer yes and why the container is not
//! opened on the others' behalf.
//!
//! A shape answering `true` there is not the whole test. A card
//! archive addresses two kinds of thing with one spelling: the entries
//! it packs (`#assets/icon/images/main.png`) and the slots the card
//! states (`#field=name`), and only the first names bytes. Which is
//! which is a question only the archive can answer, so this module
//! asks it — an address the archive does not hold reads as `None`,
//! the permanent answer, rather than as a read that failed. Retrying a
//! slot suffix on every backfill pass, forever, is the walk that never
//! shrinks.
//!
//! ## The ceiling
//!
//! An entry states its own uncompressed length and the file it sits in
//! was written by somebody else, so the length is a claim rather than a
//! fact. [`MAX_ENTRY_BYTES`] is the ceiling the read is held to, and an
//! entry over it is refused rather than allocated for — the same
//! judgement `fingerprint::hash_artefact` makes about a file it is
//! asked to hold whole.

use std::io;
use std::path::Path;

use asterism_core::domain::source_locator::SourceLocator;

/// The most an entry is read into memory, at 64 MiB.
///
/// The same ceiling the content walk holds a file to
/// (`MAX_CONTENT_WALK_BYTES`), and for the same reason: the buffer is
/// this process's memory, and what is on the other side of the number
/// is a picture nobody put in a character card.
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// The bytes this locator addresses.
///
/// See the module docstring for what each of the three answers means to
/// a caller; the ordering (`None` before `Err`) is the whole point of
/// returning them together.
pub async fn read(locator: &SourceLocator) -> Option<io::Result<Vec<u8>>> {
    match locator {
        SourceLocator::File(path) => Some(tokio::fs::read(path.as_path()).await),
        SourceLocator::Record(record) if record.holds_its_own_bytes() => {
            let container = record.container().as_path().to_path_buf();
            let entry = record.record().as_str().to_string();
            // Blocking: `zip` seeks the file it is handed, and the
            // entry is inflated before it is returned.
            match tokio::task::spawn_blocking(move || entry_bytes(&container, &entry)).await {
                Ok(read) => read,
                Err(join) => Some(Err(io::Error::other(join))),
            }
        }
        SourceLocator::Record(_) | SourceLocator::Remote(_) | SourceLocator::Logical(_) => None,
    }
}

/// Opens the archive and inflates one entry.
///
/// `None` when the archive holds no such entry: the address is a slot
/// the card states rather than a file it packs, or a picture that was
/// never packed, and neither becomes bytes on a later pass. An archive
/// that cannot be opened at all is `Some(Err(_))` — that one is a
/// disk saying no, which is a different sentence.
fn entry_bytes(container: &Path, entry: &str) -> Option<io::Result<Vec<u8>>> {
    entry_bytes_within(container, entry, MAX_ENTRY_BYTES)
}

/// [`entry_bytes`] with the ceiling as an argument, so a test can put a
/// real entry on the far side of it without writing 64 MiB — the shape
/// `fingerprint::hash_artefact` takes `max_walk` in, and for the same
/// reason.
fn entry_bytes_within(container: &Path, entry: &str, ceiling: u64) -> Option<io::Result<Vec<u8>>> {
    use std::io::Read as _;

    let file = match std::fs::File::open(container) {
        Ok(file) => file,
        Err(err) => return Some(Err(err)),
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(err) => return Some(Err(io::Error::other(err))),
    };
    let mut member = match archive.by_name(entry) {
        Ok(member) => member,
        Err(zip::result::ZipError::FileNotFound) => return None,
        Err(err) => return Some(Err(io::Error::other(err))),
    };
    if member.size() > ceiling {
        return Some(Err(io::Error::other(format!(
            "entry {entry:?} states {} bytes, over the {ceiling}-byte ceiling",
            member.size()
        ))));
    }
    // Read under the ceiling rather than to the length the entry
    // claims: the claim is the archive's, and a wrong one would be this
    // process's memory.
    let mut bytes = Vec::new();
    match member.by_ref().take(ceiling + 1).read_to_end(&mut bytes) {
        Ok(_) if bytes.len() as u64 > ceiling => Some(Err(io::Error::other(format!(
            "entry {entry:?} runs past the {ceiling}-byte ceiling"
        )))),
        Ok(_) => Some(Ok(bytes)),
        Err(err) => Some(Err(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::source_locator::{ContainerRecord, LocalPath, RecordAddress};
    use std::io::Write as _;

    fn charx_at(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn record_at(container: &Path, entry: &str) -> SourceLocator {
        ContainerRecord::new(
            LocalPath::try_from(container.to_str().unwrap()).unwrap(),
            RecordAddress::try_from(entry).unwrap(),
        )
        .into()
    }

    #[tokio::test]
    async fn an_entry_inside_a_card_archive_reads_its_own_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let card = dir.path().join("lyra.charx");
        charx_at(
            &card,
            &[
                ("card.json", b"{}"),
                ("assets/icon/images/main.png", b"\x89PNG-icon"),
                ("assets/emotion/images/joy.png", b"\x89PNG-joy"),
            ],
        );

        let bytes = read(&record_at(&card, "assets/icon/images/main.png"))
            .await
            .expect("a card archive opens")
            .expect("the entry is in there");
        assert_eq!(
            bytes, b"\x89PNG-icon",
            "the entry's own bytes, not the archive's"
        );
    }

    /// The answer that keeps a walk shrinking. A card addresses its own
    /// slots with the same spelling as its entries, and both reach
    /// here; if a slot read as "not just now" every backfill pass would
    /// pick it up again and every one would fail.
    #[tokio::test]
    async fn an_address_the_archive_does_not_hold_is_never_rather_than_later() {
        let dir = tempfile::tempdir().unwrap();
        let card = dir.path().join("lyra.charx");
        charx_at(&card, &[("card.json", b"{}")]);

        assert!(
            read(&record_at(&card, "field=name")).await.is_none(),
            "a slot the card states is not a file it packs"
        );
        assert!(
            read(&record_at(&card, "assets/icon/images/main.png"))
                .await
                .is_none(),
            "a picture the card names and the packer left out is not coming later"
        );
    }

    /// An archive states its own entry lengths and was written by
    /// somebody else, so the ceiling is what the read is held to rather
    /// than the number in the file.
    #[test]
    fn an_entry_over_the_ceiling_is_refused_rather_than_allocated_for() {
        let dir = tempfile::tempdir().unwrap();
        let card = dir.path().join("big.charx");
        charx_at(&card, &[("assets/icon/images/main.png", &[0u8; 4096])]);
        let entry = "assets/icon/images/main.png";

        let refused = entry_bytes_within(&card, entry, 1024)
            .expect("the entry is in there")
            .expect_err("4096 bytes do not fit under 1024");
        assert!(
            refused.to_string().contains("ceiling"),
            "the refusal says what it was: {refused}"
        );

        assert_eq!(
            entry_bytes_within(&card, entry, 8192)
                .unwrap()
                .unwrap()
                .len(),
            4096,
            "and the same entry under the ceiling comes back whole"
        );
    }

    /// The line this module exists to draw: a JSONL record is not
    /// answered with its container, and a plain `.zip` is not opened at
    /// all.
    #[tokio::test]
    async fn a_container_nothing_opens_answers_never_rather_than_wrongly() {
        let dir = tempfile::tempdir().unwrap();

        let log = dir.path().join("session.jsonl");
        std::fs::write(&log, b"{\"uuid\":\"u1\"}\n").unwrap();
        assert!(read(&record_at(&log, "u1")).await.is_none());

        let bundle = dir.path().join("pictures.zip");
        charx_at(&bundle, &[("a.png", b"\x89PNG")]);
        assert!(
            read(&record_at(&bundle, "a.png")).await.is_none(),
            "an archive is not opened because it is an archive"
        );
    }

    #[tokio::test]
    async fn a_file_is_read_whole_and_a_name_has_nothing_to_read() {
        let dir = tempfile::tempdir().unwrap();
        let picture = dir.path().join("a.png");
        std::fs::write(&picture, b"\x89PNG-plain").unwrap();

        let locator: SourceLocator = LocalPath::try_from(picture.to_str().unwrap())
            .unwrap()
            .into();
        assert_eq!(read(&locator).await.unwrap().unwrap(), b"\x89PNG-plain");

        let missing: SourceLocator =
            LocalPath::try_from(dir.path().join("gone.png").to_str().unwrap())
                .unwrap()
                .into();
        assert!(read(&missing).await.unwrap().is_err());

        assert!(
            read(&SourceLocator::from_wire("https://example.com/a.png").unwrap())
                .await
                .is_none()
        );
    }
}
