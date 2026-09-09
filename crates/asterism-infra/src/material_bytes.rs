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
//! ## Which containers open
//!
//! A [`Record`](SourceLocator::Record) is a container plus an address
//! inside it, and `SourceLocator::local_path` refuses to hand over the
//! container on the record's behalf — a thousand-line log would answer
//! every line with the whole file, which is one fingerprint repeated a
//! thousand times. That reasoning holds for a record whose bytes *are*
//! the container's: a JSONL line is text the importer already carried,
//! and there is nothing else in the file that belongs to it alone.
//!
//! A ZIP entry is the other case. It has bytes of its own, at a known
//! offset, and reading it yields those and nothing else. So the opening
//! is per container shape rather than blanket, and [`opens_for_records`]
//! is the whole list: `.charx`, the character-card archive. A plain
//! `.zip` is not on it and is never handed here — an archive is not
//! opened because it is an archive.

use std::io;
use std::path::Path;

use asterism_core::domain::source_locator::SourceLocator;

/// Whether a container's shape is one whose records can be read out of
/// it.
///
/// Extension-keyed, like `source_text`'s reader dispatch. The list is
/// short on purpose: a shape joins it when something files records
/// inside that shape, not because the format could be opened.
pub fn opens_for_records(container: &Path) -> bool {
    container
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("charx"))
}

/// The bytes this locator addresses.
///
/// See the module docstring for what each of the three answers means to
/// a caller; the ordering (`None` before `Err`) is the whole point of
/// returning them together.
pub async fn read(locator: &SourceLocator) -> Option<io::Result<Vec<u8>>> {
    match locator {
        SourceLocator::File(path) => Some(tokio::fs::read(path.as_path()).await),
        SourceLocator::Record(record) => {
            let container = record.container().as_path();
            if !opens_for_records(container) {
                return None;
            }
            let container = container.to_path_buf();
            let entry = record.record().as_str().to_string();
            // `zip` is a blocking reader over a seeking file, and the
            // entry is inflated in full before it is handed back.
            let read = tokio::task::spawn_blocking(move || entry_bytes(&container, &entry)).await;
            Some(match read {
                Ok(result) => result,
                Err(join) => Err(io::Error::other(join)),
            })
        }
        SourceLocator::Remote(_) | SourceLocator::Logical(_) => None,
    }
}

/// Opens the archive and inflates one entry.
///
/// A missing entry is [`NotFound`](io::ErrorKind::NotFound) rather than
/// a distinct answer: a locator naming an entry the archive does not
/// hold is the same shape of problem as a path naming a file that is
/// not there, and both deserve the later pass that a re-import would
/// settle.
fn entry_bytes(container: &Path, entry: &str) -> io::Result<Vec<u8>> {
    use std::io::Read as _;

    let file = std::fs::File::open(container)?;
    let mut archive = zip::ZipArchive::new(file).map_err(io::Error::other)?;
    let mut member = archive.by_name(entry).map_err(|err| match err {
        zip::result::ZipError::FileNotFound => {
            io::Error::new(io::ErrorKind::NotFound, format!("no entry {entry:?}"))
        }
        other => io::Error::other(other),
    })?;
    let mut bytes = Vec::with_capacity(member.size() as usize);
    member.read_to_end(&mut bytes)?;
    Ok(bytes)
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

    #[tokio::test]
    async fn an_entry_the_archive_does_not_hold_is_a_later_pass() {
        let dir = tempfile::tempdir().unwrap();
        let card = dir.path().join("lyra.charx");
        charx_at(&card, &[("card.json", b"{}")]);

        let outcome = read(&record_at(&card, "assets/icon/images/main.png"))
            .await
            .expect("the container opens, so this is not 'never'");
        assert_eq!(
            outcome.unwrap_err().kind(),
            io::ErrorKind::NotFound,
            "missing entry defers like a missing file, rather than retiring the row"
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
