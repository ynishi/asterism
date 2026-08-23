//! Data-profile and on-disk layout conventions.
//!
//! Asterism separates three local workloads that have very different
//! durability requirements:
//!
//! - [`DataProfile::Dev`] — disposable development data.
//! - [`DataProfile::Dogfood`] — durable, real daily-use data.
//! - [`DataProfile::Bench`] — reproducible large/stress datasets.
//!
//! `$ASTERISM_PROFILE` names the profile and `$ASTERISM_HOME` overrides
//! where it lives; with neither, the build decides (`dev` for debug,
//! `dogfood` for release). Named profiles live below
//! `$HOME/.asterism/profiles/<profile>`.
//!
//! An explicit home used to fall back to `custom` when the profile was
//! absent, which made the unguarded mode something you reached by
//! forgetting. It is now asked for by name, and the table is:
//!
//! | `$ASTERISM_HOME` | `$ASTERISM_PROFILE` | result |
//! |---|---|---|
//! | unset | unset | the build's default — `dev` in debug, `dogfood` in release |
//! | unset | `dev` / `dogfood` / `bench` | that profile, under `$HOME/.asterism/profiles/` |
//! | unset | `custom` | error: `custom` is a home, and none was given |
//! | set | unset | error: name the profile too |
//! | set | `dev` / `dogfood` / `bench` | that profile, at the explicit path |
//! | set | `custom` | `custom` at the explicit path, unguarded |
//!
//! `$ASTERISM_PROFILE` alone is ordinary. It is the home-without-a-name
//! direction that is refused, because that is the one that used to
//! silently disable the marker.
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
    /// An explicit `$ASTERISM_HOME`, opened under
    /// `$ASTERISM_PROFILE=custom`. The one profile the marker does not
    /// guard, which is why it has to be asked for by name.
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
    ///
    /// `Custom` shares dogfood's, and that is now worth saying rather
    /// than leaving in the match arm. It was written when `Custom` was
    /// what you got by forgetting `$ASTERISM_PROFILE` — reached mostly
    /// by invocations that pass `--port` anyway, so the default rarely
    /// applied. It is now a scratch mode somebody selects while their
    /// real library exists, and only one core binds a port per machine:
    /// a scratch instance started first answers on the port dogfood's
    /// clients use. Every recipe here passes `--port`, so nothing in the
    /// repository depends on it; anyone running `custom` by hand should
    /// pass one too.
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
        "custom" => Ok(DataProfile::Custom),
        other => Err(DomainError::Infra(anyhow::anyhow!(
            "invalid {PROFILE_ENV}={other:?}; expected dev, dogfood, bench, or custom"
        ))),
    }
}

/// Chooses the profile from the two environment variables, without
/// reading them — `active_profile` does that and hands the answers here,
/// the way `resolved_home` is handed its paths, so the table below can
/// be tested without a process-wide `set_var`.
///
/// `Custom` is selected and never inferred. It used to be what an
/// explicit `$ASTERISM_HOME` fell back to when `$ASTERISM_PROFILE` was
/// absent, and that made it reachable by *forgetting* something — which
/// matters because `Custom` writes no marker and so takes no ownership
/// of the home it opens. A named profile can afterwards claim the same
/// home and both are admitted, deterministically, with the two
/// processes recording different [`Env`] values into one database. That
/// is the outcome the marker exists to prevent, and the fallback was
/// how you reached it without deciding to.
///
/// Asking for `custom` by name is still allowed, and still opts out of
/// the guard for that home. The difference is that it is now a decision
/// with a name on it rather than the consequence of an omission.
///
/// The table this implements is in the module documentation, where a
/// reader can reach it — this function is private, so a link to it from
/// there would resolve for nobody.
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

/// Returns the active data profile without creating directories.
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
        .join(".asterism")
        .join("profiles")
        .join(profile.as_str()))
}

/// `Custom` reads the marker and never writes one, so it takes no
/// ownership of the home it opens: open an unmarked home as `Custom`
/// and then as `dev`, and both are admitted. That is deliberate — it is
/// what "unguarded scratch home" means — and it is why nobody arrives
/// at `Custom` any more by leaving `$ASTERISM_PROFILE` out (the table in
/// this module's documentation). Opting out of the guard is a thing you
/// say; it is not a thing that happens to you.
///
/// Between named profiles the guard is total, and [`publish_marker`] is
/// what makes the ownership it records observable only once it is
/// complete.
fn verify_profile_marker(home: &Path, profile: DataProfile) -> Result<(), DomainError> {
    if profile == DataProfile::Custom {
        let marker = home.join(PROFILE_MARKER);
        return match std::fs::read_to_string(&marker) {
            // A marker saying `custom` is a home nothing can open:
            // `Custom` never publishes one, so it can only have been
            // put there by hand or carried in by a copy, and both this
            // branch and the named one below would refuse it. Worth its
            // own sentence — the message for a named marker tells the
            // reader to set `ASTERISM_PROFILE`, which is advice they
            // have already taken.
            Ok(existing) if existing.trim() == DataProfile::Custom.as_str() => {
                Err(DomainError::Infra(anyhow::anyhow!(
                    "profile guard rejected {}: its marker says {:?}, which no profile \
                     publishes and none can open — remove the marker if this home is \
                     meant to be scratch",
                    home.display(),
                    existing.trim()
                )))
            }
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

/// Returns (creating on demand) the profile-local model-package root
/// (#112): one subdirectory per installed package
/// (`<asterism_home>/models/<model_id>/`).
///
/// Profile-local like the Tantivy index and the preview renditions,
/// not shared across profiles: `$ASTERISM_HOME` must sandbox
/// everything a profile touches, or a bench run reads the weights the
/// user's real library trusts. Sharing bytes between profiles is a
/// symlink the person makes, not a path this function invents.
pub fn models_dir() -> Result<PathBuf, DomainError> {
    let dir = asterism_home()?.join("models");
    std::fs::create_dir_all(&dir)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot create models dir: {e}")))?;
    Ok(dir)
}

/// Returns (creating on demand) the profile-local trained-head root
/// (#132 phase 2): one subdirectory per trained head
/// (`<asterism_home>/heads/<label>/`), plus the `current` pointer file
/// promotion writes. Profile-local for the same reason as
/// [`models_dir`]: a head is trained on one profile's rulings, and a
/// bench profile must not read — or win over — the real library's.
pub fn heads_dir() -> Result<PathBuf, DomainError> {
    let dir = asterism_home()?.join("heads");
    std::fs::create_dir_all(&dir)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot create heads dir: {e}")))?;
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
    fn custom_is_selected_and_never_inferred() {
        // The whole table from `select_profile`, because the row that
        // used to be wrong is the one nobody would think to look at:
        // an explicit home with no profile named silently became
        // `Custom`, and `Custom` is the mode that opts out of the
        // marker guard. Reaching it by omission is what made the guard
        // avoidable by accident.
        assert_eq!(select_profile(None, false).unwrap(), default_profile());
        assert_eq!(
            select_profile(Some("dev"), false).unwrap(),
            DataProfile::Dev
        );
        assert_eq!(
            select_profile(Some("dogfood"), true).unwrap(),
            DataProfile::Dogfood
        );
        assert_eq!(
            select_profile(Some("custom"), true).unwrap(),
            DataProfile::Custom
        );

        // An explicit home with no profile named is now refused, where
        // it used to yield `Custom`.
        let err = select_profile(None, true).unwrap_err().to_string();
        assert!(
            err.contains(HOME_ENV) && err.contains(PROFILE_ENV),
            "the refusal must name both variables so the fix is obvious: {err}"
        );

        // `custom` is a home. Asking for one without giving it is not a
        // request this can honour.
        let err = select_profile(Some("custom"), false)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(HOME_ENV),
            "the refusal must say which variable is missing: {err}"
        );

        // With a home given, so that a misspelling cannot be rescued by
        // the missing-home refusal: both errors mention `custom`, and
        // asserting on that substring alone let a `parse_profile` whose
        // fallback returned `Ok(Custom)` pass this test — which is the
        // defect this whole change is about, since it would open a
        // mistyped `ASTERISM_PROFILE` as the one unguarded profile.
        let err = select_profile(Some("nonsense"), true)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("invalid"),
            "an unknown value must be refused as invalid, not resolved: {err}"
        );
        assert!(
            err.contains("custom"),
            "and the refusal should say custom is selectable: {err}"
        );
    }

    #[test]
    fn custom_takes_no_ownership_of_the_home_it_opens() {
        // Asserted rather than described, because it is the one place
        // the guard deliberately does not hold and prose is where that
        // turns back into a bug. `custom` writes no marker, so a named
        // profile can claim the same home afterwards and both are
        // admitted. That is what "unguarded scratch home" means; what
        // makes it safe is `select_profile` refusing to reach `custom`
        // by omission, which the test above covers.
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();

        verify_profile_marker(&home, DataProfile::Custom).expect("custom opens an unmarked home");
        assert!(
            !home.join(PROFILE_MARKER).exists(),
            "custom must leave the home unmarked"
        );
        verify_profile_marker(&home, DataProfile::Dev)
            .expect("a named profile may still claim a home custom left unmarked");
        assert_eq!(
            std::fs::read_to_string(home.join(PROFILE_MARKER)).unwrap(),
            "dev\n"
        );

        // The reverse order is refused, which is the half that does hold.
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();
        verify_profile_marker(&home, DataProfile::Dev).unwrap();
        let err = verify_profile_marker(&home, DataProfile::Custom).unwrap_err();
        assert!(err.to_string().contains("profile guard rejected"));
    }

    #[test]
    fn profile_ports_do_not_collide() {
        assert_eq!(DataProfile::Dogfood.default_http_port(), 8989);
        assert_eq!(DataProfile::Dev.default_http_port(), 18_989);
        assert_eq!(DataProfile::Bench.default_http_port(), 28_989);
        // The name of this test claims a property the fourth profile
        // does not have, so the exception is asserted rather than left
        // out — a reader who finds three of four covered cannot tell
        // whether the fourth was considered.
        assert_eq!(
            DataProfile::Custom.default_http_port(),
            DataProfile::Dogfood.default_http_port(),
            "custom shares dogfood's port on purpose; see default_http_port"
        );
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
