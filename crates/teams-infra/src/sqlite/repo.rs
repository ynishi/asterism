//! The teams repository — state tables and the per-team ledger behind
//! one write rule.
//!
//! ## The same-tx rule is the only write API shape (#83 §2)
//!
//! Every public state-changing method here opens one transaction,
//! applies the state change **and** appends the corresponding ledger
//! event, and commits or rolls back the two together. There is no
//! public method that writes state without appending, and none that
//! appends without a state change. The one documented exception is
//! [`SqliteTeamsRepository::record_locator`]: locators are
//! private-space, and private-space operations never land in any
//! team's ledger (#83 §2 — the ledger's scope is the team boundary),
//! which is also why the v0 kind registry has no locator kind.
//!
//! ## Where the domain runs
//!
//! Invariants are evaluated by the domain, on current state, *inside*
//! the transaction that would record the change: the last-owner rule
//! is [`TeamRoster`]'s answer over the membership rows as they read
//! under the write lock, and role TEXT goes through [`Role::parse`] in
//! both directions. This is the deliberate exception to the
//! read-side convention (promotion outside the closure —
//! [`map`](crate::sqlite::map)): a check made outside the transaction
//! would be a check against state the transaction no longer holds.
//!
//! Domain refusals inside a closure travel as the inner `Result` of a
//! nested pair — the outer error is SQLite's, the inner is the
//! domain's, and the transaction rolls back on either.
//!
//! ## What `seq` means here
//!
//! Storage assigns it: `MAX(seq) + 1` per team, computed inside the
//! write transaction. The primary key makes it monotonic; the
//! single-writer deployment shape (#83 §4 — one server, one
//! connection, `BEGIN IMMEDIATE`) makes it gapless, because a
//! rolled-back transaction never leaves a hole behind — the next
//! append recomputes over what actually committed. The domain's
//! [`EventSeq`] validates what storage hands back and mints nothing.

use rusqlite::{Transaction, params};
use rusqlite_isle::AsyncIsle;
use teams_core::DomainError;
use teams_core::domain::identity::{LedgerActor, Membership, Role, TeamRoster};
use teams_core::domain::ledger::{
    BLOB_COPY_COMPLETED, EventKind, EventSeq, LedgerEvent, MEMBERSHIP_ADDED, MEMBERSHIP_REMOVED,
    ROLE_CHANGED, SubjectRef, TEAM_CREATED, TEAM_DELETED, is_v0_kind,
};
use teams_core::domain::store::{Locator, TeamBlobLink, parse_digest};
use uuid::Uuid;

use crate::sqlite::map::{
    actor_from_json, actor_to_json, infra_err, subject_from_ref, subject_to_ref,
};

/// What a closure hands back through the isle: the outer error is
/// SQLite's (rolls the transaction back by propagation), the inner is a
/// domain refusal (rolls it back explicitly).
type TxOutcome<T> = Result<Result<T, DomainError>, rusqlite::Error>;

/// SQLite repository for the teams plane — teams, memberships, blob
/// links, locators, and the append-only ledger, all through the write
/// rule in the module doc.
#[derive(Clone)]
pub struct SqliteTeamsRepository {
    isle: AsyncIsle,
}

impl SqliteTeamsRepository {
    /// Wraps a writer `AsyncIsle` handle (from
    /// [`open_and_migrate`](crate::sqlite::open_and_migrate) or its
    /// in-memory sibling).
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    // ------------------------------------------------------------------
    // Writes — one tx each: state change + ledger append, or neither.
    // ------------------------------------------------------------------

    /// Creates a team with its founding owner, appending
    /// `teams.team.created/1`.
    ///
    /// The founding row must be an **owner** row for this team: a team
    /// is never created ownerless (#83 §1 — every team has ≥1 owner at
    /// all times, and creation is where the first one lands). The
    /// founding owner is a [`Membership`] rather than being derived
    /// from `actor`, because the two are allowed to differ: under
    /// closed registration the operator creates the team, and the
    /// operator is never implicitly a member — the owner row belongs
    /// to whichever user will own it, while the ledger stamps whoever
    /// acted.
    pub async fn create_team(
        &self,
        team_id: Uuid,
        founding_owner: Membership,
        actor: LedgerActor,
        occurred_at_ms: i64,
    ) -> Result<LedgerEvent, DomainError> {
        if founding_owner.team_id != team_id {
            return Err(DomainError::Validation(format!(
                "founding owner row belongs to team {}, not {team_id}",
                founding_owner.team_id
            )));
        }
        if founding_owner.role != Role::Owner {
            return Err(DomainError::Validation(
                "the founding membership must be an owner row; \
                 a team is never created ownerless"
                    .into(),
            ));
        }
        self.write_tx(move |tx| {
            if team_exists_in_tx(tx, team_id)? {
                return Ok(Err(DomainError::Validation(format!(
                    "team {team_id} already exists"
                ))));
            }
            tx.execute(
                "INSERT INTO team (id, created_at) VALUES (?1, ?2)",
                params![team_id, occurred_at_ms],
            )?;
            tx.execute(
                "INSERT INTO membership (team_id, user_id, role) VALUES (?1, ?2, ?3)",
                params![
                    team_id,
                    founding_owner.user_id,
                    founding_owner.role.as_str()
                ],
            )?;
            append_event_in_tx(
                tx,
                team_id,
                &actor,
                occurred_at_ms,
                TEAM_CREATED,
                vec![SubjectRef::user(founding_owner.user_id)],
                serde_json::json!({
                    "founding_owner": founding_owner.user_id,
                    "role": founding_owner.role.as_str(),
                }),
            )
        })
        .await
    }

    /// Deletes a team, appending `teams.team.deleted/1`.
    ///
    /// Membership rows and blob links cascade away with the team row;
    /// the ledger survives by construction (no foreign key points from
    /// it to `team`), so the stream ends with the event that says why
    /// it ended. Whether the caller *may* delete is [`verb_allowed`]'s
    /// question and the server's to ask — this method enforces state
    /// invariants, not authority.
    ///
    /// [`verb_allowed`]: teams_core::domain::identity::verb_allowed
    pub async fn delete_team(
        &self,
        team_id: Uuid,
        actor: LedgerActor,
        occurred_at_ms: i64,
    ) -> Result<LedgerEvent, DomainError> {
        self.write_tx(move |tx| {
            let affected = tx.execute("DELETE FROM team WHERE id = ?1", params![team_id])?;
            if affected == 0 {
                return Ok(Err(DomainError::Validation(format!(
                    "team {team_id} does not exist"
                ))));
            }
            append_event_in_tx(
                tx,
                team_id,
                &actor,
                occurred_at_ms,
                TEAM_DELETED,
                vec![],
                serde_json::json!({}),
            )
        })
        .await
    }

    /// Adds a member, appending `teams.membership.added/1`.
    ///
    /// The candidate roster — current rows plus the new one — goes
    /// through [`TeamRoster::new`] inside the transaction, so a
    /// duplicate user is the domain's refusal, not a constraint
    /// error's.
    pub async fn add_member(
        &self,
        membership: Membership,
        actor: LedgerActor,
        occurred_at_ms: i64,
    ) -> Result<LedgerEvent, DomainError> {
        self.write_tx(move |tx| {
            let team_id = membership.team_id;
            if !team_exists_in_tx(tx, team_id)? {
                return Ok(Err(DomainError::Validation(format!(
                    "team {team_id} does not exist"
                ))));
            }
            let mut rows = match roster_rows_in_tx(tx, team_id)? {
                Ok(rows) => rows,
                Err(refused) => return Ok(Err(refused)),
            };
            rows.push(membership.clone());
            if let Err(refused) = TeamRoster::new(team_id, rows) {
                return Ok(Err(refused));
            }
            tx.execute(
                "INSERT INTO membership (team_id, user_id, role) VALUES (?1, ?2, ?3)",
                params![team_id, membership.user_id, membership.role.as_str()],
            )?;
            append_event_in_tx(
                tx,
                team_id,
                &actor,
                occurred_at_ms,
                MEMBERSHIP_ADDED,
                vec![SubjectRef::user(membership.user_id)],
                serde_json::json!({
                    "user_id": membership.user_id,
                    "role": membership.role.as_str(),
                }),
            )
        })
        .await
    }

    /// Removes a member — leaving and being removed are the same
    /// transition to the roster — appending
    /// `teams.membership.removed/1`.
    ///
    /// The last-owner rule is [`TeamRoster::check_remove`]'s answer
    /// over the rows as they read inside this transaction: the last
    /// owner cannot go, and the refusal rolls back before anything is
    /// written.
    pub async fn remove_member(
        &self,
        team_id: Uuid,
        user_id: Uuid,
        actor: LedgerActor,
        occurred_at_ms: i64,
    ) -> Result<LedgerEvent, DomainError> {
        self.write_tx(move |tx| {
            let roster = match roster_in_tx(tx, team_id)? {
                Ok(roster) => roster,
                Err(refused) => return Ok(Err(refused)),
            };
            if let Err(refused) = roster.check_remove(user_id) {
                return Ok(Err(refused));
            }
            let removed_role = roster
                .role_of(user_id)
                .expect("check_remove admitted the user, so the roster holds a role");
            tx.execute(
                "DELETE FROM membership WHERE team_id = ?1 AND user_id = ?2",
                params![team_id, user_id],
            )?;
            append_event_in_tx(
                tx,
                team_id,
                &actor,
                occurred_at_ms,
                MEMBERSHIP_REMOVED,
                vec![SubjectRef::user(user_id)],
                serde_json::json!({
                    "user_id": user_id,
                    "role": removed_role.as_str(),
                }),
            )
        })
        .await
    }

    /// Changes a member's role, appending
    /// `teams.membership.role_changed/1` with **both** the old and the
    /// new value in the payload (#83 §1 — the entry reads on its own).
    ///
    /// [`TeamRoster::check_role_change`] answers for the transition
    /// inside the transaction (self-demotion of the last owner is the
    /// case it exists for). A no-op change — the role the member
    /// already holds — is refused rather than recorded: an event with
    /// no state change is the shape the write rule forbids.
    pub async fn change_role(
        &self,
        team_id: Uuid,
        user_id: Uuid,
        new_role: Role,
        actor: LedgerActor,
        occurred_at_ms: i64,
    ) -> Result<LedgerEvent, DomainError> {
        self.write_tx(move |tx| {
            let roster = match roster_in_tx(tx, team_id)? {
                Ok(roster) => roster,
                Err(refused) => return Ok(Err(refused)),
            };
            if let Err(refused) = roster.check_role_change(user_id, new_role) {
                return Ok(Err(refused));
            }
            let old_role = roster
                .role_of(user_id)
                .expect("check_role_change admitted the user, so the roster holds a role");
            if old_role == new_role {
                return Ok(Err(DomainError::Validation(format!(
                    "user {user_id} already holds role {new_role} in team {team_id}; \
                     a no-op change would append an event with no state change"
                ))));
            }
            tx.execute(
                "UPDATE membership SET role = ?3 WHERE team_id = ?1 AND user_id = ?2",
                params![team_id, user_id, new_role.as_str()],
            )?;
            append_event_in_tx(
                tx,
                team_id,
                &actor,
                occurred_at_ms,
                ROLE_CHANGED,
                vec![SubjectRef::user(user_id)],
                serde_json::json!({
                    "user_id": user_id,
                    "old": old_role.as_str(),
                    "new": new_role.as_str(),
                }),
            )
        })
        .await
    }

    /// Records a blob link — the row that makes a digest exist for a
    /// team (#83 §3) — appending `teams.blob.copy_completed/1`.
    ///
    /// The bytes themselves are the next slice's concern
    /// (`LocalFileStorageAdapter`); the §3 ordering rule (bytes into
    /// the CAS first, then link row + ledger event in one tx) is why
    /// this method is only the second half and takes an
    /// already-validated [`TeamBlobLink`]. A duplicate link is refused:
    /// dedupe is decided server-side before this call, and a second
    /// row for the same `(team, digest)` would append an event for a
    /// copy that changed nothing.
    pub async fn add_blob_link(
        &self,
        link: TeamBlobLink,
        actor: LedgerActor,
        occurred_at_ms: i64,
    ) -> Result<LedgerEvent, DomainError> {
        self.write_tx(move |tx| {
            let team_id = link.team_id();
            if !team_exists_in_tx(tx, team_id)? {
                return Ok(Err(DomainError::Validation(format!(
                    "team {team_id} does not exist"
                ))));
            }
            let already: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM team_blob_link WHERE team_id = ?1 AND digest = ?2)",
                params![team_id, link.digest()],
                |row| row.get(0),
            )?;
            if already {
                return Ok(Err(DomainError::Validation(format!(
                    "digest {} is already linked to team {team_id}",
                    link.digest()
                ))));
            }
            let subject = match SubjectRef::blob(link.digest()) {
                Ok(subject) => subject,
                Err(refused) => return Ok(Err(refused)),
            };
            tx.execute(
                "INSERT INTO team_blob_link (team_id, digest, created_at) VALUES (?1, ?2, ?3)",
                params![team_id, link.digest(), occurred_at_ms],
            )?;
            append_event_in_tx(
                tx,
                team_id,
                &actor,
                occurred_at_ms,
                BLOB_COPY_COMPLETED,
                vec![subject],
                serde_json::json!({ "digest": link.digest() }),
            )
        })
        .await
    }

    /// Records (or refreshes) a locator — a private-space sighting.
    ///
    /// **The documented exception to the write rule**: no ledger event
    /// is appended, because the ledger's scope is the team boundary
    /// and private-space operations never land in any team's stream
    /// (#83 §2); accordingly the v0 kind registry has no locator kind.
    /// A re-sighting of the same `(user, uri)` updates the hint and
    /// the timestamp — a locator records the *last* sighting.
    pub async fn record_locator(&self, locator: Locator) -> Result<(), DomainError> {
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO locator (user_id, uri, digest_hint, seen_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (user_id, uri)
                     DO UPDATE SET digest_hint = excluded.digest_hint,
                                   seen_at = excluded.seen_at",
                    params![
                        locator.user_id,
                        locator.uri,
                        locator.digest_hint,
                        locator.seen_at_ms
                    ],
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Reads — promotion outside the closure (the map.rs convention).
    // ------------------------------------------------------------------

    /// Whether a team row exists.
    pub async fn team_exists(&self, team_id: Uuid) -> Result<bool, DomainError> {
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM team WHERE id = ?1)",
                    params![team_id],
                    |row| row.get(0),
                )
            })
            .await
            .map_err(infra_err)
    }

    /// The team's current membership set, every stored role passed
    /// back through [`Role::parse`] — a word the domain does not admit
    /// surfaces as a validation error instead of a guessed authority.
    pub async fn roster(&self, team_id: Uuid) -> Result<TeamRoster, DomainError> {
        let rows: Vec<(Uuid, String)> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT user_id, role FROM membership WHERE team_id = ?1 ORDER BY user_id",
                )?;
                let rows =
                    stmt.query_map(params![team_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
                rows.collect()
            })
            .await
            .map_err(infra_err)?;
        promote_roster(team_id, rows)
    }

    /// Whether `digest` is linked to `team_id` — the visibility
    /// question the blob read surface asks (#83 §3: a digest exists
    /// for a caller iff a link row sits in a team they belong to).
    /// The digest goes through the domain's parser first, so a
    /// malformed probe is a refusal, never a silent `false`.
    pub async fn blob_link_exists(&self, team_id: Uuid, digest: &str) -> Result<bool, DomainError> {
        let digest = parse_digest(digest)?;
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM team_blob_link
                     WHERE team_id = ?1 AND digest = ?2)",
                    params![team_id, digest],
                    |row| row.get(0),
                )
            })
            .await
            .map_err(infra_err)
    }

    /// The team's blob links, each re-validated through
    /// [`TeamBlobLink::new`] on the way out.
    pub async fn blob_links(&self, team_id: Uuid) -> Result<Vec<TeamBlobLink>, DomainError> {
        let digests: Vec<String> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT digest FROM team_blob_link WHERE team_id = ?1 ORDER BY digest",
                )?;
                let rows = stmt.query_map(params![team_id], |row| row.get(0))?;
                rows.collect()
            })
            .await
            .map_err(infra_err)?;
        digests
            .iter()
            .map(|digest| TeamBlobLink::new(team_id, digest))
            .collect()
    }

    /// A user's locators, most recently seen first.
    pub async fn locators_for_user(&self, user_id: Uuid) -> Result<Vec<Locator>, DomainError> {
        let rows: Vec<(String, Option<String>, i64)> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT uri, digest_hint, seen_at FROM locator
                     WHERE user_id = ?1 ORDER BY seen_at DESC, uri",
                )?;
                let rows = stmt.query_map(params![user_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
                rows.collect()
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(uri, hint, seen_at)| Locator::new(user_id, uri, hint.as_deref(), seen_at))
            .collect()
    }

    /// The team's stream in order — seq ascending, which storage makes
    /// insertion order.
    pub async fn events(&self, team_id: Uuid) -> Result<Vec<LedgerEvent>, DomainError> {
        let (events, subjects) = self
            .isle
            .call(move |conn| {
                let events = fetch_event_rows(
                    conn,
                    "SELECT seq, event_id, actor, occurred_at, kind, payload
                     FROM ledger_event WHERE team_id = ?1 ORDER BY seq",
                    params![team_id],
                )?;
                let subjects = fetch_subject_rows(
                    conn,
                    "SELECT seq, ref_type, ref_value FROM ledger_subject
                     WHERE team_id = ?1 ORDER BY seq, rowid",
                    params![team_id],
                )?;
                Ok((events, subjects))
            })
            .await
            .map_err(infra_err)?;
        promote_events(team_id, events, subjects)
    }

    /// The events in a team's stream that reference `subject` — the
    /// trace query, answered by walking the `(ref_type, ref_value)`
    /// index and never by parsing payload JSON (#83 §2).
    pub async fn events_for_subject(
        &self,
        team_id: Uuid,
        subject: &SubjectRef,
    ) -> Result<Vec<LedgerEvent>, DomainError> {
        let (ref_type, ref_value) = subject_to_ref(subject)?;
        let (events, subjects) = self
            .isle
            .call(move |conn| {
                let events = fetch_event_rows(
                    conn,
                    "SELECT e.seq, e.event_id, e.actor, e.occurred_at, e.kind, e.payload
                     FROM ledger_event e
                     JOIN (SELECT DISTINCT seq FROM ledger_subject
                           WHERE team_id = ?1 AND ref_type = ?2 AND ref_value = ?3) hits
                       ON hits.seq = e.seq
                     WHERE e.team_id = ?1 ORDER BY e.seq",
                    params![team_id, ref_type, ref_value],
                )?;
                let subjects = fetch_subject_rows(
                    conn,
                    "SELECT seq, ref_type, ref_value FROM ledger_subject
                     WHERE team_id = ?1 AND seq IN
                           (SELECT DISTINCT seq FROM ledger_subject
                            WHERE team_id = ?1 AND ref_type = ?2 AND ref_value = ?3)
                     ORDER BY seq, rowid",
                    params![team_id, ref_type, ref_value],
                )?;
                Ok((events, subjects))
            })
            .await
            .map_err(infra_err)?;
        promote_events(team_id, events, subjects)
    }

    /// Runs `f` inside one transaction: commit when it returns the
    /// inner `Ok`, roll back when it returns the inner `Err` (a domain
    /// refusal) or propagates a SQLite error.
    async fn write_tx<T, F>(&self, f: F) -> Result<T, DomainError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> TxOutcome<T> + Send + 'static,
    {
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                match f(&tx)? {
                    Ok(value) => {
                        tx.commit()?;
                        Ok(Ok(value))
                    }
                    Err(refused) => {
                        tx.rollback()?;
                        Ok(Err(refused))
                    }
                }
            })
            .await
            .map_err(infra_err)?
    }
}

// ----------------------------------------------------------------------
// In-transaction helpers.
// ----------------------------------------------------------------------

fn team_exists_in_tx(tx: &Transaction<'_>, team_id: Uuid) -> Result<bool, rusqlite::Error> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM team WHERE id = ?1)",
        params![team_id],
        |row| row.get(0),
    )
}

/// The membership rows as they read inside this transaction, promoted
/// through the domain (roles through [`Role::parse`]).
fn roster_rows_in_tx(tx: &Transaction<'_>, team_id: Uuid) -> TxOutcome<Vec<Membership>> {
    let mut stmt =
        tx.prepare("SELECT user_id, role FROM membership WHERE team_id = ?1 ORDER BY user_id")?;
    let rows: Vec<(Uuid, String)> = stmt
        .query_map(params![team_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut members = Vec::with_capacity(rows.len());
    for (user_id, role_text) in rows {
        match Role::parse(&role_text) {
            Ok(role) => members.push(Membership {
                user_id,
                team_id,
                role,
            }),
            Err(refused) => return Ok(Err(refused)),
        }
    }
    Ok(Ok(members))
}

/// The roster the invariants are evaluated over — current rows, under
/// this transaction's write lock.
fn roster_in_tx(tx: &Transaction<'_>, team_id: Uuid) -> TxOutcome<TeamRoster> {
    match roster_rows_in_tx(tx, team_id)? {
        Ok(rows) => Ok(TeamRoster::new(team_id, rows)),
        Err(refused) => Ok(Err(refused)),
    }
}

/// Appends one event to the team's stream inside the caller's
/// transaction: computes `MAX(seq) + 1`, has the domain validate every
/// part ([`EventSeq`] for the storage-assigned position, the writer's
/// registry check for the kind, [`LedgerEvent::new`] for the
/// envelope), then writes the event row and its subject index rows.
fn append_event_in_tx(
    tx: &Transaction<'_>,
    team_id: Uuid,
    actor: &LedgerActor,
    occurred_at_ms: i64,
    kind: &str,
    subjects: Vec<SubjectRef>,
    payload: serde_json::Value,
) -> TxOutcome<LedgerEvent> {
    let next: i64 = tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM ledger_event WHERE team_id = ?1",
        params![team_id],
        |row| row.get(0),
    )?;
    let seq = match EventSeq::new(next) {
        Ok(seq) => seq,
        Err(refused) => return Ok(Err(refused)),
    };
    let kind = match EventKind::parse(kind) {
        Ok(kind) => kind,
        Err(refused) => return Ok(Err(refused)),
    };
    // The writer's question, not the reader's: shape-valid kinds this
    // build does not register must not be appended by it (#83 §2).
    if !is_v0_kind(&kind) {
        return Ok(Err(DomainError::Validation(format!(
            "event kind {kind} is not in the v0 registry; this build does not write it"
        ))));
    }
    let event = match LedgerEvent::new(
        seq,
        Uuid::now_v7(),
        team_id,
        actor.clone(),
        occurred_at_ms,
        kind,
        subjects,
        payload,
    ) {
        Ok(event) => event,
        Err(refused) => return Ok(Err(refused)),
    };
    let actor_json = match actor_to_json(&event.actor) {
        Ok(json) => json,
        Err(refused) => return Ok(Err(refused)),
    };
    tx.execute(
        "INSERT INTO ledger_event (team_id, seq, event_id, actor, occurred_at, kind, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            team_id,
            next,
            event.event_id,
            actor_json,
            event.occurred_at_ms,
            event.kind.as_str(),
            event.payload.to_string()
        ],
    )?;
    for subject in &event.subjects {
        let (ref_type, ref_value) = match subject_to_ref(subject) {
            Ok(pair) => pair,
            Err(refused) => return Ok(Err(refused)),
        };
        tx.execute(
            "INSERT INTO ledger_subject (team_id, seq, ref_type, ref_value)
             VALUES (?1, ?2, ?3, ?4)",
            params![team_id, next, ref_type, ref_value],
        )?;
    }
    Ok(Ok(event))
}

// ----------------------------------------------------------------------
// Read-side row shapes and promotion (outside the closures).
// ----------------------------------------------------------------------

/// One `ledger_event` row as scanned — promotion into [`LedgerEvent`]
/// happens outside the closure.
struct EventRow {
    seq: i64,
    event_id: Uuid,
    actor: String,
    occurred_at: i64,
    kind: String,
    payload: String,
}

fn fetch_event_rows(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<EventRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        Ok(EventRow {
            seq: row.get(0)?,
            event_id: row.get(1)?,
            actor: row.get(2)?,
            occurred_at: row.get(3)?,
            kind: row.get(4)?,
            payload: row.get(5)?,
        })
    })?;
    rows.collect()
}

fn fetch_subject_rows(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<(i64, String, String)>, rusqlite::Error> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.collect()
}

fn promote_roster(team_id: Uuid, rows: Vec<(Uuid, String)>) -> Result<TeamRoster, DomainError> {
    let mut members = Vec::with_capacity(rows.len());
    for (user_id, role_text) in rows {
        members.push(Membership {
            user_id,
            team_id,
            role: Role::parse(&role_text)?,
        });
    }
    TeamRoster::new(team_id, members)
}

/// Reassembles envelopes from event rows and their subject index rows —
/// the index table is the only store of subjects, so the join *is* the
/// envelope's subject list, in insertion order.
fn promote_events(
    team_id: Uuid,
    rows: Vec<EventRow>,
    subject_rows: Vec<(i64, String, String)>,
) -> Result<Vec<LedgerEvent>, DomainError> {
    let mut events = Vec::with_capacity(rows.len());
    for row in rows {
        let subjects = subject_rows
            .iter()
            .filter(|(seq, _, _)| *seq == row.seq)
            .map(|(_, ref_type, ref_value)| subject_from_ref(ref_type, ref_value))
            .collect::<Result<Vec<_>, _>>()?;
        let payload: serde_json::Value = serde_json::from_str(&row.payload)
            .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt payload column: {e}")))?;
        events.push(LedgerEvent::new(
            EventSeq::new(row.seq)?,
            row.event_id,
            team_id,
            actor_from_json(&row.actor)?,
            row.occurred_at,
            EventKind::parse(&row.kind)?,
            subjects,
            payload,
        )?);
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite_isle::AsyncIsleDriver;
    use teams_core::domain::identity::ActorStamp;

    use crate::sqlite::open_and_migrate_in_memory;

    const T0: i64 = 1_755_000_000_000;

    async fn repo() -> (SqliteTeamsRepository, AsyncIsle, AsyncIsleDriver) {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        (SqliteTeamsRepository::new(isle.clone()), isle, driver)
    }

    fn actor_for(user_id: Uuid) -> LedgerActor {
        LedgerActor::member(ActorStamp {
            user_id,
            display_name: "Hoshino".into(),
        })
    }

    fn membership(team_id: Uuid, user_id: Uuid, role: Role) -> Membership {
        Membership {
            user_id,
            team_id,
            role,
        }
    }

    /// A team with one owner, created through the repository path.
    async fn team_with_owner(repo: &SqliteTeamsRepository) -> (Uuid, Uuid, LedgerActor) {
        let team_id = Uuid::now_v7();
        let owner_id = Uuid::now_v7();
        let actor = actor_for(owner_id);
        repo.create_team(
            team_id,
            membership(team_id, owner_id, Role::Owner),
            actor.clone(),
            T0,
        )
        .await
        .unwrap();
        (team_id, owner_id, actor)
    }

    /// A distinct well-formed digest per hex seed — spelled literally
    /// rather than hashed, because this crate deliberately has no
    /// asterism-* edge to hash with (#83 §4), and the shared notation
    /// is `sha256:` + 64 lowercase hex whichever side spells it. The
    /// domain constructors these strings pass through
    /// ([`TeamBlobLink::new`], [`SubjectRef::blob`]) are what keeps
    /// the spelling honest.
    fn digest_of(seed: char) -> String {
        assert!(seed.is_ascii_hexdigit() && !seed.is_ascii_uppercase());
        format!("sha256:{}", seed.to_string().repeat(64))
    }

    async fn ledger_row_count(isle: &AsyncIsle) -> i64 {
        isle.call(|conn| conn.query_row("SELECT count(*) FROM ledger_event", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_team_seeds_the_owner_and_appends_the_first_event() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, owner_id, _) = team_with_owner(&repo).await;

        assert!(repo.team_exists(team_id).await.unwrap());
        let roster = repo.roster(team_id).await.unwrap();
        assert_eq!(roster.owner_count(), 1);
        assert_eq!(roster.role_of(owner_id), Some(Role::Owner));

        let events = repo.events(team_id).await.unwrap();
        assert_eq!(events.len(), 1);
        let created = &events[0];
        assert_eq!(created.seq.get(), 1);
        assert_eq!(created.kind.as_str(), TEAM_CREATED);
        assert_eq!(created.payload["founding_owner"], owner_id.to_string());
        assert_eq!(created.subjects, vec![SubjectRef::user(owner_id)]);
        assert_eq!(created.occurred_at_ms, T0);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_team_is_never_created_ownerless_or_for_another_team() {
        let (repo, _isle, driver) = repo().await;
        let team_id = Uuid::now_v7();
        let user = Uuid::now_v7();

        // A member-roled founding row would create a zero-owner team.
        let refused = repo
            .create_team(
                team_id,
                membership(team_id, user, Role::Member),
                actor_for(user),
                T0,
            )
            .await;
        assert!(matches!(refused, Err(DomainError::Validation(_))));

        // A founding row for a different team is malformed.
        let refused = repo
            .create_team(
                team_id,
                membership(Uuid::now_v7(), user, Role::Owner),
                actor_for(user),
                T0,
            )
            .await;
        assert!(matches!(refused, Err(DomainError::Validation(_))));

        // Neither refusal left anything behind — no state, no event.
        assert!(!repo.team_exists(team_id).await.unwrap());
        assert!(repo.events(team_id).await.unwrap().is_empty());

        driver.shutdown().await.unwrap();
    }

    /// The done-criterion test: a failure induced *after* the state
    /// write rolls back both halves. The trigger fires on the ledger
    /// append, which the repository performs after the membership
    /// insert inside the same transaction — so a surviving membership
    /// row would mean the two halves can part ways.
    #[tokio::test]
    async fn an_induced_failure_after_the_state_write_rolls_back_both_halves() {
        let (repo, isle, driver) = repo().await;
        let (team_id, _, actor) = team_with_owner(&repo).await;
        let bob = Uuid::now_v7();

        isle.call(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER induced_failure
                 BEFORE INSERT ON ledger_event
                 WHEN new.kind = 'teams.membership.added/1'
                 BEGIN SELECT RAISE(ABORT, 'induced failure'); END;",
            )
        })
        .await
        .unwrap();

        let failed = repo
            .add_member(
                membership(team_id, bob, Role::Member),
                actor.clone(),
                T0 + 1,
            )
            .await;
        assert!(
            matches!(&failed, Err(DomainError::Infra(e)) if e.to_string().contains("induced")),
            "the induced abort must surface as an infra error: {failed:?}"
        );

        // Both halves rolled back: no membership row, no event.
        let roster = repo.roster(team_id).await.unwrap();
        assert_eq!(roster.role_of(bob), None);
        assert_eq!(repo.events(team_id).await.unwrap().len(), 1);

        // And the failed attempt left no hole: with the trigger gone,
        // the next append lands at seq 2.
        isle.call(|conn| conn.execute_batch("DROP TRIGGER induced_failure"))
            .await
            .unwrap();
        let added = repo
            .add_member(membership(team_id, bob, Role::Member), actor, T0 + 2)
            .await
            .unwrap();
        assert_eq!(added.seq.get(), 2);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn seq_is_monotonic_and_gapless_within_each_team() {
        let (repo, _isle, driver) = repo().await;
        let (team_a, _, actor_a) = team_with_owner(&repo).await;
        let (team_b, _, actor_b) = team_with_owner(&repo).await;

        // Interleave writes across the two streams.
        let bob = Uuid::now_v7();
        let carol = Uuid::now_v7();
        repo.add_member(membership(team_a, bob, Role::Member), actor_a.clone(), T0)
            .await
            .unwrap();
        repo.add_member(membership(team_b, carol, Role::Member), actor_b, T0)
            .await
            .unwrap();
        repo.change_role(team_a, bob, Role::Owner, actor_a.clone(), T0 + 1)
            .await
            .unwrap();
        repo.remove_member(team_a, bob, actor_a, T0 + 2)
            .await
            .unwrap();

        let seqs_a: Vec<i64> = repo
            .events(team_a)
            .await
            .unwrap()
            .iter()
            .map(|e| e.seq.get())
            .collect();
        let seqs_b: Vec<i64> = repo
            .events(team_b)
            .await
            .unwrap()
            .iter()
            .map(|e| e.seq.get())
            .collect();
        assert_eq!(seqs_a, vec![1, 2, 3, 4], "team A's stream is 1..=n");
        assert_eq!(seqs_b, vec![1, 2], "team B's stream counts alone");

        driver.shutdown().await.unwrap();
    }

    /// The repository API has no update/delete on the ledger — that is
    /// its surface, checkable by reading it. This pins the schema half:
    /// raw SQL around the API is aborted by the triggers.
    #[tokio::test]
    async fn the_schema_refuses_update_and_delete_on_the_ledger() {
        let (repo, isle, driver) = repo().await;
        let (team_id, _, _) = team_with_owner(&repo).await;

        for sql in [
            "UPDATE ledger_event SET payload = '{}'",
            "DELETE FROM ledger_event",
            "UPDATE ledger_subject SET ref_value = 'x'",
            "DELETE FROM ledger_subject",
        ] {
            let sql_owned = sql.to_string();
            let refused = isle.call(move |conn| conn.execute(&sql_owned, [])).await;
            assert!(
                matches!(&refused, Err(e) if e.to_string().contains("append-only")),
                "{sql} must abort: {refused:?}"
            );
        }
        // The stream is intact.
        assert_eq!(repo.events(team_id).await.unwrap().len(), 1);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_last_owner_is_refused_through_the_repository_path() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, owner_id, actor) = team_with_owner(&repo).await;
        let bob = Uuid::now_v7();
        repo.add_member(membership(team_id, bob, Role::Member), actor.clone(), T0)
            .await
            .unwrap();
        let events_before = repo.events(team_id).await.unwrap().len();

        // Both phrasings of the departure, and the self-demotion.
        let removed = repo
            .remove_member(team_id, owner_id, actor.clone(), T0)
            .await;
        assert!(matches!(removed, Err(DomainError::LastOwner { team_id: t }) if t == team_id));
        let demoted = repo
            .change_role(team_id, owner_id, Role::Member, actor.clone(), T0)
            .await;
        assert!(matches!(demoted, Err(DomainError::LastOwner { team_id: t }) if t == team_id));

        // Refusals write nothing: state and stream both unchanged.
        let roster = repo.roster(team_id).await.unwrap();
        assert_eq!(roster.role_of(owner_id), Some(Role::Owner));
        assert_eq!(repo.events(team_id).await.unwrap().len(), events_before);

        // With a second owner the same departure is ordinary.
        repo.change_role(team_id, bob, Role::Owner, actor.clone(), T0 + 1)
            .await
            .unwrap();
        repo.remove_member(team_id, owner_id, actor, T0 + 2)
            .await
            .unwrap();
        let roster = repo.roster(team_id).await.unwrap();
        assert_eq!(roster.role_of(owner_id), None);
        assert_eq!(roster.owner_count(), 1);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_role_change_records_old_and_new_and_a_no_op_is_refused() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, _, actor) = team_with_owner(&repo).await;
        let bob = Uuid::now_v7();
        repo.add_member(membership(team_id, bob, Role::Member), actor.clone(), T0)
            .await
            .unwrap();

        let changed = repo
            .change_role(team_id, bob, Role::Owner, actor.clone(), T0 + 1)
            .await
            .unwrap();
        assert_eq!(changed.kind.as_str(), ROLE_CHANGED);
        assert_eq!(changed.payload["old"], "member");
        assert_eq!(changed.payload["new"], "owner");
        assert_eq!(
            repo.roster(team_id).await.unwrap().role_of(bob),
            Some(Role::Owner)
        );

        // The role bob already holds: an event would record no change,
        // so the write rule refuses it.
        let events_before = repo.events(team_id).await.unwrap().len();
        let noop = repo
            .change_role(team_id, bob, Role::Owner, actor, T0 + 2)
            .await;
        assert!(matches!(noop, Err(DomainError::Validation(_))));
        assert_eq!(repo.events(team_id).await.unwrap().len(), events_before);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn membership_round_trips_and_a_duplicate_is_the_domains_refusal() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, owner_id, actor) = team_with_owner(&repo).await;
        let bob = Uuid::now_v7();

        let added = repo
            .add_member(membership(team_id, bob, Role::Member), actor.clone(), T0)
            .await
            .unwrap();
        assert_eq!(added.kind.as_str(), MEMBERSHIP_ADDED);
        assert_eq!(added.subjects, vec![SubjectRef::user(bob)]);

        let roster = repo.roster(team_id).await.unwrap();
        assert_eq!(roster.role_of(owner_id), Some(Role::Owner));
        assert_eq!(roster.role_of(bob), Some(Role::Member));

        let duplicate = repo
            .add_member(membership(team_id, bob, Role::Owner), actor.clone(), T0)
            .await;
        assert!(matches!(duplicate, Err(DomainError::Validation(_))));

        let removed = repo
            .remove_member(team_id, bob, actor, T0 + 1)
            .await
            .unwrap();
        assert_eq!(removed.kind.as_str(), MEMBERSHIP_REMOVED);
        assert_eq!(removed.payload["role"], "member");
        assert_eq!(repo.roster(team_id).await.unwrap().role_of(bob), None);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn blob_links_round_trip_and_refuse_duplicates_and_unknown_teams() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, _, actor) = team_with_owner(&repo).await;
        let digest = digest_of('a');
        let link = TeamBlobLink::new(team_id, &digest).unwrap();

        let copied = repo
            .add_blob_link(link.clone(), actor.clone(), T0)
            .await
            .unwrap();
        assert_eq!(copied.kind.as_str(), BLOB_COPY_COMPLETED);
        assert_eq!(copied.subjects, vec![SubjectRef::blob(&digest).unwrap()]);

        let links = repo.blob_links(team_id).await.unwrap();
        assert_eq!(links, vec![link.clone()]);

        let duplicate = repo.add_blob_link(link, actor.clone(), T0 + 1).await;
        assert!(matches!(duplicate, Err(DomainError::Validation(_))));

        let stranger = TeamBlobLink::new(Uuid::now_v7(), &digest).unwrap();
        let unknown = repo.add_blob_link(stranger, actor, T0 + 1).await;
        assert!(matches!(unknown, Err(DomainError::Validation(_))));

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_locator_round_trips_re_sights_and_never_touches_the_ledger() {
        let (repo, isle, driver) = repo().await;
        let user = Uuid::now_v7();
        let first = digest_of('1');
        let second = digest_of('2');
        let ledger_before = ledger_row_count(&isle).await;

        repo.record_locator(Locator::new(user, "file:///tmp/a.png", Some(&first), T0).unwrap())
            .await
            .unwrap();
        repo.record_locator(Locator::new(user, "file:///tmp/b.png", None, T0 + 1).unwrap())
            .await
            .unwrap();

        let locators = repo.locators_for_user(user).await.unwrap();
        assert_eq!(locators.len(), 2);
        assert_eq!(locators[0].uri, "file:///tmp/b.png");
        assert_eq!(locators[1].digest_hint.as_deref(), Some(first.as_str()));

        // A re-sighting of the same (user, uri) updates, not duplicates.
        repo.record_locator(
            Locator::new(user, "file:///tmp/a.png", Some(&second), T0 + 5).unwrap(),
        )
        .await
        .unwrap();
        let locators = repo.locators_for_user(user).await.unwrap();
        assert_eq!(locators.len(), 2);
        assert_eq!(locators[0].uri, "file:///tmp/a.png");
        assert_eq!(locators[0].digest_hint.as_deref(), Some(second.as_str()));
        assert_eq!(locators[0].seen_at_ms, T0 + 5);

        // Private-space operations land in no team's ledger (#83 §2).
        assert_eq!(ledger_row_count(&isle).await, ledger_before);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn trace_queries_walk_the_subjects_index_by_ref() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, owner_id, actor) = team_with_owner(&repo).await;
        let linked = digest_of('b');
        let unlinked = digest_of('c');
        repo.add_blob_link(
            TeamBlobLink::new(team_id, &linked).unwrap(),
            actor.clone(),
            T0,
        )
        .await
        .unwrap();
        let bob = Uuid::now_v7();
        repo.add_member(
            membership(team_id, bob, Role::Member),
            actor.clone(),
            T0 + 1,
        )
        .await
        .unwrap();
        repo.remove_member(team_id, bob, actor, T0 + 2)
            .await
            .unwrap();

        // By blob: exactly the copy event.
        let hits = repo
            .events_for_subject(team_id, &SubjectRef::blob(&linked).unwrap())
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind.as_str(), BLOB_COPY_COMPLETED);
        assert_eq!(hits[0].subjects, vec![SubjectRef::blob(&linked).unwrap()]);

        // By user: bob's whole story, in stream order.
        let hits = repo
            .events_for_subject(team_id, &SubjectRef::user(bob))
            .await
            .unwrap();
        let kinds: Vec<&str> = hits.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec![MEMBERSHIP_ADDED, MEMBERSHIP_REMOVED]);

        // The founding owner's ref reaches the creation event; a digest
        // nothing referenced reaches nothing.
        let hits = repo
            .events_for_subject(team_id, &SubjectRef::user(owner_id))
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind.as_str(), TEAM_CREATED);
        let hits = repo
            .events_for_subject(team_id, &SubjectRef::blob(&unlinked).unwrap())
            .await
            .unwrap();
        assert!(hits.is_empty());

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn stored_role_text_passes_back_through_the_domain_on_read() {
        let (repo, isle, driver) = repo().await;
        let (team_id, _, _) = team_with_owner(&repo).await;

        // A word the domain does not admit, planted around the API.
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO membership (team_id, user_id, role) VALUES (?1, ?2, 'admin')",
                params![team_id, Uuid::now_v7()],
            )
        })
        .await
        .unwrap();

        let read = repo.roster(team_id).await;
        assert!(
            matches!(&read, Err(DomainError::Validation(_))),
            "an unknown role word must be refused, not guessed at: {read:?}"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn deleting_a_team_cascades_state_and_the_stream_survives() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, _, actor) = team_with_owner(&repo).await;
        let digest = digest_of('d');
        repo.add_blob_link(
            TeamBlobLink::new(team_id, &digest).unwrap(),
            actor.clone(),
            T0,
        )
        .await
        .unwrap();

        let deleted = repo.delete_team(team_id, actor, T0 + 1).await.unwrap();
        assert_eq!(deleted.kind.as_str(), TEAM_DELETED);

        // The state is gone…
        assert!(!repo.team_exists(team_id).await.unwrap());
        assert_eq!(repo.roster(team_id).await.unwrap().owner_count(), 0);
        assert!(repo.blob_links(team_id).await.unwrap().is_empty());

        // …and the record survives it, ending with the event that says
        // why (#83 §2: the ledger keeps what the state no longer holds).
        let events = repo.events(team_id).await.unwrap();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec![TEAM_CREATED, BLOB_COPY_COMPLETED, TEAM_DELETED]);

        // A second delete has no state to change, so it appends nothing.
        let again = repo
            .delete_team(team_id, actor_for(Uuid::now_v7()), T0 + 2)
            .await;
        assert!(matches!(again, Err(DomainError::Validation(_))));
        assert_eq!(repo.events(team_id).await.unwrap().len(), 3);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn an_operator_action_reads_back_as_the_operator() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, _, _) = team_with_owner(&repo).await;

        let operator =
            teams_core::domain::identity::InstanceOperator::new(Uuid::now_v7(), "op").unwrap();
        repo.delete_team(team_id, LedgerActor::operator(&operator), T0 + 1)
            .await
            .unwrap();

        let events = repo.events(team_id).await.unwrap();
        let last = events.last().unwrap();
        assert!(
            last.actor.is_operator(),
            "the operator's delete must read back operator-stamped, never disguised (#83 §1)"
        );

        driver.shutdown().await.unwrap();
    }
}
