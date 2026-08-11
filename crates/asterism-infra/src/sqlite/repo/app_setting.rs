//! SQLite adapter for the `AppSettingRepository` port.
//!
//! Stores only the keys a user has actually overridden; absence of a row
//! is the "use the code default" state, so `delete` is the reset path.
//!
//! **Downgrade tolerance.** A row this build cannot interpret — an
//! unknown key, or a timestamp outside the representable range — is
//! treated as "not overridden" rather than as an error. A profile opened
//! by a newer build can carry keys this one has never heard of, and a
//! settings screen that refuses to render at all would be a far worse
//! outcome than ignoring one row. The row is left on disk, so going back
//! to the newer build restores the preference, and every skip is logged
//! at warn level so the anomaly is visible rather than silent.
//!
//! `list` and `find` apply the *same* rule on purpose: if `find` raised
//! an error where `list` skipped, one uninterpretable row would make the
//! two read paths disagree about whether a key is overridden.

use asterism_core::domain::app_setting::{AppSetting, SettingKey};
use asterism_core::domain::repository::AppSettingRepository;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `AppSettingRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteAppSettingRepository {
    isle: AsyncIsle,
}

impl SqliteAppSettingRepository {
    /// Wraps a writer `AsyncIsle` handle (same discipline as the sibling
    /// adapters).
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Row shape scanned inside the isle closure.
struct SettingRow {
    key: String,
    value_json: String,
    updated_at: i64,
}

impl SettingRow {
    const COLUMNS: &'static str = "key, value_json, updated_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            key: row.get(0)?,
            value_json: row.get(1)?,
            updated_at: row.get(2)?,
        })
    }

    /// Promotes the row, or returns `None` when this build cannot
    /// interpret it (unknown key / unrepresentable timestamp). The
    /// reason is logged so a dropped override is diagnosable — see the
    /// module docs for why this is not an error.
    fn into_domain(self) -> Option<AppSetting> {
        let key = match SettingKey::parse(&self.key) {
            Ok(key) => key,
            Err(e) => {
                tracing::warn!(
                    event = "diag.setting.key_unknown",
                    key = %self.key,
                    error = %e,
                    "ignoring app_setting row: this build does not know the key"
                );
                return None;
            }
        };
        let updated_at = match ms_to_datetime(self.updated_at) {
            Ok(at) => at,
            Err(e) => {
                tracing::warn!(
                    event = "diag.setting.timestamp_unrepresentable",
                    key = %self.key,
                    updated_at = self.updated_at,
                    error = %e,
                    "ignoring app_setting row: updated_at is not representable"
                );
                return None;
            }
        };
        Some(AppSetting {
            key,
            value_json: self.value_json,
            updated_at,
        })
    }
}

#[async_trait]
impl AppSettingRepository for SqliteAppSettingRepository {
    async fn list(&self) -> Result<Vec<AppSetting>, DomainError> {
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM app_setting ORDER BY key",
                    SettingRow::COLUMNS
                ))?;
                let rows = stmt
                    .query_map([], SettingRow::from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        // Uninterpretable rows are skipped, not fatal — see module docs.
        Ok(rows
            .into_iter()
            .filter_map(SettingRow::into_domain)
            .collect())
    }

    async fn find(&self, key: SettingKey) -> Result<Option<AppSetting>, DomainError> {
        let key_str = key.as_str();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM app_setting WHERE key = ?1",
                        SettingRow::COLUMNS
                    ),
                    params![key_str],
                    SettingRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        // Same rule as `list`: a row this build cannot interpret reads
        // as "not overridden" so the two paths cannot disagree.
        Ok(row.and_then(SettingRow::into_domain))
    }

    async fn upsert(&self, setting: &AppSetting) -> Result<(), DomainError> {
        let key = setting.key.as_str();
        let value_json = setting.value_json.clone();
        let updated = datetime_to_ms(&setting.updated_at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO app_setting (key, value_json, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET
                         value_json = excluded.value_json,
                         updated_at = excluded.updated_at",
                    params![key, value_json, updated],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn delete(&self, key: SettingKey) -> Result<(), DomainError> {
        let key_str = key.as_str();
        self.isle
            .call(move |conn| {
                // No row is the default state, so removing an absent key
                // is success rather than `NotFound`: "make this the
                // default" has to be idempotent.
                conn.execute("DELETE FROM app_setting WHERE key = ?1", params![key_str])?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use chrono::{DateTime, TimeZone, Utc};

    fn now() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(1_700_000_000_000).unwrap()
    }

    fn setting(key: &str, value_json: &str) -> AppSetting {
        AppSetting {
            key: SettingKey::parse(key).unwrap(),
            value_json: value_json.to_string(),
            updated_at: now(),
        }
    }

    #[tokio::test]
    async fn upsert_find_delete_roundtrip() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAppSettingRepository::new(isle);
        let key = SettingKey::parse("ui.clean_mode").unwrap();

        assert!(repo.find(key).await.unwrap().is_none());

        repo.upsert(&setting("ui.clean_mode", "true"))
            .await
            .unwrap();
        let stored = repo.find(key).await.unwrap().unwrap();
        assert_eq!(stored.value_json, "true");
        assert_eq!(stored.updated_at, now());

        repo.delete(key).await.unwrap();
        assert!(repo.find(key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_replaces_rather_than_duplicating() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAppSettingRepository::new(isle);

        repo.upsert(&setting("ui.clean_mode", "true"))
            .await
            .unwrap();
        repo.upsert(&setting("ui.clean_mode", "false"))
            .await
            .unwrap();

        let rows = repo.list().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value_json, "false");
    }

    #[tokio::test]
    async fn delete_of_absent_key_is_a_no_op() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAppSettingRepository::new(isle);
        let key = SettingKey::parse("ui.clean_mode").unwrap();
        repo.delete(key).await.unwrap();
        repo.delete(key).await.unwrap();
        assert!(repo.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_skips_keys_this_build_does_not_know() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAppSettingRepository::new(isle.clone());
        repo.upsert(&setting("ui.clean_mode", "true"))
            .await
            .unwrap();
        // Simulate a row written by a newer build that knows a key this
        // one does not: the listing must degrade to the known subset
        // instead of failing outright.
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO app_setting (key, value_json, updated_at)
                 VALUES ('ui.from_the_future', 'true', 1)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let rows = repo.list().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key.as_str(), "ui.clean_mode");

        // The unknown row survives on disk, so the newer build still
        // sees the preference it wrote.
        let total: i64 = isle
            .call(move |conn| conn.query_row("SELECT COUNT(*) FROM app_setting", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn both_read_paths_agree_when_a_timestamp_is_unrepresentable() {
        let (isle, _driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAppSettingRepository::new(isle.clone());
        let key = SettingKey::parse("ui.clean_mode").unwrap();
        // `STRICT` only requires INTEGER, so any i64 is storable — a
        // build that switched to microseconds would land out of range.
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO app_setting (key, value_json, updated_at)
                 VALUES ('ui.clean_mode', 'true', ?1)",
                params![i64::MAX],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // Neither path may error, and neither may claim the key is
        // overridden — a disagreement here would make the settings
        // screen and a single-key read tell different stories.
        assert!(repo.list().await.unwrap().is_empty());
        assert!(repo.find(key).await.unwrap().is_none());
    }
}
