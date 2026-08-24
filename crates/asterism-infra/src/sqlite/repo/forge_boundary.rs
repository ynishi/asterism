//! What the forge asks of everything outside it, answered by SQLite.
//!
//! Two contracts, and neither is a repository: they are the forge's
//! side of questions it cannot answer itself.
//!
//! - [`SqliteStore`] answers whether an asset exists. One question,
//!   because the forge has one.
//! - [`SqliteActors`] answers what a handle stands for, and mints one
//!   when it has not seen the subject before.
//!
//! They live beside the repositories rather than among them because
//! what they implement is the boundary, not storage — the forge names
//! `boundary::Store`, not `AssetRepository`, and that is the whole
//! arrangement. Putting them in one file keeps the two halves of "what
//! the forge asks" readable together.

use asterism_core::domain::attribution::{AttributionContext, Author};
use asterism_core::domain::forge::boundary::{Actors, Store};
use asterism_core::domain::forge::model::value::ActorId;
use asterism_core::domain::value::AssetId;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::infra_err;

/// Answers [`Store`] over the `asset` table.
#[derive(Clone)]
pub struct SqliteStore {
    isle: AsyncIsle,
}

impl SqliteStore {
    /// Wraps a connection.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl Store for SqliteStore {
    /// Whether this asset exists.
    ///
    /// A trashed asset counts. Trashing is reversible and the row is
    /// still there; the forge is asking whether the reference is real,
    /// not whether it is in somebody's active view. Reading the trash
    /// stamp here would put a raw-layer display state into a question
    /// about existence, and would make an operation legal or not
    /// depending on what the owner had tidied away that morning.
    async fn exists(&self, asset: &AssetId) -> Result<bool, DomainError> {
        let asset = *asset.as_uuid();
        self.isle
            .call(move |conn| {
                let found: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM asset WHERE id = ?1",
                    params![asset],
                    |row| row.get(0),
                )?;
                Ok(found > 0)
            })
            .await
            .map_err(infra_err)
    }
}

/// Keeps what a forge handle stands for.
#[derive(Clone)]
pub struct SqliteActors {
    isle: AsyncIsle,
}

impl SqliteActors {
    /// Wraps a connection.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    /// The handle for one thing, minted if this is the first time.
    ///
    /// `INSERT … ON CONFLICT DO NOTHING` and then a read, rather than
    /// a read and then an insert: the two statements are one
    /// transaction, and the conflict clause is what makes the second
    /// caller find the first one's row instead of a violation. Written
    /// this way even though this store serialises its writes, because
    /// what holds the rule should be the statement rather than the
    /// arrangement around it.
    ///
    /// `display_name` is captured here and nowhere else, which is what
    /// the conflict clause buys beyond concurrency: a later resolve of
    /// the same handle does not reach the row, so the name a handle was
    /// minted under is the name it keeps. The same discipline as the
    /// teams plane's actor stamp, and for the same reason — a record
    /// that re-derived the name would answer for today rather than for
    /// when the work was done.
    async fn handle(
        &self,
        stands_for: &'static str,
        subject: Option<String>,
        display_name: Option<String>,
    ) -> Result<ActorId, DomainError> {
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO forge_actor (id, stands_for, subject, display_name, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5) \
                     ON CONFLICT DO NOTHING",
                    params![
                        Uuid::now_v7(),
                        stands_for,
                        subject.as_deref(),
                        display_name.as_deref(),
                        chrono::Utc::now().timestamp_millis(),
                    ],
                )?;
                let id: Uuid = tx.query_row(
                    "SELECT id FROM forge_actor \
                      WHERE stands_for = ?1 AND COALESCE(subject, '') = COALESCE(?2, '')",
                    params![stands_for, subject.as_deref()],
                    |row| row.get(0),
                )?;
                tx.commit()?;
                Ok(ActorId::from_uuid(id))
            })
            .await
            .map_err(infra_err)
    }
}

#[async_trait]
impl Actors for SqliteActors {
    /// The handle for whoever this write is by.
    ///
    /// Keyed on the author and nothing else. The triple also says
    /// which agent carried the write out and through which entry
    /// point, and neither is who did it — the forge keeps a handle on
    /// an actor, and an agent acting for somebody does not make a
    /// second somebody. What the triple says about the agent is the
    /// raw layer's to record, and it does.
    ///
    /// A write that named nobody resolves to one handle rather than a
    /// fresh one each time: "nobody said who" is a single answer.
    ///
    /// The name a handle is minted under comes from the same triple,
    /// and the triple has none to give: an author is the owner or a
    /// subject token, and the token is an identifier the sharing lists
    /// and viewers carry rather than anything a person would recognise
    /// as their name. Passing it as the display snapshot would put a
    /// copy of the `subject` column into the column beside it and call
    /// the duplicate a capture. So the snapshot is `None` until a
    /// caller has a name to state, and the seat is here for the day
    /// one does.
    async fn resolve(&self, by: &AttributionContext) -> Result<ActorId, DomainError> {
        match by.author() {
            None => self.handle("unrecorded", None, None).await,
            Some(Author::Owner) => self.handle("owner", None, None).await,
            Some(Author::Subject(subject)) => {
                self.handle("subject", Some(subject.clone()), None).await
            }
        }
    }

    /// The handle for this instance, which is what a line's rule
    /// writes as.
    ///
    /// One row, with no subject, because one deployment is one server.
    /// The column is there for the setting the port's doc names — when
    /// there are several, they are told apart by it, and nothing
    /// already written has to move.
    async fn server(&self) -> Result<ActorId, DomainError> {
        self.handle("server", None, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    // The asset rows below need an owner because the schema says so,
    // not because the forge asks about one.
    use asterism_core::domain::value::PersonaId;

    async fn seeded() -> (
        AsyncIsle,
        rusqlite_isle::AsyncIsleDriver,
        PersonaId,
        AssetId,
    ) {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let persona = PersonaId::new();
        let asset = AssetId::new();
        let (p, a) = (*persona.as_uuid(), *asset.as_uuid());
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, accent_color, display_order, archived, \
                                      created_at, updated_at) \
                 VALUES (?1, 'p', NULL, 0, 0, 0, 0)",
                params![p],
            )?;
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, labels, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', 'a.md', 'dialogue', '[]', 0, 0, 0)",
                params![a, p],
            )
        })
        .await
        .unwrap();
        (isle, driver, persona, asset)
    }

    /// An asset exists or it does not, and whose it is does not come
    /// into it.
    ///
    /// This asked `owns(persona, asset)` until the second surface over
    /// the forge made the shape visible: a line carries no owner, so
    /// there was nothing for an answer about ownership to be measured
    /// against, and the caller supplying the persona meant the check
    /// could only ever agree with itself.
    #[tokio::test]
    async fn an_asset_exists_whoever_it_belongs_to() {
        let (isle, driver, _persona, asset) = seeded().await;
        let store = SqliteStore::new(isle);

        assert!(store.exists(&asset).await.unwrap());
        assert!(
            !store.exists(&AssetId::new()).await.unwrap(),
            "an id nobody minted names nothing"
        );

        driver.shutdown().await.unwrap();
    }

    /// Trashing is reversible and the row is still there. Reading the
    /// stamp here would make an operation legal or not depending on
    /// what the owner had tidied away that morning.
    #[tokio::test]
    async fn a_trashed_asset_still_exists() {
        let (isle, driver, _persona, asset) = seeded().await;
        let id = *asset.as_uuid();
        isle.call(move |conn| {
            conn.execute("UPDATE asset SET trashed_at = 1 WHERE id = ?1", params![id])
        })
        .await
        .unwrap();

        assert!(SqliteStore::new(isle).exists(&asset).await.unwrap());
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_same_subject_gets_the_same_handle_and_a_different_one_does_not() {
        let (isle, driver, _, _) = seeded().await;
        let actors = SqliteActors::new(isle);

        let ana = AttributionContext::asserted(Some(Author::Subject("ana".into())), None).unwrap();
        let boro =
            AttributionContext::asserted(Some(Author::Subject("boro".into())), None).unwrap();

        let first = actors.resolve(&ana).await.unwrap();
        assert_eq!(
            first,
            actors.resolve(&ana).await.unwrap(),
            "one handle each"
        );
        assert_ne!(first, actors.resolve(&boro).await.unwrap());

        driver.shutdown().await.unwrap();
    }

    /// The four things a handle can stand for are four handles, and
    /// each is stable. The three that carry no subject are the ones a
    /// plain unique index would have let duplicate.
    #[tokio::test]
    async fn every_kind_of_actor_gets_one_handle_and_keeps_it() {
        let (isle, driver, _, _) = seeded().await;
        let actors = SqliteActors::new(isle.clone());

        let named =
            AttributionContext::asserted(Some(Author::Subject("ana".into())), None).unwrap();
        let agent_only = AttributionContext::asserted(
            None,
            Some(asterism_core::domain::attribution::OperatorRef::new("claude-code").unwrap()),
        )
        .unwrap();
        let owner = AttributionContext::owner_surface();

        let handles = [
            actors.resolve(&named).await.unwrap(),
            actors.resolve(&agent_only).await.unwrap(),
            actors.resolve(&owner).await.unwrap(),
            actors.server().await.unwrap(),
        ];
        let unique: std::collections::BTreeSet<_> = handles.iter().collect();
        assert_eq!(unique.len(), 4, "four different things, four handles");

        // Asked again, every one of them answers the same.
        assert_eq!(actors.resolve(&named).await.unwrap(), handles[0]);
        assert_eq!(actors.resolve(&agent_only).await.unwrap(), handles[1]);
        assert_eq!(actors.resolve(&owner).await.unwrap(), handles[2]);
        assert_eq!(actors.server().await.unwrap(), handles[3]);

        let rows: i64 = isle
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM forge_actor", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(rows, 4, "and minted nothing on the second pass");

        driver.shutdown().await.unwrap();
    }

    /// The display snapshot is written when the row is minted and not
    /// afterwards. What holds that is the mint statement's conflict
    /// clause, so the test is on the statement rather than on the one
    /// value today's callers happen to pass.
    #[tokio::test]
    async fn the_display_snapshot_is_captured_at_mint_and_not_updated() {
        let (isle, driver, _, _) = seeded().await;
        let actors = SqliteActors::new(isle.clone());

        let named =
            AttributionContext::asserted(Some(Author::Subject("ana".into())), None).unwrap();
        let handle = actors.resolve(&named).await.unwrap();

        // Nothing on a write path has a name to state, so the column
        // is NULL rather than a second copy of the subject token.
        let captured: Option<String> = isle
            .call(|conn| {
                conn.query_row(
                    "SELECT display_name FROM forge_actor WHERE stands_for = 'subject'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(captured, None);

        // A mint that does carry one keeps it, and a later resolve of
        // the same handle does not reach the row to change it.
        let second = SqliteActors::new(isle.clone())
            .handle("owner", None, Some("Hoshino".into()))
            .await
            .unwrap();
        assert_eq!(
            SqliteActors::new(isle.clone())
                .handle("owner", None, Some("Someone Else".into()))
                .await
                .unwrap(),
            second,
            "the same handle comes back rather than a second row"
        );
        let kept: Option<String> = isle
            .call(|conn| {
                conn.query_row(
                    "SELECT display_name FROM forge_actor WHERE stands_for = 'owner'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(kept.as_deref(), Some("Hoshino"));

        assert_ne!(handle, second);
        driver.shutdown().await.unwrap();
    }

    /// Which agent carried a write out is not who did it. Two agents
    /// acting for one subject are one actor, and the raw layer is
    /// where the agent is recorded.
    #[tokio::test]
    async fn the_agent_does_not_make_a_second_actor() {
        let (isle, driver, _, _) = seeded().await;
        let actors = SqliteActors::new(isle);

        let by_hand =
            AttributionContext::asserted(Some(Author::Subject("ana".into())), None).unwrap();
        let by_agent = AttributionContext::asserted(
            Some(Author::Subject("ana".into())),
            Some(asterism_core::domain::attribution::OperatorRef::new("claude-code").unwrap()),
        )
        .unwrap();

        assert_eq!(
            actors.resolve(&by_hand).await.unwrap(),
            actors.resolve(&by_agent).await.unwrap()
        );

        driver.shutdown().await.unwrap();
    }
}
