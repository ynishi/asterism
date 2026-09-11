//! SQLite adapter for the `SendRepository` port.
//!
//! `forge_send` holds where a release went, when, and by whose hand. It
//! is written once and read; nothing here updates a row, for the reason
//! [`asterism_core::domain::send`] gives.
//!
//! Why this is not on `SqliteForge` is the same answer the release
//! adapter beside it gives, and [`super::release`] is where it is
//! written. What it borrows is the same thing, and for the same reason:
//! [`ActRow`], because an act is three columns wherever it appears.

use asterism_core::domain::repository::SendRepository;
use asterism_core::domain::send::ReleaseSend;
use asterism_core::domain::value::{DispatchId, ReleaseId, SendId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::forge::rows::ActRow;
use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `SendRepository`.
#[derive(Clone)]
pub struct SqliteSendRepository {
    isle: AsyncIsle,
}

impl SqliteSendRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// One row, as the columns hold it.
struct SendRow {
    id: Uuid,
    release: Uuid,
    destination: String,
    dispatch: Uuid,
    at: i64,
    actor: Uuid,
    actor_kind: String,
}

impl SendRow {
    const COLUMNS: &'static str =
        "id, release_id, destination, dispatch_id, at, actor_id, actor_kind";

    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            release: row.get(1)?,
            destination: row.get(2)?,
            dispatch: row.get(3)?,
            at: row.get(4)?,
            actor: row.get(5)?,
            actor_kind: row.get(6)?,
        })
    }

    fn into_domain(self) -> Result<ReleaseSend, DomainError> {
        let kind = match self.actor_kind.as_str() {
            "user" => "user",
            "system" => "system",
            other => {
                return Err(DomainError::Validation(format!(
                    "a stored send names an actor kind this model does not have: {other}"
                )));
            }
        };
        let act = ActRow {
            at: ms_to_datetime(self.at)?,
            actor: asterism_core::domain::forge::model::value::ActorId::from_uuid(self.actor),
            kind,
        }
        .read()?;
        Ok(ReleaseSend::restored(
            SendId::from_uuid(self.id),
            ReleaseId::from_uuid(self.release),
            self.destination,
            DispatchId::from_uuid(self.dispatch),
            act,
        ))
    }
}

#[async_trait]
impl SendRepository for SqliteSendRepository {
    async fn record(&self, send: &ReleaseSend) -> Result<(), DomainError> {
        let id = *send.id().as_uuid();
        let release = *send.release().as_uuid();
        let destination = send.destination().to_string();
        let dispatch = *send.dispatch().as_uuid();
        let act = ActRow::of(send.act());
        let at = datetime_to_ms(&act.at);
        let actor = *act.actor.as_uuid();
        let kind = act.kind;
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO forge_send \
                         (id, release_id, destination, dispatch_id, at, actor_id, actor_kind) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id, release, destination, dispatch, at, actor, kind],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn find(&self, id: &SendId) -> Result<Option<ReleaseSend>, DomainError> {
        let key = *id.as_uuid();
        let found: Option<SendRow> = self
            .isle
            .call(move |conn| {
                let sql = format!("SELECT {} FROM forge_send WHERE id = ?1", SendRow::COLUMNS);
                match conn.query_row(&sql, params![key], SendRow::from_row) {
                    Ok(row) => Ok(Some(row)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(err) => Err(err),
                }
            })
            .await
            .map_err(infra_err)?;
        found.map(SendRow::into_domain).transpose()
    }

    async fn of_release(&self, release: &ReleaseId) -> Result<Vec<ReleaseSend>, DomainError> {
        let key = *release.as_uuid();
        let rows: Vec<SendRow> = self
            .isle
            .call(move |conn| {
                conn.prepare(&format!(
                    "SELECT {} FROM forge_send WHERE release_id = ?1 \
                     ORDER BY at DESC, id DESC",
                    SendRow::COLUMNS
                ))?
                .query_map(params![key], SendRow::from_row)?
                .collect::<rusqlite::Result<_>>()
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(SendRow::into_domain).collect()
    }
}
