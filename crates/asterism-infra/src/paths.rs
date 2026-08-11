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
    match std::fs::read_to_string(&marker) {
        Ok(existing) if existing.trim() == profile.as_str() => Ok(()),
        Ok(existing) => Err(DomainError::Infra(anyhow::anyhow!(
            "profile guard rejected {}: marker says {:?}, process requested {:?}",
            home.display(),
            existing.trim(),
            profile.as_str()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(&marker, format!("{}\n", profile.as_str())).map_err(|e| {
                DomainError::Infra(anyhow::anyhow!(
                    "cannot create profile marker {}: {e}",
                    marker.display()
                ))
            })
        }
        Err(err) => Err(DomainError::Infra(anyhow::anyhow!(
            "cannot read profile marker {}: {err}",
            marker.display()
        ))),
    }
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
