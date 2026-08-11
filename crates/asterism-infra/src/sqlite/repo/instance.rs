//! SQLite adapter for the `InstanceRepository` port.
//!
//! One row, minted by the V49 migration. The adapter deliberately does
//! **not** create it on read: a migrated database always has it, so an
//! empty table is an anomaly, and minting a replacement would hand out
//! an identity that disagrees with whatever the rows on disk were
//! attributed against.

use asterism_core::domain::instance::InstanceIdentity;
use asterism_core::domain::repository::InstanceRepository;
use asterism_core::domain::value::InstanceId;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{infra_err, ms_to_datetime};

/// SQLite adapter for `InstanceRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteInstanceRepository {
    isle: AsyncIsle,
}

impl SqliteInstanceRepository {
    /// Wraps a writer `AsyncIsle` handle (same discipline as the sibling
    /// adapters).
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Row shape scanned inside the isle closure.
struct InstanceRow {
    instance_id: Uuid,
    created_at: i64,
    owner_subject: Option<String>,
}

impl InstanceRow {
    const COLUMNS: &'static str = "instance_id, created_at, owner_subject";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            instance_id: row.get(0)?,
            created_at: row.get(1)?,
            owner_subject: row.get(2)?,
        })
    }

    fn into_domain(self) -> Result<InstanceIdentity, DomainError> {
        Ok(InstanceIdentity {
            id: InstanceId::from_uuid(self.instance_id),
            created_at: ms_to_datetime(self.created_at)?,
            owner_subject: self.owner_subject,
        })
    }
}

#[async_trait]
impl InstanceRepository for SqliteInstanceRepository {
    async fn get(&self) -> Result<Option<InstanceIdentity>, DomainError> {
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!("SELECT {} FROM instance WHERE id = 0", InstanceRow::COLUMNS),
                    [],
                    InstanceRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(InstanceRow::into_domain).transpose()
    }

    async fn bind_owner(&self, subject: &str) -> Result<(), DomainError> {
        let subject = subject.trim().to_string();
        if subject.is_empty() {
            return Err(DomainError::Validation(
                "owner subject must not be empty".into(),
            ));
        }
        let affected = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE instance SET owner_subject = ?1 WHERE id = 0",
                    params![subject],
                )
            })
            .await
            .map_err(infra_err)?;
        if affected == 0 {
            // The row is the migration's to create. Writing one here
            // would mint an identity the existing rows were never
            // attributed against.
            return Err(DomainError::NotFound {
                entity: "instance",
                id: "0".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::instance::OwnerResolution;

    use crate::sqlite::open_and_migrate_in_memory;

    #[tokio::test]
    async fn a_migrated_profile_carries_one_unbound_identity() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteInstanceRepository::new(isle);

        let identity = repo.get().await.unwrap().expect("V49 mints the row");
        assert_eq!(identity.id.as_uuid().get_version_num(), 7);
        assert_eq!(
            identity.resolve_owner(),
            OwnerResolution::Unresolved,
            "a local instance has no bound subject, so `Owner` stays a relative reference"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn binding_the_owner_makes_the_reference_resolve() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteInstanceRepository::new(isle);

        repo.bind_owner("alice").await.unwrap();

        let identity = repo.get().await.unwrap().unwrap();
        assert_eq!(identity.owner_subject.as_deref(), Some("alice"));
        assert_eq!(
            identity.resolve_owner(),
            OwnerResolution::Resolved("alice"),
            "once bound, `Author::Owner` names a subject in the sharing namespace"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn binding_keeps_the_minted_identity() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteInstanceRepository::new(isle);
        let before = repo.get().await.unwrap().unwrap();

        repo.bind_owner("alice").await.unwrap();

        let after = repo.get().await.unwrap().unwrap();
        assert_eq!(
            (after.id, after.created_at),
            (before.id, before.created_at),
            "binding an owner is not a new instance"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_blank_subject_is_refused() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteInstanceRepository::new(isle);

        // A blank token names nobody; storing it would make the instance
        // look bound while resolving to nothing.
        assert!(repo.bind_owner("").await.is_err());
        assert!(repo.bind_owner("   ").await.is_err());
        assert_eq!(repo.get().await.unwrap().unwrap().owner_subject, None);

        driver.shutdown().await.unwrap();
    }
}
