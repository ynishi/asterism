//! Data-profile and on-disk layout conventions for the teams plane.
//!
//! This mirrors `asterism-infra`'s profile mechanism — the same
//! selection table, the same marker guard — with every name swapped so
//! the two planes cannot open each other's data by accident:
//!
//! | | local app | teams plane |
//! |---|---|---|
//! | profile env | `ASTERISM_PROFILE` | `ASTERISM_TEAMS_PROFILE` |
//! | home env | `ASTERISM_HOME` | `ASTERISM_TEAMS_HOME` |
//! | named home | `~/.asterism/profiles/<p>` | `~/.asterism-teams/profiles/<p>` |
//! | marker | `.asterism-profile` | `.asterism-teams-profile` |
//! | database | `asterism.db` | `teams.db` |
//!
//! Mirrored rather than imported because importing would mean a
//! `teams-* → asterism-infra` edge, which #83 §4 forbids in any form.
//!
//! The selection table is the one `asterism-infra` settled on:
//!
//! | `$ASTERISM_TEAMS_HOME` | `$ASTERISM_TEAMS_PROFILE` | result |
//! |---|---|---|
//! | unset | unset | the build's default — `dev` in debug, `dogfood` in release |
//! | unset | `dev` / `dogfood` / `bench` | that profile, under `~/.asterism-teams/profiles/` |
//! | unset | `custom` | error: `custom` is a home, and none was given |
//! | set | unset | error: name the profile too |
//! | set | `dev` / `dogfood` / `bench` | that profile, at the explicit path |
//! | set | `custom` | `custom` at the explicit path, unguarded |
//!
//! Named homes contain a marker; opening a home whose marker disagrees
//! with the requested profile is rejected before SQLite is touched.
//! The marker is published the way the sibling publishes its own —
//! contents written to a temporary file, synced, then hard-linked
//! under the marker's name (`create_new` fallback only where the
//! filesystem has no hard links): the name appears already complete or
//! not at all, so no observable state ever holds an empty marker. A
//! crash between creating the name and writing its contents would
//! otherwise wedge the home permanently — an empty marker rejects
//! every later open with `marker says ""`, and nothing here repairs
//! one — which is why the mechanism is mirrored rather than simplified
//! for this plane's quieter topology. `hard_link` rather than `rename`
//! for the sibling's reason too: rename replaces, and two profiles
//! racing for one fresh home must never both claim it.

use std::fmt;
use std::path::{Path, PathBuf};

use teams_core::DomainError;

const PROFILE_ENV: &str = "ASTERISM_TEAMS_PROFILE";
const HOME_ENV: &str = "ASTERISM_TEAMS_HOME";
const PROFILE_MARKER: &str = ".asterism-teams-profile";
/// The teams database file name inside the home — the teams plane's
/// own file, sharing nothing with the app's `asterism.db`.
const DB_FILE: &str = "teams.db";
/// The blob store's directory inside the home — the CAS root the
/// `blob` module lays `sha256/` and `staging/` under (#83 §3).
const BLOBS_DIR: &str = "blobs";

/// Isolated local dataset selected for the current process — same four
/// profiles as the local app, because the workloads they separate
/// (disposable dev data / durable daily data / reproducible stress
/// data / an explicit unguarded path) are the same on this plane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataProfile {
    /// Disposable development data.
    Dev,
    /// Durable real-world daily-use data.
    Dogfood,
    /// Reproducible large/stress dataset.
    Bench,
    /// An explicit `$ASTERISM_TEAMS_HOME`, opened under
    /// `$ASTERISM_TEAMS_PROFILE=custom`. The one profile the marker
    /// does not guard, which is why it has to be asked for by name.
    Custom,
}

impl DataProfile {
    /// Stable slug used in environment values, paths and markers.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Dogfood => "dogfood",
            Self::Bench => "bench",
            Self::Custom => "custom",
        }
    }
}

impl fmt::Display for DataProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn parse_profile(raw: &str) -> Result<DataProfile, DomainError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "dev" => Ok(DataProfile::Dev),
        "dogfood" => Ok(DataProfile::Dogfood),
        "bench" => Ok(DataProfile::Bench),
        "custom" => Ok(DataProfile::Custom),
        other => Err(DomainError::Infra(anyhow::anyhow!(
            "invalid {PROFILE_ENV}={other:?}; expected dev, dogfood, bench, or custom"
        ))),
    }
}

/// Chooses the profile from the two environment values without reading
/// them — [`active_profile`] does the reading and hands the answers
/// here, so the table in the module doc can be tested without a
/// process-wide `set_var`.
///
/// `Custom` is selected and never inferred, for the reason the local
/// app's module records at length: `Custom` writes no marker, so
/// reaching it by *forgetting* `$ASTERISM_TEAMS_PROFILE` would disable
/// the guard by omission. An explicit home with no profile named is
/// refused instead.
fn select_profile(
    profile_env: Option<&str>,
    has_explicit_home: bool,
) -> Result<DataProfile, DomainError> {
    match profile_env {
        Some(raw) => {
            let profile = parse_profile(raw)?;
            if profile == DataProfile::Custom && !has_explicit_home {
                return Err(DomainError::Infra(anyhow::anyhow!(
                    "{PROFILE_ENV}=custom names an explicit home, but {HOME_ENV} is not set"
                )));
            }
            Ok(profile)
        }
        None if has_explicit_home => Err(DomainError::Infra(anyhow::anyhow!(
            "{HOME_ENV} is set without {PROFILE_ENV}; name the profile that home holds \
             (dev, dogfood, bench), or {PROFILE_ENV}=custom to open it unguarded"
        ))),
        None => Ok(default_profile()),
    }
}

fn default_profile() -> DataProfile {
    if cfg!(debug_assertions) {
        DataProfile::Dev
    } else {
        DataProfile::Dogfood
    }
}

/// Returns the active teams-plane data profile without creating
/// directories.
pub fn active_profile() -> Result<DataProfile, DomainError> {
    let has_explicit_home = std::env::var_os(HOME_ENV).is_some();
    match std::env::var(PROFILE_ENV) {
        Ok(raw) => select_profile(Some(&raw), has_explicit_home),
        Err(std::env::VarError::NotPresent) => select_profile(None, has_explicit_home),
        Err(err) => Err(DomainError::Infra(anyhow::anyhow!(
            "cannot read {PROFILE_ENV}: {err}"
        ))),
    }
}

fn resolved_home(
    explicit_home: Option<PathBuf>,
    user_home: Option<PathBuf>,
    profile: DataProfile,
) -> Result<PathBuf, DomainError> {
    if let Some(path) = explicit_home {
        return Ok(path);
    }
    let user_home = user_home.ok_or_else(|| {
        DomainError::Infra(anyhow::anyhow!("HOME environment variable is not set"))
    })?;
    Ok(user_home
        .join(".asterism-teams")
        .join("profiles")
        .join(profile.as_str()))
}

/// The marker guard: a named home belongs to exactly one profile, and
/// opening it under another is rejected before SQLite is touched.
///
/// `Custom` reads the marker and never writes one — it takes no
/// ownership of the home it opens, which is what "unguarded scratch
/// home" means; what keeps that safe is [`select_profile`] refusing to
/// reach `custom` by omission.
fn verify_profile_marker(home: &Path, profile: DataProfile) -> Result<(), DomainError> {
    let marker = home.join(PROFILE_MARKER);
    if profile == DataProfile::Custom {
        return match std::fs::read_to_string(&marker) {
            Ok(existing) => Err(DomainError::Infra(anyhow::anyhow!(
                "profile guard rejected {}: its marker names profile {:?}, \
                 which requires matching {PROFILE_ENV}",
                home.display(),
                existing.trim()
            ))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(DomainError::Infra(anyhow::anyhow!(
                "cannot read profile marker {}: {err}",
                marker.display()
            ))),
        };
    }
    // Two passes, mirroring the sibling: a marker that does not exist
    // is created below, and whoever loses that race comes back around
    // and reads what the winner wrote — rather than reporting a
    // mismatch against a file it watched being born. Bounded at two
    // because once `publish_marker` reports another process holds the
    // name, the marker exists with its full contents; a third pass
    // could learn nothing a second one did not.
    for _ in 0..2 {
        match std::fs::read_to_string(&marker) {
            Ok(existing) if existing.trim() == profile.as_str() => return Ok(()),
            Ok(existing) => {
                return Err(DomainError::Infra(anyhow::anyhow!(
                    "profile guard rejected {}: marker says {:?}, process requested {:?}",
                    home.display(),
                    existing.trim(),
                    profile.as_str()
                )));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                match publish_marker(&marker, profile) {
                    Ok(Published::ByUs) => return Ok(()),
                    // Someone else published first. Their contents are
                    // already complete, so the next pass either agrees
                    // with us or is a genuine cross-profile open.
                    Ok(Published::ByAnother) => continue,
                    Err(err) => {
                        return Err(DomainError::Infra(anyhow::anyhow!(
                            "cannot create profile marker {}: {err}",
                            marker.display()
                        )));
                    }
                }
            }
            Err(err) => {
                return Err(DomainError::Infra(anyhow::anyhow!(
                    "cannot read profile marker {}: {err}",
                    marker.display()
                )));
            }
        }
    }
    Err(DomainError::Infra(anyhow::anyhow!(
        "profile guard could not settle {}: another process published the \
         marker and removed it again, twice, while this one was opening \
         the home",
        marker.display()
    )))
}

/// What [`publish_marker`] managed to do — separate from the
/// `io::Error` it also returns, because the caller dispatches on
/// "somebody else got there first" and the temporary file's own
/// creation can raise `AlreadyExists` too when it exhausts its name
/// retries; conflating them would send the caller round the loop over
/// a marker that was never published.
enum Published {
    /// This process created the marker.
    ByUs,
    /// Another process holds the name. Read it and compare.
    ByAnother,
}

/// Creates the marker so that no reader — and no crash — can observe
/// it half-written, and so that a concurrent creator cannot be
/// overwritten. The mechanism is `asterism-infra`'s, mirrored: the
/// contents go to a temporary file first (written through the open
/// handle, synced), then the name is published with `hard_link`, which
/// fails with `AlreadyExists` instead of replacing. Where the
/// filesystem has no hard links, `create_new` keeps the no-replace
/// property and gives up only the atomicity of the contents — the
/// empty window returns on exactly the filesystems where nothing can
/// close it, and refusing to open the home there would be the worse
/// trade.
fn publish_marker(marker: &Path, profile: DataProfile) -> std::io::Result<Published> {
    use std::io::Write as _;

    let dir = marker.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "profile marker has no parent directory",
        )
    })?;
    let contents = format!("{}\n", profile.as_str());

    // Same directory as the marker, so the link cannot cross a
    // filesystem boundary; `NamedTempFile` creates with `O_EXCL` and
    // 0600 and keeps the name unique across racing processes.
    let mut temp = tempfile::Builder::new()
        .prefix(".asterism-teams-profile.")
        .tempfile_in(dir)?;
    temp.write_all(contents.as_bytes())?;
    temp.as_file().sync_all()?;

    match std::fs::hard_link(temp.path(), marker) {
        Ok(()) => Ok(Published::ByUs),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(Published::ByAnother),
        // No hard links here. `create_new` is the same refusal to
        // replace, bought without them.
        Err(_) => match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)
        {
            Ok(mut file) => {
                file.write_all(contents.as_bytes())?;
                file.sync_all()?;
                Ok(Published::ByUs)
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(Published::ByAnother),
            Err(err) => Err(err),
        },
    }
    // `temp` drops here, unlinking itself — it must not be left behind
    // when the publish failed.
}

/// Returns (creating on demand) the teams-plane home directory for the
/// active profile, with the marker guard applied.
pub fn teams_home() -> Result<PathBuf, DomainError> {
    let profile = active_profile()?;
    let home = resolved_home(
        std::env::var_os(HOME_ENV).map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        profile,
    )?;
    std::fs::create_dir_all(&home)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot create teams home: {e}")))?;
    verify_profile_marker(&home, profile)?;
    Ok(home)
}

/// Returns the default teams SQLite database path
/// (`<teams_home>/teams.db`) — the teams plane's own file, sharing
/// nothing with the app database.
pub fn default_db_path() -> Result<PathBuf, DomainError> {
    Ok(teams_home()?.join(DB_FILE))
}

/// Returns the default blob store root (`<teams_home>/blobs`), under
/// the same profile guard as the database — the two live and move
/// together, which is what lets #83 §3's ordering rule (bytes durable
/// before the link commits) mean one machine's durability, not two.
pub fn default_blob_root() -> Result<PathBuf, DomainError> {
    Ok(teams_home()?.join(BLOBS_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_profiles_have_isolated_default_homes() {
        let base = PathBuf::from("/users/test");
        assert_eq!(
            resolved_home(None, Some(base.clone()), DataProfile::Dev).unwrap(),
            base.join(".asterism-teams/profiles/dev")
        );
        assert_eq!(
            resolved_home(None, Some(base.clone()), DataProfile::Dogfood).unwrap(),
            base.join(".asterism-teams/profiles/dogfood")
        );
        assert_eq!(
            resolved_home(None, Some(base), DataProfile::Bench).unwrap(),
            PathBuf::from("/users/test/.asterism-teams/profiles/bench")
        );
    }

    #[test]
    fn explicit_home_wins_for_every_profile() {
        let explicit = PathBuf::from("/tmp/teams-test");
        assert_eq!(
            resolved_home(
                Some(explicit.clone()),
                Some(PathBuf::from("/users/test")),
                DataProfile::Dogfood
            )
            .unwrap(),
            explicit
        );
    }

    #[test]
    fn custom_is_selected_and_never_inferred() {
        assert_eq!(select_profile(None, false).unwrap(), default_profile());
        assert_eq!(
            select_profile(Some("dev"), false).unwrap(),
            DataProfile::Dev
        );
        assert_eq!(
            select_profile(Some("custom"), true).unwrap(),
            DataProfile::Custom
        );

        // An explicit home with no profile named is refused — reaching
        // the unguarded mode by omission is the failure the local app's
        // table exists to close, and the mirror keeps it closed.
        let err = select_profile(None, true).unwrap_err().to_string();
        assert!(
            err.contains(HOME_ENV) && err.contains(PROFILE_ENV),
            "the refusal must name both variables so the fix is obvious: {err}"
        );

        let err = select_profile(Some("custom"), false)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(HOME_ENV),
            "the refusal must say which variable is missing: {err}"
        );

        let err = select_profile(Some("nonsense"), true)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("invalid"),
            "an unknown value must be refused as invalid, not resolved: {err}"
        );
    }

    #[test]
    fn marker_rejects_cross_profile_open() {
        let temp = tempfile::tempdir().unwrap();
        verify_profile_marker(temp.path(), DataProfile::Dev).unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join(PROFILE_MARKER)).unwrap(),
            "dev\n"
        );
        assert_eq!(
            std::fs::read_dir(temp.path()).unwrap().count(),
            1,
            "the publish's temporary must not survive"
        );

        // The same profile re-opens; another named profile and the
        // unguarded one are both refused.
        verify_profile_marker(temp.path(), DataProfile::Dev).unwrap();
        let err = verify_profile_marker(temp.path(), DataProfile::Dogfood).unwrap_err();
        assert!(err.to_string().contains("profile guard rejected"));
        let err = verify_profile_marker(temp.path(), DataProfile::Custom).unwrap_err();
        assert!(
            err.to_string()
                .contains("requires matching ASTERISM_TEAMS_PROFILE")
        );
    }

    #[test]
    fn custom_takes_no_ownership_of_the_home_it_opens() {
        let temp = tempfile::tempdir().unwrap();
        verify_profile_marker(temp.path(), DataProfile::Custom)
            .expect("custom opens an unmarked home");
        assert!(
            !temp.path().join(PROFILE_MARKER).exists(),
            "custom must leave the home unmarked"
        );
        verify_profile_marker(temp.path(), DataProfile::Dev)
            .expect("a named profile may still claim a home custom left unmarked");
    }
}
