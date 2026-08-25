//! Storage for captured projections (#148 decisions 12 and 14).
//!
//! **Its own module, beside the forge rather than inside it.** Decision
//! 12 puts the projection outside the forge, and where the code lives
//! is part of how that stays true: nothing here goes through
//! [`TeamForge`](crate::sqlite::forge::TeamForge), touches a forge
//! table, or appends to the ledger. What connects the two is that the
//! push handler calls both, in that order, which is the whole of the
//! coupling.
//!
//! ## Why the write is not in the forge's transaction
//!
//! The forge's writes and their ledger events share one transaction
//! because #83 §2 makes the event the receipt for the write — two
//! independently writable truths is the one forbidden arrangement, and
//! same-tx is what forecloses it. A projection is not in that
//! relationship with anything: decision 12 makes it losable, which is
//! what makes a separate write correct here rather than merely
//! tolerable.
//!
//! [`capture`](SqliteProjectionStore::capture) therefore opens its own
//! transaction and never appends to the ledger. When it runs relative
//! to the push is not this file's rule and is argued where it can be
//! broken — `teams_server::forge::push_round`.
//!
//! ## Nothing here reads the body
//!
//! It arrives as a
//! [`ProjectionBody`](teams_core::domain::projection::ProjectionBody),
//! goes into a `TEXT` column, and comes back out. There is no parse,
//! no index and no column lifted out of it — decision 14's check
//! applied to the one file that would be tempted to break it.

use rusqlite::{OptionalExtension as _, params};
use rusqlite_isle::AsyncIsle;
use teams_core::DomainError;
use teams_core::domain::projection::{EntryProjection, ProjectionBody};
use uuid::Uuid;

use crate::sqlite::map::infra_err;

/// The `asset_projection` table, and nothing else.
#[derive(Clone)]
pub struct SqliteProjectionStore {
    isle: AsyncIsle,
}

impl SqliteProjectionStore {
    /// Wraps the teams database's writer isle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    /// Captures what a push said about a set of entries, replacing
    /// whatever an earlier push said about the same ones.
    ///
    /// Replaced rather than appended: decision 12 makes the projection
    /// what the promoter said *at the time*, and the time that counts
    /// is the most recent push, which is a forge op — the only thing
    /// permitted to replace one. What was said before is not lost from
    /// the record, because each push has its own ledger event; it is
    /// lost from this table, which holds the present.
    ///
    /// One transaction for the batch, so a round's descriptions land
    /// together or not at all. That is not the forge's same-tx rule
    /// reaching over here — see the module doc — it is the smaller
    /// claim that one push's projections are one write.
    pub async fn capture(
        &self,
        team_id: Uuid,
        line_id: Uuid,
        promoted_by: Uuid,
        pushed_at_ms: i64,
        entries: Vec<(Uuid, u32, ProjectionBody)>,
    ) -> Result<(), DomainError> {
        if entries.is_empty() {
            return Ok(());
        }
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                {
                    let mut stmt = tx.prepare(
                        "INSERT INTO asset_projection
                             (line_id, entry_id, team_id, version, body, promoted_by, pushed_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT (line_id, entry_id) DO UPDATE SET
                             version     = excluded.version,
                             body        = excluded.body,
                             promoted_by = excluded.promoted_by,
                             pushed_at   = excluded.pushed_at
                         WHERE asset_projection.team_id = excluded.team_id",
                    )?;
                    for (entry_id, version, body) in entries {
                        stmt.execute(params![
                            line_id,
                            entry_id,
                            team_id,
                            version,
                            body.as_str(),
                            promoted_by,
                            pushed_at_ms,
                        ])?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    /// What was said about one entry, or nothing.
    ///
    /// **Scoped to the team, and that filter is load-bearing.** A line
    /// id is unique across teams, so `(line, entry)` alone would find
    /// the row — for whichever team holds it. The caller arrives
    /// holding one team's session, so a row belonging to another
    /// answers as absent here, which is the only answer that does not
    /// hand one team's promoter a description written inside another.
    ///
    /// Absent is otherwise an ordinary answer: an entry may have been
    /// named by a client that captured no description, and a
    /// projection may have been lost.
    pub async fn find(
        &self,
        team_id: Uuid,
        line_id: Uuid,
        entry_id: Uuid,
    ) -> Result<Option<EntryProjection>, DomainError> {
        let row: Option<(u32, String, Uuid, i64)> = self
            .isle
            .call(move |conn| {
                let found = conn
                    .query_row(
                        "SELECT version, body, promoted_by, pushed_at
                           FROM asset_projection
                          WHERE line_id = ?1 AND entry_id = ?2 AND team_id = ?3",
                        params![line_id, entry_id, team_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .optional()?;
                Ok(found)
            })
            .await
            .map_err(infra_err)?;
        row.map(|(version, body, promoted_by, pushed_at_ms)| {
            Ok(EntryProjection {
                line_id,
                entry_id,
                team_id,
                version,
                body: ProjectionBody::parse(body)?,
                promoted_by,
                pushed_at_ms,
            })
        })
        .transpose()
    }
}
