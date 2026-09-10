//! SQLite adapter for the `ReleaseRepository` port.
//!
//! `forge_release` holds what was released, by whom and when;
//! `forge_release_file` holds what the disclosure writer reported for
//! each copy that left, in the order the run wrote them. Reads join the
//! two, on the same terms the snapshot adapter joins its membership: a
//! release read without its stamps is a release that cannot answer the
//! question it exists to answer.
//!
//! # Why this is not on `SqliteForge`
//!
//! That adapter satisfies the forge's four ports and is stated in the
//! forge's own words. This port is not one of them — a release names a
//! snapshot and a dispatch, which the forge may not
//! ([`asterism_core::domain::release`]) — so putting it there would
//! give one type two vocabularies and no reason to have both.
//!
//! What it does borrow is [`ActRow`], because an act is stored the same
//! way wherever it appears: three columns, and a kind this build refuses
//! to widen by guessing.
//!
//! # The two halves of a stamp, as columns
//!
//! [`Half`] is three states, and the third carries words. So each half
//! is a state column and a detail column, and the table's own `CHECK`
//! says a state that is not `written` has to carry one — a skipped half
//! with no reason and a failed half with no cause are the values the
//! type exists to refuse, and a row is where they would come back.

use asterism_core::domain::disclosure::{Half, Skipped, Stamped};
use asterism_core::domain::forge::model::value::{ChangePointId, LineId};
use asterism_core::domain::release::{FileStamp, Release};
use asterism_core::domain::repository::ReleaseRepository;
use asterism_core::domain::value::{AssetId, DispatchId, ReleaseId, SnapshotId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::{Connection, params};
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::forge::rows::ActRow;
use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `ReleaseRepository`.
#[derive(Clone)]
pub struct SqliteReleaseRepository {
    isle: AsyncIsle,
}

impl SqliteReleaseRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// One head row, as the columns hold it.
struct ReleaseRow {
    id: Uuid,
    line: Uuid,
    change_point: Uuid,
    snapshot: Uuid,
    dispatch: Uuid,
    at: i64,
    actor: Uuid,
    actor_kind: String,
}

impl ReleaseRow {
    const COLUMNS: &'static str =
        "id, line_id, change_point, snapshot_id, dispatch_id, at, actor_id, actor_kind";

    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            line: row.get(1)?,
            change_point: row.get(2)?,
            snapshot: row.get(3)?,
            dispatch: row.get(4)?,
            at: row.get(5)?,
            actor: row.get(6)?,
            actor_kind: row.get(7)?,
        })
    }

    fn into_domain(self, files: Vec<FileStamp>) -> Result<Release, DomainError> {
        let kind = match self.actor_kind.as_str() {
            "user" => "user",
            "system" => "system",
            other => {
                return Err(DomainError::Validation(format!(
                    "a stored release names an actor kind this model does not have: {other}"
                )));
            }
        };
        let act = ActRow {
            at: ms_to_datetime(self.at)?,
            actor: asterism_core::domain::forge::model::value::ActorId::from_uuid(self.actor),
            kind,
        }
        .read()?;
        Ok(Release::restored(
            ReleaseId::from_uuid(self.id),
            LineId::from_uuid(self.line),
            ChangePointId::from_uuid(self.change_point),
            SnapshotId::from_uuid(self.snapshot),
            DispatchId::from_uuid(self.dispatch),
            act,
            files,
        ))
    }
}

/// One half of a stamp, as the two columns hold it.
///
/// The detail column is `NULL` only for [`Half::Written`], which is the
/// one state that has nothing to say beyond itself.
fn half_columns(half: &Half) -> (&'static str, Option<String>) {
    match half {
        Half::Written => ("written", None),
        Half::Skipped(why) => ("skipped", Some(why.as_str().to_string())),
        Half::Failed(cause) => ("failed", Some(cause.clone())),
    }
}

/// The half those two columns describe.
///
/// No wildcard arm on the reason: a skipped half whose recorded reason
/// this build does not have is a row it cannot read, and coercing it
/// into one of the reasons it does have would report a certificate that
/// was never configured, or a container that can carry anything.
fn read_half(state: &str, detail: Option<String>) -> Result<Half, DomainError> {
    match (state, detail) {
        ("written", _) => Ok(Half::Written),
        ("failed", Some(cause)) => Ok(Half::Failed(cause)),
        ("skipped", Some(reason)) => match reason.as_str() {
            "nothing_to_disclose" => Ok(Half::Skipped(Skipped::NothingToDisclose)),
            "container_cannot_carry_it" => Ok(Half::Skipped(Skipped::ContainerCannotCarryIt)),
            "no_signing_identity" => Ok(Half::Skipped(Skipped::NoSigningIdentity)),
            other => Err(DomainError::Validation(format!(
                "a stored stamp was skipped for a reason this model does not have: {other}"
            ))),
        },
        (state, detail) => Err(DomainError::Validation(format!(
            "a stored stamp reads {state:?} with detail {detail:?}, which is not a state \
             this model has"
        ))),
    }
}

/// The stamps of one release, in the order they were written.
fn load_files(conn: &Connection, release: Uuid) -> rusqlite::Result<Vec<StampRow>> {
    let mut stmt = conn.prepare(
        "SELECT asset_id, path, xmp_state, xmp_detail, manifest_state, manifest_detail, \
                prompt_dropped, system_dropped \
           FROM forge_release_file WHERE release_id = ?1 ORDER BY position",
    )?;
    stmt.query_map(params![release], |row| {
        Ok(StampRow {
            asset: row.get(0)?,
            path: row.get(1)?,
            xmp_state: row.get(2)?,
            xmp_detail: row.get(3)?,
            manifest_state: row.get(4)?,
            manifest_detail: row.get(5)?,
            prompt_dropped: row.get::<_, i64>(6)? != 0,
            system_dropped: row.get::<_, i64>(7)? != 0,
        })
    })?
    .collect()
}

/// One stamp, as the columns hold it.
struct StampRow {
    asset: Uuid,
    path: String,
    xmp_state: String,
    xmp_detail: Option<String>,
    manifest_state: String,
    manifest_detail: Option<String>,
    prompt_dropped: bool,
    system_dropped: bool,
}

impl StampRow {
    fn into_domain(self) -> Result<FileStamp, DomainError> {
        let mut outcome = Stamped::new(
            read_half(&self.xmp_state, self.xmp_detail)?,
            read_half(&self.manifest_state, self.manifest_detail)?,
        );
        outcome.prompt_dropped = self.prompt_dropped;
        outcome.system_dropped = self.system_dropped;
        Ok(FileStamp {
            asset: AssetId::from_uuid(self.asset),
            path: self.path,
            outcome,
        })
    }
}

/// A head row and its stamps, made into a value.
fn built(row: ReleaseRow, stamps: Vec<StampRow>) -> Result<Release, DomainError> {
    let files = stamps
        .into_iter()
        .map(StampRow::into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    row.into_domain(files)
}

#[async_trait]
impl ReleaseRepository for SqliteReleaseRepository {
    async fn record(&self, release: &Release) -> Result<(), DomainError> {
        let id = *release.id().as_uuid();
        let line = *release.line().as_uuid();
        let point = *release.change_point().as_uuid();
        let snapshot = *release.snapshot().as_uuid();
        let dispatch = *release.dispatch().as_uuid();
        let act = ActRow::of(release.act());
        let at = datetime_to_ms(&act.at);
        let actor = *act.actor.as_uuid();
        let kind = act.kind;
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO forge_release \
                         (id, line_id, change_point, snapshot_id, dispatch_id, \
                          at, actor_id, actor_kind) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id, line, point, snapshot, dispatch, at, actor, kind],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn find(&self, id: &ReleaseId) -> Result<Option<Release>, DomainError> {
        self.one(
            format!(
                "SELECT {} FROM forge_release WHERE id = ?1",
                ReleaseRow::COLUMNS
            ),
            *id.as_uuid(),
        )
        .await
    }

    async fn by_dispatch(&self, dispatch: &DispatchId) -> Result<Option<Release>, DomainError> {
        self.one(
            format!(
                "SELECT {} FROM forge_release WHERE dispatch_id = ?1",
                ReleaseRow::COLUMNS
            ),
            *dispatch.as_uuid(),
        )
        .await
    }

    async fn of_change_point(
        &self,
        change_point: &ChangePointId,
    ) -> Result<Vec<Release>, DomainError> {
        let point = *change_point.as_uuid();
        let loaded: Vec<(ReleaseRow, Vec<StampRow>)> = self
            .isle
            .call(move |conn| {
                let heads: Vec<ReleaseRow> = conn
                    .prepare(&format!(
                        "SELECT {} FROM forge_release WHERE change_point = ?1 \
                         ORDER BY at DESC, id DESC",
                        ReleaseRow::COLUMNS
                    ))?
                    .query_map(params![point], ReleaseRow::from_row)?
                    .collect::<rusqlite::Result<_>>()?;
                let mut out = Vec::with_capacity(heads.len());
                for head in heads {
                    let stamps = load_files(conn, head.id)?;
                    out.push((head, stamps));
                }
                Ok(out)
            })
            .await
            .map_err(infra_err)?;
        loaded
            .into_iter()
            .map(|(row, stamps)| built(row, stamps))
            .collect()
    }

    async fn note_files(&self, id: &ReleaseId, files: &[FileStamp]) -> Result<(), DomainError> {
        let release = *id.as_uuid();
        let rows: Vec<_> = files
            .iter()
            .map(|file| {
                let (xmp_state, xmp_detail) = half_columns(&file.outcome.xmp);
                let (manifest_state, manifest_detail) = half_columns(&file.outcome.manifest);
                (
                    *file.asset.as_uuid(),
                    file.path.clone(),
                    xmp_state,
                    xmp_detail,
                    manifest_state,
                    manifest_detail,
                    i64::from(file.outcome.prompt_dropped),
                    i64::from(file.outcome.system_dropped),
                )
            })
            .collect();
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                // A second pass over the same files is a correction of
                // the first rather than more of it, so what was there
                // goes before what is written now.
                tx.execute(
                    "DELETE FROM forge_release_file WHERE release_id = ?1",
                    params![release],
                )?;
                {
                    let mut stmt = tx.prepare(
                        "INSERT INTO forge_release_file \
                             (release_id, position, asset_id, path, xmp_state, xmp_detail, \
                              manifest_state, manifest_detail, prompt_dropped, system_dropped) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    )?;
                    for (position, row) in rows.into_iter().enumerate() {
                        stmt.execute(params![
                            release,
                            position as i64,
                            row.0,
                            row.1,
                            row.2,
                            row.3,
                            row.4,
                            row.5,
                            row.6,
                            row.7,
                        ])?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}

impl SqliteReleaseRepository {
    /// One release by whichever unique column the query names.
    async fn one(&self, sql: String, key: Uuid) -> Result<Option<Release>, DomainError> {
        let loaded: Option<(ReleaseRow, Vec<StampRow>)> = self
            .isle
            .call(move |conn| {
                let head = match conn.query_row(&sql, params![key], ReleaseRow::from_row) {
                    Ok(row) => row,
                    Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                    Err(err) => return Err(err),
                };
                let stamps = load_files(conn, head.id)?;
                Ok(Some((head, stamps)))
            })
            .await
            .map_err(infra_err)?;
        loaded.map(|(row, stamps)| built(row, stamps)).transpose()
    }
}
