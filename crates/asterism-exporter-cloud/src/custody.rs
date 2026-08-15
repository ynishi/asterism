//! Where a produced file lands once we hold it.
//!
//! `<custody_root>/dispatch/<dispatch_id>/<nnn>-<name>`.
//!
//! Dispatch-addressed rather than content-addressed. The question the
//! harvest asks is "which files did this dispatch produce", and this
//! layout answers it by listing a directory — no index, no lookup, and
//! it still answers after the platform's URL has expired and the
//! response that named it is gone. Content addressing answers a
//! different question (is this the same file as that one), the core's
//! digest axes already answer that one, and a digest can be computed
//! over these bytes later without moving them.
//!
//! Writing is idempotent by path: the index within the harvest and the
//! dispatch id are both stable, so a re-collect after a failed fetch
//! overwrites the same file rather than producing a second asset beside
//! the first.

use std::path::{Path, PathBuf};

use asterism_dispatch_sdk::ExporterError;

/// Resolves custody paths under one root.
#[derive(Debug, Clone)]
pub struct CustodyPaths {
    root: PathBuf,
}

impl CustodyPaths {
    /// Binds a root directory (the application directory).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Directory holding everything one dispatch produced.
    pub fn dispatch_dir(&self, dispatch_id: &str) -> PathBuf {
        self.root.join("dispatch").join(sanitise(dispatch_id))
    }

    /// Path for one harvested item.
    pub fn item_path(&self, dispatch_id: &str, index: usize, source_url: &str) -> PathBuf {
        self.dispatch_dir(dispatch_id)
            .join(format!("{index:03}-{}", file_name_from(source_url)))
    }

    /// Writes one harvested item, creating the dispatch directory.
    pub async fn write(
        &self,
        dispatch_id: &str,
        index: usize,
        source_url: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, ExporterError> {
        let path = self.item_path(dispatch_id, index, source_url);
        let dir = path.parent().expect("an item path always has a parent");
        tokio::fs::create_dir_all(dir).await.map_err(|e| {
            ExporterError::Other(anyhow::anyhow!(
                "creating custody directory {}: {e}",
                dir.display()
            ))
        })?;
        tokio::fs::write(&path, bytes).await.map_err(|e| {
            ExporterError::Other(anyhow::anyhow!(
                "writing custody file {}: {e}",
                path.display()
            ))
        })?;
        Ok(path)
    }
}

/// The file name a URL suggests, or `artefact` when it suggests none.
///
/// The query string is dropped before the last segment is taken.
/// Signed download URLs carry the whole signature there, and a file
/// named after one is unreadable in a directory listing and outlives
/// the signature it was named for.
fn file_name_from(source_url: &str) -> String {
    let without_query = source_url
        .split(['?', '#'])
        .next()
        .unwrap_or(source_url)
        .trim_end_matches('/');
    let candidate = Path::new(without_query)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cleaned = sanitise(&candidate);
    if cleaned.is_empty() {
        "artefact".into()
    } else {
        cleaned
    }
}

/// Reduces a path segment to characters that mean the same thing on
/// every filesystem this runs on, and cannot climb out of a directory.
///
/// A platform decides what its URLs look like, so the segment is
/// untrusted input.
fn sanitise(segment: &str) -> String {
    segment
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_names_the_file_and_its_query_string_does_not() {
        assert_eq!(file_name_from("https://cdn.test/out/a.png"), "a.png");
        assert_eq!(
            file_name_from("https://cdn.test/out/a.png?sig=abc123&exp=99"),
            "a.png"
        );
        assert_eq!(file_name_from("https://cdn.test/out/"), "out");
    }

    /// The last segment is untrusted, so it must not be able to leave
    /// the dispatch's own directory.
    #[test]
    fn a_hostile_last_segment_cannot_climb_out() {
        let paths = CustodyPaths::new(PathBuf::from("/data"));
        assert_eq!(
            paths.item_path("disp-1", 0, "https://cdn.test/a/../../etc/passwd"),
            PathBuf::from("/data/dispatch/disp-1/000-passwd"),
            "the segment has to be reduced to a name"
        );
        // Dots survive — a name is allowed to have an extension — so
        // what makes a traversal impossible is that separators do not,
        // and that a segment cannot end up as `.` or `..` on its own.
        assert_eq!(
            paths.item_path("../../escape", 1, "https://cdn.test/x.png"),
            PathBuf::from("/data/dispatch/_.._escape/001-x.png")
        );
        for hostile in ["..", ".", "../..", "a/b", "a\\b"] {
            let segment = sanitise(hostile);
            assert!(
                !segment.contains(['/', '\\']) && segment != "." && segment != "..",
                "sanitise({hostile:?}) produced a traversable segment: {segment:?}"
            );
        }
    }

    #[tokio::test]
    async fn writing_twice_overwrites_rather_than_adding_a_second_file() {
        let temp = tempfile::tempdir().unwrap();
        let paths = CustodyPaths::new(temp.path().to_path_buf());

        let first = paths
            .write("disp-1", 0, "https://cdn.test/a.png", b"one")
            .await
            .unwrap();
        let second = paths
            .write("disp-1", 0, "https://cdn.test/a.png", b"two")
            .await
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(tokio::fs::read(&first).await.unwrap(), b"two");

        let mut entries = tokio::fs::read_dir(paths.dispatch_dir("disp-1"))
            .await
            .unwrap();
        let mut count = 0;
        while entries.next_entry().await.unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 1, "a re-collect must not leave two files behind");
    }
}
