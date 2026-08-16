//! Data-profile and on-disk layout conventions.
//!
//! Asterism separates three local workloads that have very different
//! durability requirements:
//!
//! - [`DataProfile::Dev`] — disposable development data.
//! - [`DataProfile::Dogfood`] — durable, real daily-use data.
//! - [`DataProfile::Bench`] — reproducible large/stress datasets.
//!
//! Resolution order is `$ASTERISM_HOME` (explicit path) followed by
//! `$ASTERISM_PROFILE`, then a build-mode default (`dev` for debug builds,
//! `dogfood` for release builds). Named profiles live below
//! `$HOME/.asterism/profiles/<profile>`. An explicit home without a profile
//! is treated as `custom`, preserving scratch/test workflows.
//!
//! Named homes contain a `.asterism-profile` marker. Opening a home whose
//! marker disagrees with `$ASTERISM_PROFILE` is rejected before SQLite or
//! Tantivy is touched; this is the last guard against a mistyped launch
//! command pointing a development build at durable dogfood data.

use std::fmt;
use std::path::{Path, PathBuf};

use asterism_core::domain::observation::Env;
use asterism_core::error::DomainError;

const PROFILE_ENV: &str = "ASTERISM_PROFILE";
const HOME_ENV: &str = "ASTERISM_HOME";
const PROFILE_MARKER: &str = ".asterism-profile";

/// Isolated local dataset selected for the current process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataProfile {
    /// Disposable development data.
    Dev,
    /// Durable real-world daily-use data.
    Dogfood,
    /// Reproducible large/stress dataset.
    Bench,
    /// Explicit `$ASTERISM_HOME` with no named profile.
    Custom,
}

impl DataProfile {
    /// Stable slug used in environment values, paths, markers, and UI chrome.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Dogfood => "dogfood",
            Self::Bench => "bench",
            Self::Custom => "custom",
        }
    }

    /// Default loopback HTTP port for this profile.
    pub const fn default_http_port(self) -> u16 {
        match self {
            Self::Dogfood | Self::Custom => 8989,
            Self::Dev => 18_989,
            Self::Bench => 28_989,
        }
    }
}

/// The profile is also the `env` every observation record carries.
///
/// Two enums rather than one because they answer different questions —
/// this one selects an on-disk layout, [`Env`] classifies a record — but
/// they range over the same four datasets, and this total match is what
/// makes a future profile fail to compile rather than fail to classify.
impl From<DataProfile> for Env {
    fn from(profile: DataProfile) -> Self {
        match profile {
            DataProfile::Dev => Env::Dev,
            DataProfile::Dogfood => Env::Dogfood,
            DataProfile::Bench => Env::Bench,
            DataProfile::Custom => Env::Custom,
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
        other => Err(DomainError::Infra(anyhow::anyhow!(
            "invalid {PROFILE_ENV}={other:?}; expected dev, dogfood, or bench"
        ))),
    }
}

fn default_profile() -> DataProfile {
    if cfg!(debug_assertions) {
        DataProfile::Dev
    } else {
        DataProfile::Dogfood
    }
}

/// Returns the active data profile without creating directories.
pub fn active_profile() -> Result<DataProfile, DomainError> {
    match std::env::var(PROFILE_ENV) {
        Ok(raw) => parse_profile(&raw),
        Err(std::env::VarError::NotPresent) => {
            if std::env::var_os(HOME_ENV).is_some() {
                Ok(DataProfile::Custom)
            } else {
                Ok(default_profile())
            }
        }
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
        .join(".asterism")
        .join("profiles")
        .join(profile.as_str()))
}

/// One race is closed here and one is not, so both are named.
///
/// Closed: two openers of the same named profile, where the loser used
/// to read the winner's marker between its creation and its contents.
/// See [`publish_marker`].
///
/// Open: a `Custom` opener and a named opener of the same home. `Custom`
/// never writes a marker, so an opener that reads `NotFound` before the
/// named one publishes is admitted, and the two then run against one
/// home recording different `Env` values into one database. Closing it
/// would mean making `Custom` take part in the publish protocol rather
/// than only reading, which is a wider change than the marker's own
/// atomicity and is not attempted here.
fn verify_profile_marker(home: &Path, profile: DataProfile) -> Result<(), DomainError> {
    if profile == DataProfile::Custom {
        let marker = home.join(PROFILE_MARKER);
        return match std::fs::read_to_string(&marker) {
            Ok(existing) => Err(DomainError::Infra(anyhow::anyhow!(
                "profile guard rejected {}: named home {:?} requires matching {PROFILE_ENV}",
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
    let marker = home.join(PROFILE_MARKER);
    // Two passes, and the second one is the whole point. A marker that
    // does not exist is created here, and two processes opening the same
    // home at once both arrive at that branch. Whoever loses the race
    // comes back around and reads what the winner wrote, rather than
    // reporting a mismatch against a file it watched being born.
    //
    // Bounded at two deliberately: after `publish_marker` reports that
    // another process holds the name, the marker exists with its full
    // contents, so a third pass could not learn anything a second one
    // did not. A loop without a bound here would be a spin on whatever
    // unexpected state produced it — and reaching the bound at all
    // means the marker was published and removed again underneath this
    // process, twice, which is what the error below says.
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

/// What [`publish_marker`] managed to do.
///
/// Separate from the `io::Error` it also returns, because the caller
/// dispatches on "somebody else got there first" and that is not the
/// only thing underneath which can raise `AlreadyExists` — the
/// temporary file's own creation can too, when it exhausts its name
/// retries. Conflating them would send the caller round the loop over a
/// marker that was never published.
enum Published {
    /// This process created the marker.
    ByUs,
    /// Another process holds the name. Read it and compare.
    ByAnother,
}

/// Creates the marker so that no reader can observe it half-written, and
/// so that a concurrent creator cannot be overwritten.
///
/// `std::fs::write` is what used to be here, and it is two operations:
/// a create-and-truncate, then a write. Between them the marker exists
/// and is empty, and a second process reading it there gets `Ok("")` —
/// which is not the profile name, so the guard rejected the open with
/// `marker says ""`. That is a race between processes rather than a test
/// artifact: the CI suite met it because a workspace run opens the `dev`
/// home from many test binaries in quick succession, but two
/// application instances starting together reach it the same way.
///
/// Writing the contents to a temporary file first and publishing it
/// under the marker's name closes the window: the name appears already
/// complete, or not at all.
///
/// `hard_link` rather than `rename`, even though rename is the more
/// familiar atomic publish, because rename replaces. Two processes
/// opening one home under different profiles both find no marker, and
/// with rename both would succeed and the second would erase the first —
/// the guard would then pass exactly the mistyped-launch case it exists
/// to refuse. `hard_link` fails with `AlreadyExists` instead, which is
/// the answer the caller needs: read what is there and compare.
///
/// Not every filesystem has hard links, and `$ASTERISM_HOME` may be
/// pointed at any of them — an external exFAT volume is an ordinary
/// place to put a media library. Where the link cannot be made, this
/// falls back to `create_new`, which keeps the no-replace property that
/// the guard depends on and gives up only the atomicity of the
/// contents: the empty window returns, on exactly the filesystems where
/// nothing can close it. Refusing to open the home there would be the
/// worse trade.
///
/// The contents are flushed before either publish. Without that a crash
/// can leave a zero-length marker behind, and an empty marker is not a
/// transient the way the original race was — it is permanent, rejects
/// every later open with the same `marker says ""`, and nothing here
/// repairs it.
///
/// One deliberate change of behaviour: `NamedTempFile` creates at 0600,
/// where `std::fs::write` created at 0666 masked by the umask. The
/// marker lives in the user's own data home and is read by the same
/// user, so the narrower mode is the right one; it is recorded here
/// because it is a change to a file that ships.
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
    // filesystem boundary, and `NamedTempFile` keeps the name unique
    // across the processes racing here.
    let mut temp = tempfile::Builder::new()
        .prefix(".asterism-profile.")
        .tempfile_in(dir)?;
    // Through the open handle rather than by re-opening `temp.path()`.
    // This crate's manifest promoted `tempfile` out of dev-dependencies
    // precisely so that a temporary is created with `O_EXCL` and 0600,
    // and writing by path with `File::create` would hand that back.
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
    // `temp` drops here, unlinking itself. It has served its purpose
    // once the marker holds the contents, and it must not be left behind
    // when the publish failed.
}

/// Returns (creating on demand) the isolated Asterism home directory.
pub fn asterism_home() -> Result<PathBuf, DomainError> {
    let profile = active_profile()?;
    let home = resolved_home(
        std::env::var_os(HOME_ENV).map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        profile,
    )?;
    std::fs::create_dir_all(&home)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot create asterism home: {e}")))?;
    verify_profile_marker(&home, profile)?;
    Ok(home)
}

/// Returns the default SQLite database path (`<asterism_home>/asterism.db`).
pub fn default_db_path() -> Result<PathBuf, DomainError> {
    Ok(asterism_home()?.join("asterism.db"))
}

/// Returns (creating on demand) the profile-local Tantivy index directory.
pub fn tantivy_index_dir() -> Result<PathBuf, DomainError> {
    let dir = asterism_home()?.join("tantivy");
    std::fs::create_dir_all(&dir)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot create tantivy index dir: {e}")))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_profiles_have_isolated_default_homes() {
        let base = PathBuf::from("/users/test");
        assert_eq!(
            resolved_home(None, Some(base.clone()), DataProfile::Dev).unwrap(),
            base.join(".asterism/profiles/dev")
        );
        assert_eq!(
            resolved_home(None, Some(base.clone()), DataProfile::Dogfood).unwrap(),
            base.join(".asterism/profiles/dogfood")
        );
        assert_eq!(
            resolved_home(None, Some(base), DataProfile::Bench).unwrap(),
            PathBuf::from("/users/test/.asterism/profiles/bench")
        );
    }

    #[test]
    fn explicit_home_wins_for_every_profile() {
        let explicit = PathBuf::from("/tmp/asterism-test");
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
    fn profile_ports_do_not_collide() {
        assert_eq!(DataProfile::Dogfood.default_http_port(), 8989);
        assert_eq!(DataProfile::Dev.default_http_port(), 18_989);
        assert_eq!(DataProfile::Bench.default_http_port(), 28_989);
    }

    #[test]
    fn concurrent_opens_of_one_home_all_succeed() {
        // The failure this is here for: `just check` on `main` went red
        // on 2026-08-16 with `profile guard rejected …: marker says "",
        // process requested "dev"`, one test out of 1424. The marker was
        // being read between its creation and its contents.
        //
        // Sixteen threads released together, because the window is the
        // few microseconds between two syscalls and one pair of threads
        // will not reliably land in it. Eight rounds on top of that: a
        // single round caught the old implementation two times in five
        // when this was checked by putting `std::fs::write` back, and a
        // regression test that reports the bug two times in five is one
        // nobody can act on. Eight rounds make that reliable on the
        // machine it was measured on; a runner with fewer cores may be
        // less sensitive, which costs false negatives rather than false
        // alarms.
        //
        // A contended round is up to sixteen reads, up to fifteen
        // temporaries created, written, failed to link and unlinked, and
        // up to fifteen second reads — so the cost is milliseconds of
        // thread spawning rather than the microseconds the syscalls
        // take, and the whole test is still well under a tenth of a
        // second.
        for round in 0..8 {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().to_path_buf();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));

            let failures: Vec<String> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..16)
                    .map(|_| {
                        let home = home.clone();
                        let barrier = std::sync::Arc::clone(&barrier);
                        scope.spawn(move || {
                            barrier.wait();
                            verify_profile_marker(&home, DataProfile::Dev)
                                .err()
                                .map(|err| err.to_string())
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().unwrap())
                    .collect()
            });

            assert!(
                failures.is_empty(),
                "round {round}: every opener of one home under one profile \
                 must succeed; got: {failures:?}"
            );
            assert_eq!(
                std::fs::read_to_string(home.join(PROFILE_MARKER)).unwrap(),
                "dev\n",
                "round {round}: the surviving marker must hold exactly one \
                 profile name"
            );
            assert_eq!(
                std::fs::read_dir(&home).unwrap().count(),
                1,
                "round {round}: the losers' temporaries must not survive"
            );
        }
    }

    #[test]
    fn concurrent_opens_under_two_profiles_leave_one_winner() {
        // The other half, and the one the first test cannot reach:
        // swapping `hard_link` for `rename` in `publish_marker` keeps
        // every assertion above green, because with one profile the
        // losers agree with the winner either way. Rename replaces, so
        // under it both profiles would publish and the second would
        // erase the first — two processes then running against one home
        // believing different things, which is the mistyped-launch case
        // the module doc says the marker exists to refuse.
        //
        // Here the two profiles are released together. Whichever wins,
        // its openers all succeed, the other profile's all fail, and the
        // marker on disk agrees with the winners. Under `rename` the
        // last of those breaks.
        for round in 0..8 {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().to_path_buf();
            let barrier = std::sync::Barrier::new(16);

            let results: Vec<(DataProfile, Result<(), String>)> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..16)
                    .map(|n| {
                        let profile = if n % 2 == 0 {
                            DataProfile::Dev
                        } else {
                            DataProfile::Dogfood
                        };
                        let home = &home;
                        let barrier = &barrier;
                        scope.spawn(move || {
                            barrier.wait();
                            let outcome =
                                verify_profile_marker(home, profile).map_err(|err| err.to_string());
                            (profile, outcome)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().unwrap())
                    .collect()
            });

            let marker = std::fs::read_to_string(home.join(PROFILE_MARKER)).unwrap();
            let winner = marker.trim().to_string();
            assert!(
                winner == "dev" || winner == "dogfood",
                "round {round}: the marker must hold one of the two profiles, got {winner:?}"
            );

            for (profile, outcome) in results {
                if profile.as_str() == winner {
                    assert!(
                        outcome.is_ok(),
                        "round {round}: an opener of the winning profile \
                         {winner:?} was rejected: {outcome:?}"
                    );
                } else {
                    let err = outcome.expect_err(
                        "an opener of the losing profile must be rejected, not admitted",
                    );
                    assert!(
                        err.contains("profile guard rejected"),
                        "round {round}: the losing profile must be rejected by \
                         the guard, got {err:?}"
                    );
                }
            }
            assert_eq!(
                std::fs::read_dir(&home).unwrap().count(),
                1,
                "round {round}: the losers' temporaries must not survive"
            );
        }
    }

    #[test]
    fn marker_rejects_cross_profile_open() {
        let temp = tempfile::tempdir().unwrap();
        verify_profile_marker(temp.path(), DataProfile::Dev).unwrap();
        let err = verify_profile_marker(temp.path(), DataProfile::Dogfood).unwrap_err();
        assert!(err.to_string().contains("profile guard rejected"));
        let err = verify_profile_marker(temp.path(), DataProfile::Custom).unwrap_err();
        assert!(
            err.to_string()
                .contains("requires matching ASTERISM_PROFILE")
        );
    }
}
