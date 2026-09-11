//! The two registered directories the release surface uses, resolved.
//!
//! [`release.output_dir`] names where a change point is written out to
//! and [`send.profile_dir`] names where destination profiles are read
//! from. Both are
//! [`Text`](asterism_core::domain::app_setting::SettingValueKind::Text)
//! keys in
//! [`SETTING_REGISTRY`](asterism_core::domain::app_setting::SETTING_REGISTRY),
//! and both default to the empty string.
//!
//! # Why the empty string, and why this module exists
//!
//! A registry default is a `&'static str`, and neither of these paths
//! is a constant: the profile home is resolved at run time from
//! `$ASTERISM_HOME` and `$ASTERISM_PROFILE`, differs between two
//! processes of the same build, and is verified against a marker on the
//! way out. There is nothing to write into the registry, so the empty
//! string is what it writes, and it means "the profile home's own".
//!
//! That convention needs exactly one reader, or it becomes two
//! resolvers that disagree the first time somebody changes the leaf
//! name. This module is it. Both transports call in here rather than
//! joining a path of their own, and the frontend is handed the answer
//! rather than deriving it — it cannot read the environment at all, and
//! a second copy of the rule in TypeScript is the copy nobody would
//! edit.
//!
//! # A value somebody set is used as typed
//!
//! Only the empty case reaches the profile home. A path a person put in
//! the settings screen is taken as it stands and never joined onto
//! anything, because a setting that silently became a subdirectory of
//! itself is a value that cannot be pointed anywhere.
//!
//! [`release.output_dir`]: asterism_core::domain::app_setting::SETTING_REGISTRY
//! [`send.profile_dir`]: asterism_core::domain::app_setting::SETTING_REGISTRY

use std::path::{Path, PathBuf};

use asterism_core::error::DomainError;

/// The registry key naming where a release writes its copies.
pub const OUTPUT_DIR_KEY: &str = "release.output_dir";

/// The registry key naming where destination profiles are read from.
pub const PROFILE_DIR_KEY: &str = "send.profile_dir";

/// What `release.output_dir` falls back to, under the profile home.
const OUTPUT_DIR_LEAF: &str = "releases";

/// What `send.profile_dir` falls back to, under the profile home.
const PROFILE_DIR_LEAF: &str = "transfer";

/// Where the next release writes its copies.
///
/// `setting` is the resolved value of [`OUTPUT_DIR_KEY`]; empty is the
/// profile home's own `releases/`.
pub fn output_dir(setting: &str) -> Result<PathBuf, DomainError> {
    resolve(setting, OUTPUT_DIR_LEAF)
}

/// Where destination profiles are read from.
///
/// `setting` is the resolved value of [`PROFILE_DIR_KEY`]; empty is the
/// profile home's own `transfer/`.
pub fn profile_dir(setting: &str) -> Result<PathBuf, DomainError> {
    resolve(setting, PROFILE_DIR_LEAF)
}

/// The profile home is read only when the setting is empty, so a
/// process that has one set answers without touching the marker.
fn resolve(setting: &str, leaf: &str) -> Result<PathBuf, DomainError> {
    match chosen(setting) {
        Some(path) => Ok(path),
        None => Ok(under_home(&asterism_infra::paths::asterism_home()?, leaf)),
    }
}

/// The path the setting names, or `None` for "the profile home's own".
///
/// Split out from [`resolve`] because it is the half worth testing: the
/// other half reads the environment, and a test that drove it would be
/// asserting about the machine it ran on.
fn chosen(setting: &str) -> Option<PathBuf> {
    let trimmed = setting.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// Where a leaf sits under a home. A function rather than a `join` at
/// each site, so the two keys cannot drift into two layouts.
fn under_home(home: &Path, leaf: &str) -> PathBuf {
    home.join(leaf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whitespace is not a path. A settings screen hands back what was
    /// typed, and a trailing newline pasted into the field would
    /// otherwise become a directory whose name ends in one.
    #[test]
    fn a_blank_setting_asks_for_the_profile_home() {
        assert_eq!(chosen(""), None);
        assert_eq!(chosen("   "), None);
        assert_eq!(chosen("\n"), None);
    }

    /// A value somebody set is the answer, whole. Nothing is appended
    /// to it, which is what lets the setting point at a directory that
    /// is not under the profile home at all.
    #[test]
    fn a_set_value_is_used_as_typed() {
        assert_eq!(
            chosen("/somewhere/else"),
            Some(PathBuf::from("/somewhere/else"))
        );
        assert_eq!(
            chosen("  /trimmed/path  "),
            Some(PathBuf::from("/trimmed/path"))
        );
    }

    /// The two keys land beside each other under one home, which is the
    /// layout the empty default promises.
    #[test]
    fn the_fallbacks_sit_under_the_home_they_are_given() {
        let home = Path::new("/profile/home");
        assert_eq!(
            under_home(home, OUTPUT_DIR_LEAF),
            PathBuf::from("/profile/home/releases")
        );
        assert_eq!(
            under_home(home, PROFILE_DIR_LEAF),
            PathBuf::from("/profile/home/transfer")
        );
    }
}
