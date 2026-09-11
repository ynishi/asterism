//! The destination profiles a send can be aimed with, listed from disk.
//!
//! A profile is one JSON file in the directory
//! [`release_dirs::profile_dir`](crate::release_dirs::profile_dir)
//! resolves, holding the transfer exporter's params minus the one key
//! the send writes. Files rather than rows, and a list rather than an
//! editor: the app reads this directory, validates what is in it and
//! lets somebody choose, and never writes to it. What an agency's
//! intake asks for moves on that agency's schedule, so the columns a
//! sidecar carries live in the file, and nothing in this tree knows any
//! of them.
//!
//! # The parser is the sender's
//!
//! [`asterism_exporter_transfer::read_profile`] is what answers for
//! each file, which is the same `serde_json::from_value` over the same
//! struct that `dispatch` runs, plus the refusals the send makes before
//! a connection opens. A list that blessed a profile the send then
//! refused would be the failure this listing exists to prevent, so the
//! two do not get separate opinions. That is also why this module sits
//! in `asterism-server`: it is the crate that already builds the
//! exporter registry, so it can name the transfer crate's parser
//! without the desktop crate taking a dependency on an adapter it does
//! not otherwise know about.
//!
//! # A file that does not parse is listed, with its reason
//!
//! Dropping it would leave somebody editing a file the app has stopped
//! mentioning, wondering why it never appears. It is listed, carrying
//! the parser's own sentence, and it cannot be picked.
//!
//! # A directory that is not there is empty rather than broken
//!
//! Nothing creates this directory: the first profile is written by
//! hand, and until then there is nothing to read. "No profiles" and "no
//! directory" are the same answer to the only question the screen asks,
//! and the directory travels with the list so the answer says where to
//! put one. A directory that exists and cannot be read is a different
//! matter and is reported as itself.

use std::path::Path;

use asterism_contract::forge::{TransferProfileDto, TransferProfileListDto};

/// What a profile file is called.
const PROFILE_SUFFIX: &str = ".json";

/// Every profile in `dir`, by name.
///
/// Never fails. Every way this can go wrong is a sentence about one
/// file or about the directory, and both belong on the list a person is
/// looking at rather than in place of it.
pub fn list(dir: &Path) -> TransferProfileListDto {
    let directory = dir.display().to_string();
    let mut profiles = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // Not yet written is the ordinary state, and it is the state
        // the directory beside the empty list explains.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return TransferProfileListDto {
                directory,
                profiles,
            };
        }
        Err(err) => {
            return TransferProfileListDto {
                directory,
                profiles: vec![TransferProfileDto {
                    name: String::new(),
                    path: dir.display().to_string(),
                    scheme: None,
                    host: None,
                    directory: None,
                    error: Some(format!("this directory cannot be read: {err}")),
                }],
            };
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.ends_with(PROFILE_SUFFIX) || !path.is_file() {
            continue;
        }
        let name = file_name[..file_name.len() - PROFILE_SUFFIX.len()].to_string();
        profiles.push(read_one(&name, &path));
    }
    // By name, so the list is the same on two machines and on two runs.
    // `read_dir` promises no order at all.
    profiles.sort_by(|left, right| left.name.cmp(&right.name));

    TransferProfileListDto {
        directory,
        profiles,
    }
}

/// One file, read and validated.
fn read_one(name: &str, path: &Path) -> TransferProfileDto {
    let refused = |reason: String| TransferProfileDto {
        name: name.to_string(),
        path: path.display().to_string(),
        scheme: None,
        host: None,
        directory: None,
        error: Some(reason),
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => return refused(format!("this file cannot be read: {err}")),
    };
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(err) => return refused(format!("this file is not JSON: {err}")),
    };
    match asterism_exporter_transfer::read_profile(&json) {
        Ok(facts) => TransferProfileDto {
            name: name.to_string(),
            path: path.display().to_string(),
            scheme: Some(facts.scheme.to_string()),
            host: Some(facts.host),
            directory: Some(facts.directory),
            error: None,
        },
        Err(reason) => refused(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped example with the send's own block removed, which is
    /// what a profile on disk is.
    fn example_profile() -> String {
        let mut whole: serde_json::Value =
            serde_json::from_str(asterism_exporter_transfer::params_example_json())
                .expect("the shipped example is JSON");
        whole
            .as_object_mut()
            .expect("the example is an object")
            .remove(asterism_exporter_transfer::RESERVED_KEY);
        whole.to_string()
    }

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).expect("write the fixture");
    }

    /// The three answers the list has to be able to give, in one
    /// directory: a file that sends, a file that does not parse, and a
    /// file that sets the word the send writes. Each is a row, and the
    /// two that cannot be picked say why.
    #[test]
    fn a_valid_file_a_broken_one_and_a_reserved_one_are_each_answered() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write(tmp.path(), "agency.json", &example_profile());
        write(tmp.path(), "broken.json", "{ this is not json");
        let mut reserved: serde_json::Value =
            serde_json::from_str(&example_profile()).expect("json");
        reserved.as_object_mut().unwrap().insert(
            asterism_exporter_transfer::RESERVED_KEY.to_string(),
            serde_json::json!({ "files": [] }),
        );
        write(tmp.path(), "reserved.json", &reserved.to_string());

        let listed = list(tmp.path());
        assert_eq!(listed.directory, tmp.path().display().to_string());
        let names: Vec<&str> = listed.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["agency", "broken", "reserved"]);

        let good = &listed.profiles[0];
        assert_eq!(good.error, None);
        assert_eq!(good.scheme.as_deref(), Some("sftp"));
        assert_eq!(good.host.as_deref(), Some("stock.example.com"));
        assert_eq!(good.directory.as_deref(), Some("/incoming/2026-09"));
        assert!(good.path.ends_with("agency.json"), "{}", good.path);

        let broken = &listed.profiles[1];
        assert!(broken.scheme.is_none() && broken.host.is_none());
        assert!(
            broken.error.as_deref().unwrap_or_default().contains("JSON"),
            "{:?}",
            broken.error
        );

        let reserved = &listed.profiles[2];
        assert!(reserved.scheme.is_none());
        assert!(
            reserved
                .error
                .as_deref()
                .unwrap_or_default()
                .contains(asterism_exporter_transfer::RESERVED_KEY),
            "{:?}",
            reserved.error
        );
    }

    /// Anything that is not a `.json` file is not a profile. A
    /// directory people keep files in collects `README`s and editor
    /// leftovers, and each one listed as a broken profile is a row
    /// telling somebody to fix a file that is doing nothing wrong.
    #[test]
    fn only_json_files_are_profiles() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write(tmp.path(), "agency.json", &example_profile());
        write(tmp.path(), "NOTES.md", "where these come from");
        write(tmp.path(), "agency.json.bak", "{}");
        std::fs::create_dir(tmp.path().join("old.json")).expect("a directory named like one");

        let listed = list(tmp.path());
        let names: Vec<&str> = listed.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["agency"]);
    }

    /// Nothing creates this directory, so its absence is the ordinary
    /// state before the first profile is written — an empty list that
    /// still says where one would go.
    #[test]
    fn a_directory_that_is_not_there_is_an_empty_list_that_says_where() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("never-made");
        let listed = list(&missing);
        assert!(listed.profiles.is_empty());
        assert_eq!(listed.directory, missing.display().to_string());
    }
}
