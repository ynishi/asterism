//! The teams repository — state tables and the per-team ledger behind
//! one write rule.
//!
//! ## The same-tx rule is the only write API shape (#83 §2)
//!
//! Every public state-changing method here opens one transaction,
//! applies the state change **and** appends the corresponding ledger
//! event, and commits or rolls back the two together. There is no
//! public method that writes state without appending, and none that
//! appends without a state change. The documented exceptions are the
//! writes outside the ledger's scope, which #83 §2 fixes at the team
//! boundary: [`SqliteTeamsRepository::record_locator`] (locators are
//! private-space, which is also why the v0 kind registry has no
//! locator kind) and
//! [`SqliteTeamsRepository::publish_head_entry`] (the head registry
//! is instance-scope — #132 — and carries its history in its own
//! superseded rows).
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

use std::collections::BTreeSet;

use rusqlite::{OptionalExtension as _, Transaction, params};
use rusqlite_isle::AsyncIsle;
use teams_core::DomainError;
use teams_core::domain::head_registry::TagHeadEntry;
use teams_core::domain::identity::{LedgerActor, Membership, Role, TeamMembership, TeamRoster};
use teams_core::domain::ledger::{
    BLOB_COPY_COMPLETED, BLOB_LINK_PURGE_MARKED, BLOB_LINK_PURGE_UNMARKED, BLOB_LINK_RECLAIMED,
    EventKind, EventSeq, LedgerEvent, MEMBERSHIP_ADDED, MEMBERSHIP_REMOVED, ROLE_CHANGED,
    SubjectRef, TEAM_CREATED, TEAM_DELETED, is_registered_kind,
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

    /// The handle this repository writes through.
    ///
    /// The hosted forge is built per request over the same one
    /// ([`TeamForge::for_request`](crate::sqlite::forge::TeamForge::for_request)),
    /// and it has to be the same one: decision 17 puts a forge write
    /// and its ledger event in a single transaction, which two handles
    /// on two connections could not be. So the transport takes it from
    /// here rather than carrying a second copy that nothing checks
    /// against this one.
    pub fn isle(&self) -> AsyncIsle {
        self.isle.clone()
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
    /// closed registration an admin creates the team, and an admin is
    /// never implicitly a member — the owner row belongs
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
    /// it ended. **The team's forge rows survive too**, and that is a
    /// deferral rather than a cascade nobody wrote — the reasoning and
    /// who owns settling it are on
    /// [`V7`](crate::sqlite::migrations)'s `team_id` paragraph.
    /// Whether the caller *may* delete is [`verb_allowed`]'s question
    /// and the server's to ask — this method enforces state
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
    ///
    /// A duplicate whose link is **marked for purge** is its own
    /// refusal — [`DomainError::MarkedForPurge`], not the plain
    /// "already linked" — because the caller's remedy differs: unmark
    /// restores the link, reclaim frees the digest for a fresh upload
    /// (#95, the grace-visibility boundary). The distinction is made
    /// *here*, inside the write transaction, rather than by a handler
    /// pre-check: a mark landing between a handler's read and this
    /// commit would turn the answer stale, and the write API is the
    /// one place that sees the row under the write lock.
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
            match link_mark_in_tx(tx, team_id, link.digest())? {
                Some(None) => {
                    return Ok(Err(DomainError::Validation(format!(
                        "digest {} is already linked to team {team_id}",
                        link.digest()
                    ))));
                }
                Some(Some(_)) => {
                    return Ok(Err(DomainError::MarkedForPurge {
                        team_id,
                        digest: link.digest().to_string(),
                    }));
                }
                None => {}
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

    /// Marks a team's blob link for purge, appending
    /// `teams.blob_link.purge_marked/1` — the first half of the
    /// trash→purge two-step (#83 §3 lifecycle, #95).
    ///
    /// The mark is `purge_marked_at` on the link row (state, never the
    /// ledger): from this instant the link is hidden from normal reads
    /// ([`Self::blob_link_exists`] / [`Self::blob_links`]) but the row
    /// — and the bytes — survive, restorable via
    /// [`Self::unmark_blob_link`] until
    /// [`Self::reclaim_marked_links`] removes it after the grace
    /// window. Marking a link that is not there, or is already marked,
    /// is refused: either would append an event for a state change
    /// that did not happen.
    pub async fn mark_blob_link_for_purge(
        &self,
        team_id: Uuid,
        digest: &str,
        actor: LedgerActor,
        occurred_at_ms: i64,
    ) -> Result<LedgerEvent, DomainError> {
        let digest = parse_digest(digest)?;
        self.write_tx(move |tx| {
            match link_mark_in_tx(tx, team_id, &digest)? {
                None => {
                    return Ok(Err(DomainError::Validation(format!(
                        "digest {digest} is not linked to team {team_id}; there is nothing to mark"
                    ))));
                }
                Some(Some(marked_at)) => {
                    return Ok(Err(DomainError::Validation(format!(
                        "digest {digest} is already marked for purge (at {marked_at}); \
                         a second mark would append an event with no state change"
                    ))));
                }
                Some(None) => {}
            }
            let subject = match SubjectRef::blob(&digest) {
                Ok(subject) => subject,
                Err(refused) => return Ok(Err(refused)),
            };
            tx.execute(
                "UPDATE team_blob_link SET purge_marked_at = ?3
                 WHERE team_id = ?1 AND digest = ?2",
                params![team_id, digest, occurred_at_ms],
            )?;
            append_event_in_tx(
                tx,
                team_id,
                &actor,
                occurred_at_ms,
                BLOB_LINK_PURGE_MARKED,
                vec![subject],
                serde_json::json!({
                    "digest": digest,
                    "marked_at_ms": occurred_at_ms,
                }),
            )
        })
        .await
    }

    /// Lifts a purge mark during the grace window, appending
    /// `teams.blob_link.purge_unmarked/1` — the link is restored
    /// intact (only the mark column changes; `created_at` and the row
    /// itself were never touched).
    ///
    /// Unmarking a link that is not marked (or not there) is refused,
    /// same reasoning as the mark's. There is no window check here on
    /// purpose: the window bounds *reclaim*, not restoration — a mark
    /// nobody reclaimed yet is restorable however old it is.
    pub async fn unmark_blob_link(
        &self,
        team_id: Uuid,
        digest: &str,
        actor: LedgerActor,
        occurred_at_ms: i64,
    ) -> Result<LedgerEvent, DomainError> {
        let digest = parse_digest(digest)?;
        self.write_tx(move |tx| {
            let was_marked_at = match link_mark_in_tx(tx, team_id, &digest)? {
                None => {
                    return Ok(Err(DomainError::Validation(format!(
                        "digest {digest} is not linked to team {team_id}; there is nothing to \
                         unmark"
                    ))));
                }
                Some(None) => {
                    return Ok(Err(DomainError::Validation(format!(
                        "digest {digest} is not marked for purge in team {team_id}; \
                         an unmark would append an event with no state change"
                    ))));
                }
                Some(Some(marked_at)) => marked_at,
            };
            let subject = match SubjectRef::blob(&digest) {
                Ok(subject) => subject,
                Err(refused) => return Ok(Err(refused)),
            };
            tx.execute(
                "UPDATE team_blob_link SET purge_marked_at = NULL
                 WHERE team_id = ?1 AND digest = ?2",
                params![team_id, digest],
            )?;
            append_event_in_tx(
                tx,
                team_id,
                &actor,
                occurred_at_ms,
                BLOB_LINK_PURGE_UNMARKED,
                vec![subject],
                serde_json::json!({
                    "digest": digest,
                    "was_marked_at_ms": was_marked_at,
                }),
            )
        })
        .await
    }

    /// Removes the team's marked links whose grace window has elapsed,
    /// appending one `teams.blob_link.reclaimed/1` event that carries
    /// every digest removed — the second half of the two-step, and the
    /// only path that removes links for reclaim's sake (#83 §3).
    ///
    /// The window is evaluated per link (`purge_marked_at +
    /// grace_window_ms <= occurred_at_ms`): marks land at different
    /// times, and one reclaim removes exactly the ripe ones while the
    /// still-waiting marks stay marked for a later reclaim. Two
    /// refusals, both before anything is written: nothing is marked at
    /// all, or marks exist but none has waited out its window — the
    /// "reclaim before the window is refused" rule, with the earliest
    /// reclaimable instant named so the caller knows when to return.
    ///
    /// The record survives, the bytes go: the link rows are deleted
    /// (state), the ledger keeps the mark/unmark/reclaim history, and
    /// the bytes are the zero-link sweep's to collect *after* this
    /// transaction commits ([`crate::gc`]).
    pub async fn reclaim_marked_links(
        &self,
        team_id: Uuid,
        grace_window_ms: i64,
        actor: LedgerActor,
        occurred_at_ms: i64,
    ) -> Result<(Vec<String>, LedgerEvent), DomainError> {
        self.write_tx(move |tx| {
            if !team_exists_in_tx(tx, team_id)? {
                return Ok(Err(DomainError::Validation(format!(
                    "team {team_id} does not exist"
                ))));
            }
            let mut stmt = tx.prepare(
                "SELECT digest, purge_marked_at FROM team_blob_link
                 WHERE team_id = ?1 AND purge_marked_at IS NOT NULL
                 ORDER BY digest",
            )?;
            let marked: Vec<(String, i64)> = stmt
                .query_map(params![team_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;
            drop(stmt);
            if marked.is_empty() {
                return Ok(Err(DomainError::Validation(format!(
                    "team {team_id} has no links marked for purge; reclaim removes marked \
                     links only"
                ))));
            }
            let ripe: Vec<String> = marked
                .iter()
                .filter(|(_, marked_at)| {
                    marked_at.saturating_add(grace_window_ms) <= occurred_at_ms
                })
                .map(|(digest, _)| digest.clone())
                .collect();
            if ripe.is_empty() {
                let earliest = marked
                    .iter()
                    .map(|(_, marked_at)| marked_at.saturating_add(grace_window_ms))
                    .min()
                    .expect("marked is non-empty");
                return Ok(Err(DomainError::Validation(format!(
                    "the grace window has not elapsed for any of team {team_id}'s {} marked \
                     link(s); the earliest becomes reclaimable at {earliest} (epoch ms)",
                    marked.len()
                ))));
            }
            let mut subjects = Vec::with_capacity(ripe.len());
            for digest in &ripe {
                match SubjectRef::blob(digest) {
                    Ok(subject) => subjects.push(subject),
                    Err(refused) => return Ok(Err(refused)),
                }
            }
            for digest in &ripe {
                tx.execute(
                    "DELETE FROM team_blob_link WHERE team_id = ?1 AND digest = ?2",
                    params![team_id, digest],
                )?;
            }
            let event = append_event_in_tx(
                tx,
                team_id,
                &actor,
                occurred_at_ms,
                BLOB_LINK_RECLAIMED,
                subjects,
                serde_json::json!({
                    "digests": ripe,
                    "grace_window_ms": grace_window_ms,
                }),
            )?;
            Ok(event.map(|event| (ripe, event)))
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

    /// Publishes a head entry, superseding the live one (if any) in
    /// the same transaction — one current head per instance (#132
    /// phase 3), the invariant the model registry carried before it.
    /// Republishing the same entry is accepted, not an error: it
    /// supersedes and re-inserts, which is a history row saying an
    /// admin published again.
    ///
    /// Like [`Self::record_locator`], no ledger append (see the module
    /// doc): the registry is instance-scope, the ledger's streams are
    /// per-team (#83 §2), and the table's own superseded rows are the
    /// publish history — so this is a plain transaction, not
    /// `write_tx`'s state+event pair.
    pub async fn publish_head_entry(
        &self,
        entry: TagHeadEntry,
        published_at_ms: i64,
    ) -> Result<(), DomainError> {
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "UPDATE head_registry_entry SET superseded_at = ?1
                     WHERE superseded_at IS NULL",
                    params![published_at_ms],
                )?;
                tx.execute(
                    "INSERT INTO head_registry_entry (label, entry, published_at)
                     VALUES (?1, ?2, ?3)",
                    params![entry.label(), entry.raw(), published_at_ms],
                )?;
                tx.commit()
            })
            .await
            .map_err(infra_err)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Reads — promotion outside the closure (the map.rs convention).
    // ------------------------------------------------------------------

    /// The live head entry, or `None` while nothing has been
    /// published. Stored bytes pass back through the domain's envelope
    /// parser on the way out (the [`Role::parse`] convention): a row
    /// this instance would no longer accept surfaces as a validation
    /// error, never as bytes served with the instance's implicit
    /// endorsement.
    pub async fn current_head_entry(&self) -> Result<Option<TagHeadEntry>, DomainError> {
        let raw: Option<String> = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT entry FROM head_registry_entry WHERE superseded_at IS NULL",
                    [],
                    |row| row.get(0),
                )
                .optional()
            })
            .await
            .map_err(infra_err)?;
        raw.as_deref().map(TagHeadEntry::parse).transpose()
    }

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

    /// The teams a user is a member of, oldest team first.
    ///
    /// [`Self::roster`] turned around: that one takes a team and reads
    /// its members, this takes a user and reads their teams. The
    /// `membership` primary key leads with `team_id`, so this
    /// direction is what `idx_membership_user` exists for — it has
    /// been in the schema since v1, ahead of any caller.
    ///
    /// Ordered by the team's creation time because that is the only
    /// order these rows carry a fact for: a membership has no
    /// timestamp of its own, and ordering by id would be ordering by a
    /// UUID. The role goes through [`Role::parse`] on the way out for
    /// the reason the roster's does — a word the domain does not admit
    /// is a validation error rather than a guessed authority.
    pub async fn teams_of_user(&self, user_id: Uuid) -> Result<Vec<TeamMembership>, DomainError> {
        let rows: Vec<(Uuid, String, i64)> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT m.team_id, m.role, t.created_at
                     FROM membership m
                     JOIN team t ON t.id = m.team_id
                     WHERE m.user_id = ?1
                     ORDER BY t.created_at, m.team_id",
                )?;
                let rows = stmt.query_map(params![user_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
                rows.collect()
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(team_id, role, created_at)| {
                Ok(TeamMembership {
                    team_id,
                    role: Role::parse(&role)?,
                    created_at,
                })
            })
            .collect()
    }

    /// Whether `digest` is linked to `team_id` — the visibility
    /// question the blob read surface asks (#83 §3: a digest exists
    /// for a caller iff a link row sits in a team they belong to).
    /// The digest goes through the domain's parser first, so a
    /// malformed probe is a refusal, never a silent `false`.
    ///
    /// A **marked** link answers `false` here: hiding from normal
    /// reads is what the mark means (#95), and this predicate is what
    /// the blob route's indistinguishable `404` stands on — a marked
    /// digest and a never-linked one read identically.
    pub async fn blob_link_exists(&self, team_id: Uuid, digest: &str) -> Result<bool, DomainError> {
        let digest = parse_digest(digest)?;
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM team_blob_link
                     WHERE team_id = ?1 AND digest = ?2 AND purge_marked_at IS NULL)",
                    params![team_id, digest],
                    |row| row.get(0),
                )
            })
            .await
            .map_err(infra_err)
    }

    /// The team's **visible** blob links — marked links are hidden
    /// from this roster for their grace window (#95); the marked set
    /// has its own read, [`Self::marked_blob_links`]. Each digest is
    /// re-validated through [`TeamBlobLink::new`] on the way out.
    pub async fn blob_links(&self, team_id: Uuid) -> Result<Vec<TeamBlobLink>, DomainError> {
        let digests: Vec<String> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT digest FROM team_blob_link
                     WHERE team_id = ?1 AND purge_marked_at IS NULL ORDER BY digest",
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

    /// Which of `digests` this team holds — the have-check's read
    /// (#148 decision 19), asked over a list rather than one at a
    /// time.
    ///
    /// [`Self::blob_link_exists`] answered one digest and this answers
    /// many, with the same predicate underneath: a marked link is not
    /// held, because a caller told "you have this" about a link inside
    /// its grace window would skip a send for bytes a reclaim is about
    /// to take. Digests are validated on the way *in* — a malformed
    /// one is the caller's grammar error and says so, rather than
    /// silently reading as "not held".
    ///
    /// The answer is a set of the ones held, not a bitmap: the caller
    /// asked what it can skip, and a shape that only names those is
    /// one a caller cannot line up against the wrong list.
    pub async fn held_digests(
        &self,
        team_id: Uuid,
        digests: Vec<String>,
    ) -> Result<BTreeSet<String>, DomainError> {
        let asked = digests
            .iter()
            .map(|digest| parse_digest(digest))
            .collect::<Result<BTreeSet<String>, DomainError>>()?;
        if asked.is_empty() {
            return Ok(BTreeSet::new());
        }
        self.isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT digest FROM team_blob_link
                     WHERE team_id = ?1 AND digest = ?2 AND purge_marked_at IS NULL",
                )?;
                let mut held = BTreeSet::new();
                for digest in asked {
                    let hit: Option<String> = stmt
                        .query_row(params![team_id, digest], |row| row.get(0))
                        .optional()?;
                    if let Some(digest) = hit {
                        held.insert(digest);
                    }
                }
                Ok(held)
            })
            .await
            .map_err(infra_err)
    }

    /// The team's marked links with their mark instants — the purge
    /// half of the ledgerless state question ("what would a reclaim
    /// look at"). Deliberately a separate read instead of a flag on
    /// [`Self::blob_links`], so no normal-read call site can forget to
    /// filter.
    pub async fn marked_blob_links(
        &self,
        team_id: Uuid,
    ) -> Result<Vec<(TeamBlobLink, i64)>, DomainError> {
        let rows: Vec<(String, i64)> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT digest, purge_marked_at FROM team_blob_link
                     WHERE team_id = ?1 AND purge_marked_at IS NOT NULL ORDER BY digest",
                )?;
                let rows =
                    stmt.query_map(params![team_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
                rows.collect()
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(digest, marked_at)| Ok((TeamBlobLink::new(team_id, &digest)?, marked_at)))
            .collect()
    }

    /// Whether **any** team links `digest` — marked links included,
    /// because a marked link is restorable and its bytes must survive
    /// (#95). This is the zero-link sweep's question ([`crate::gc`]),
    /// deliberately distinct from [`Self::blob_link_exists`]'s
    /// visibility question: the sweep protects what *could* come back,
    /// the read surface shows what *is* there.
    pub async fn digest_linked_anywhere(&self, digest: &str) -> Result<bool, DomainError> {
        let digest = parse_digest(digest)?;
        self.isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM team_blob_link WHERE digest = ?1)",
                    params![digest],
                    |row| row.get(0),
                )
            })
            .await
            .map_err(infra_err)
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
    ///
    /// The whole stream, which is what makes this the read to stop
    /// reaching for: a ledger only grows, and a team old enough to be
    /// worth reading is a team whose stream does not fit in one
    /// response. [`Self::events_page`] is the same walk with a cursor
    /// and a bound on it.
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

    /// One page of the team's stream: up to `limit` events with a seq
    /// above `after`, seq ascending.
    ///
    /// A keyset cursor rather than an offset, and the key is one the
    /// table already has — `(team_id, seq)` is the primary key, so the
    /// page is a range scan from where the last one stopped and costs
    /// the same whether it is the first page or the ten-thousandth.
    /// `OFFSET` would walk and discard every row before the page, which
    /// is the read getting slower exactly as the stream it reads gets
    /// longer. It is also stable under concurrent appends in a way an
    /// offset is not: appends land after the highest seq, so a page
    /// taken from `after` never shifts under a reader mid-walk.
    ///
    /// `after = None` starts at the beginning. The caller resumes by
    /// passing the last seq it received, so a page that came back
    /// shorter than `limit` is the end of the stream *for now* — a
    /// ledger has no final page, only a position nothing has been
    /// appended past yet.
    pub async fn events_page(
        &self,
        team_id: Uuid,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<LedgerEvent>, DomainError> {
        // Zero is the caller asking for nothing, and it gets nothing
        // without a round trip — `LIMIT 0` would return the same empty
        // result from SQLite, so this is saving the query rather than
        // correcting it.
        //
        // What it is not is a *cursor* answer. An empty page here
        // satisfies `len() == limit` at zero, so a caller deriving
        // "was this page full?" from the count gets `true` off a
        // stream it has read nothing of. That is the reader's problem
        // to avoid, and the HTTP surface avoids it by refusing to pass
        // zero down at all; a reader that does pass zero is asking for
        // no rows and must not also ask what came after them.
        if limit == 0 {
            return Ok(Vec::new());
        }
        let after = after.unwrap_or(0);
        let limit = i64::from(limit);
        let (events, subjects) = self
            .isle
            .call(move |conn| {
                let events = fetch_event_rows(
                    conn,
                    "SELECT seq, event_id, actor, occurred_at, kind, payload
                     FROM ledger_event WHERE team_id = ?1 AND seq > ?2
                     ORDER BY seq LIMIT ?3",
                    params![team_id, after, limit],
                )?;
                // The subjects of exactly the events on this page. The
                // bound is the page's own last seq rather than a second
                // LIMIT: an event carries any number of subjects, so a
                // row count would cut one event's subjects in half.
                let highest = events.last().map(|row| row.seq).unwrap_or(after);
                let subjects = fetch_subject_rows(
                    conn,
                    "SELECT seq, ref_type, ref_value FROM ledger_subject
                     WHERE team_id = ?1 AND seq > ?2 AND seq <= ?3
                     ORDER BY seq, rowid",
                    params![team_id, after, highest],
                )?;
                Ok((events, subjects))
            })
            .await
            .map_err(infra_err)?;
        promote_events(team_id, events, subjects)
    }

    /// One page of the events that reference `subject`:
    /// [`Self::events_for_subject`]'s walk under
    /// [`Self::events_page`]'s cursor.
    ///
    /// The transport exposes this one rather than its unpaged sibling,
    /// and for decision 18's reason applied to a narrower question: a
    /// subject filter bounds the stream by *what* rather than by how
    /// much, and the forge's busiest subjects — a line, the work
    /// against it — are exactly the ones that gain a row per push. A
    /// read whose response size is a function of how much a line has
    /// been worked on is the shape #149 took the cursor out for.
    ///
    /// Same keyset, same meaning: `after` is the last seq the caller
    /// received, a short page is what ends the walk, and the index
    /// walked is `(ref_type, ref_value)` rather than payload JSON
    /// (#83 §2).
    pub async fn events_for_subject_page(
        &self,
        team_id: Uuid,
        subject: &SubjectRef,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<LedgerEvent>, DomainError> {
        // Zero for [`Self::events_page`]'s reason: an empty page here
        // is not a cursor answer, and the caller gets it without a
        // round trip.
        if limit == 0 {
            return Ok(Vec::new());
        }
        let (ref_type, ref_value) = subject_to_ref(subject)?;
        let after = after.unwrap_or(0);
        let limit = i64::from(limit);
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
                     WHERE e.team_id = ?1 AND e.seq > ?4 ORDER BY e.seq LIMIT ?5",
                    params![team_id, ref_type, ref_value, after, limit],
                )?;
                // The subject rows of exactly the page above, so an
                // event never comes back carrying a partial subject
                // list: the same seq window, not the same filter.
                let subjects = fetch_subject_rows(
                    conn,
                    "SELECT seq, ref_type, ref_value FROM ledger_subject
                     WHERE team_id = ?1 AND seq IN
                           (SELECT seq FROM (SELECT DISTINCT seq FROM ledger_subject
                                             WHERE team_id = ?1 AND ref_type = ?2
                                               AND ref_value = ?3)
                             WHERE seq > ?4 ORDER BY seq LIMIT ?5)
                     ORDER BY seq, rowid",
                    params![team_id, ref_type, ref_value, after, limit],
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
    ///
    /// The whole set, which is what makes this the one to stop
    /// reaching for over HTTP — [`Self::events_for_subject_page`] is
    /// the same walk with a cursor, and the transport exposes that.
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

/// The link row's mark state as it reads inside this transaction:
/// `None` = no link row, `Some(None)` = linked and live, `Some(Some(t))`
/// = linked and marked for purge at `t`. The three-way answer is what
/// mark and unmark dispatch their refusals on.
///
/// `pub(crate)` for [`append_event_in_tx`]'s reason: the content verb
/// (#148 decision 5) links a digest from
/// [`sqlite::forge`](crate::sqlite::forge), inside its own
/// transaction, and it dispatches on the same three answers. A second
/// spelling of this read is a second place for "marked counts as
/// held" to be got wrong.
pub(crate) fn link_mark_in_tx(
    tx: &Transaction<'_>,
    team_id: Uuid,
    digest: &str,
) -> Result<Option<Option<i64>>, rusqlite::Error> {
    tx.query_row(
        "SELECT purge_marked_at FROM team_blob_link WHERE team_id = ?1 AND digest = ?2",
        params![team_id, digest],
        |row| row.get(0),
    )
    .optional()
}

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
///
/// `pub(crate)` because the same-tx rule has a second writer now: the
/// hosted forge (#148 decisions 17 and 20) appends through this from
/// [`sqlite::forge`](crate::sqlite::forge), inside the transaction its
/// own write is in. Sharing the function rather than the SQL is what
/// keeps `seq` allocation, the registry check and the subject index
/// one implementation — a forge verb that wrote its own `MAX(seq) + 1`
/// would be a second place for the gapless guarantee to be got right.
pub(crate) fn append_event_in_tx(
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
    // Both registries answer — the substrate's own gestures and the
    // hosted forge's verbs — because both write through here.
    if !is_registered_kind(&kind) {
        return Ok(Err(DomainError::Validation(format!(
            "event kind {kind} is in neither registry; this build does not write it"
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
    async fn a_mark_hides_the_link_and_an_unmark_restores_it_intact() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, _, actor) = team_with_owner(&repo).await;
        let digest = digest_of('a');
        let link = TeamBlobLink::new(team_id, &digest).unwrap();
        repo.add_blob_link(link.clone(), actor.clone(), T0)
            .await
            .unwrap();

        let marked = repo
            .mark_blob_link_for_purge(team_id, &digest, actor.clone(), T0 + 1)
            .await
            .unwrap();
        assert_eq!(marked.kind.as_str(), BLOB_LINK_PURGE_MARKED);
        assert_eq!(marked.payload["digest"], digest);
        assert_eq!(marked.payload["marked_at_ms"], T0 + 1);
        assert_eq!(marked.subjects, vec![SubjectRef::blob(&digest).unwrap()]);

        // Hidden from every normal read…
        assert!(!repo.blob_link_exists(team_id, &digest).await.unwrap());
        assert!(repo.blob_links(team_id).await.unwrap().is_empty());
        // …but not gone: the marked read shows it, and any-link (the
        // sweep's question) still counts it.
        let marked_links = repo.marked_blob_links(team_id).await.unwrap();
        assert_eq!(marked_links, vec![(link.clone(), T0 + 1)]);
        assert!(repo.digest_linked_anywhere(&digest).await.unwrap());

        let unmarked = repo
            .unmark_blob_link(team_id, &digest, actor.clone(), T0 + 2)
            .await
            .unwrap();
        assert_eq!(unmarked.kind.as_str(), BLOB_LINK_PURGE_UNMARKED);
        assert_eq!(unmarked.payload["was_marked_at_ms"], T0 + 1);

        // Restored intact: visible again, same row (created_at was
        // never touched — only the mark column moved).
        assert!(repo.blob_link_exists(team_id, &digest).await.unwrap());
        assert_eq!(repo.blob_links(team_id).await.unwrap(), vec![link]);
        assert!(repo.marked_blob_links(team_id).await.unwrap().is_empty());

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn mark_and_unmark_refuse_when_there_is_no_state_change_to_record() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, _, actor) = team_with_owner(&repo).await;
        let linked = digest_of('b');
        let never = digest_of('c');
        repo.add_blob_link(
            TeamBlobLink::new(team_id, &linked).unwrap(),
            actor.clone(),
            T0,
        )
        .await
        .unwrap();
        let events_before = repo.events(team_id).await.unwrap().len();

        // Marking what is not linked, unmarking what is not marked.
        let refused = repo
            .mark_blob_link_for_purge(team_id, &never, actor.clone(), T0 + 1)
            .await;
        assert!(matches!(refused, Err(DomainError::Validation(_))));
        let refused = repo
            .unmark_blob_link(team_id, &linked, actor.clone(), T0 + 1)
            .await;
        assert!(matches!(refused, Err(DomainError::Validation(_))));

        // A second mark on a marked link.
        repo.mark_blob_link_for_purge(team_id, &linked, actor.clone(), T0 + 2)
            .await
            .unwrap();
        let refused = repo
            .mark_blob_link_for_purge(team_id, &linked, actor.clone(), T0 + 3)
            .await;
        assert!(matches!(refused, Err(DomainError::Validation(_))));

        // Re-linking a marked digest is the purge-aware refusal, not
        // the plain duplicate: the caller's remedy is unmark or
        // reclaim, and the variant is what lets the transport name it
        // (#95). The mark itself is untouched.
        let refused = repo
            .add_blob_link(
                TeamBlobLink::new(team_id, &linked).unwrap(),
                actor.clone(),
                T0 + 3,
            )
            .await;
        assert!(matches!(
            refused,
            Err(DomainError::MarkedForPurge { team_id: t, digest: d })
                if t == team_id && d == linked
        ));
        assert_eq!(repo.marked_blob_links(team_id).await.unwrap().len(), 1);

        // Exactly one event landed — the successful mark; every
        // refusal wrote nothing.
        assert_eq!(repo.events(team_id).await.unwrap().len(), events_before + 1);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn reclaim_refuses_early_removes_ripe_links_and_the_stream_keeps_the_story() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, _, actor) = team_with_owner(&repo).await;
        const GRACE: i64 = 1_000;
        let early = digest_of('d');
        let late = digest_of('e');
        for digest in [&early, &late] {
            repo.add_blob_link(
                TeamBlobLink::new(team_id, digest).unwrap(),
                actor.clone(),
                T0,
            )
            .await
            .unwrap();
        }

        // Nothing marked: reclaim has nothing to do and says so.
        let refused = repo
            .reclaim_marked_links(team_id, GRACE, actor.clone(), T0 + 1)
            .await;
        assert!(matches!(refused, Err(DomainError::Validation(_))));

        // Two marks, a window apart.
        repo.mark_blob_link_for_purge(team_id, &early, actor.clone(), T0 + 10)
            .await
            .unwrap();
        repo.mark_blob_link_for_purge(team_id, &late, actor.clone(), T0 + 500)
            .await
            .unwrap();

        // Before any window elapses: refused, and the refusal names
        // when the earliest mark becomes reclaimable.
        let refused = repo
            .reclaim_marked_links(team_id, GRACE, actor.clone(), T0 + 600)
            .await;
        match refused {
            Err(DomainError::Validation(message)) => assert!(
                message.contains(&(T0 + 10 + GRACE).to_string()),
                "the refusal must name the earliest reclaimable instant: {message}"
            ),
            other => panic!("expected a Validation refusal, got {other:?}"),
        }
        assert_eq!(repo.marked_blob_links(team_id).await.unwrap().len(), 2);

        // Between the two windows: exactly the ripe link goes, the
        // still-waiting mark stays marked.
        let (removed, event) = repo
            .reclaim_marked_links(team_id, GRACE, actor.clone(), T0 + 10 + GRACE)
            .await
            .unwrap();
        assert_eq!(removed, vec![early.clone()]);
        assert_eq!(event.kind.as_str(), BLOB_LINK_RECLAIMED);
        assert_eq!(event.payload["digests"], serde_json::json!([early]));
        assert_eq!(event.payload["grace_window_ms"], GRACE);
        assert_eq!(event.subjects, vec![SubjectRef::blob(&early).unwrap()]);
        let still_marked = repo.marked_blob_links(team_id).await.unwrap();
        assert_eq!(still_marked.len(), 1);
        assert_eq!(still_marked[0].0.digest(), late);

        // The record survives the row: the removed link's whole story
        // reads back off the subjects index.
        assert!(!repo.digest_linked_anywhere(&early).await.unwrap());
        let hits = repo
            .events_for_subject(team_id, &SubjectRef::blob(&early).unwrap())
            .await
            .unwrap();
        let story: Vec<&str> = hits.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            story,
            vec![
                BLOB_COPY_COMPLETED,
                BLOB_LINK_PURGE_MARKED,
                BLOB_LINK_RECLAIMED
            ]
        );

        // The second window elapses; the second reclaim takes the rest.
        let (removed, _) = repo
            .reclaim_marked_links(team_id, GRACE, actor, T0 + 500 + GRACE)
            .await
            .unwrap();
        assert_eq!(removed, vec![late]);
        assert!(repo.marked_blob_links(team_id).await.unwrap().is_empty());

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn an_admin_action_reads_back_as_the_admin() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, _, _) = team_with_owner(&repo).await;

        let admin =
            teams_core::domain::identity::InstanceAdmin::new(Uuid::now_v7(), "admin").unwrap();
        repo.delete_team(team_id, LedgerActor::admin(&admin), T0 + 1)
            .await
            .unwrap();

        let events = repo.events(team_id).await.unwrap();
        let last = events.last().unwrap();
        assert!(
            last.actor.is_admin(),
            "the admin's delete must read back admin-stamped, never disguised (#83 §1)"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_paged_walk_sees_the_whole_stream_once_and_in_order() {
        let (repo, _isle, driver) = repo().await;
        let (team_id, _, actor) = team_with_owner(&repo).await;

        // Enough events that a page smaller than the stream is a real
        // page: the founding event plus one per invite.
        for step in 0..6 {
            repo.add_member(
                membership(team_id, Uuid::now_v7(), Role::Member),
                actor.clone(),
                T0 + 1 + step,
            )
            .await
            .unwrap();
        }
        let whole = repo.events(team_id).await.unwrap();
        assert!(whole.len() > 3, "the fixture must outgrow one page");

        // Walking in pages of three reproduces the unpaged read
        // exactly — same events, same order, nothing seen twice and
        // nothing skipped at a page boundary.
        let mut walked = Vec::new();
        let mut after = None;
        loop {
            let page = repo.events_page(team_id, after, 3).await.unwrap();
            assert!(page.len() <= 3, "a page never exceeds its limit");
            let Some(last) = page.last() else { break };
            after = Some(last.seq.get());
            walked.extend(page);
        }
        assert_eq!(walked, whole);

        // An event's subjects arrive with it rather than being cut at
        // the page boundary: every walked event matches the unpaged
        // one field for field, subjects included, which the equality
        // above already asserts — this pins that they are not empty,
        // so that equality is not two empty lists agreeing.
        assert!(
            walked.iter().any(|event| !event.subjects.is_empty()),
            "the fixture must carry subjects for the page bound to be tested"
        );

        // The cursor is exclusive, and a limit of zero asks for
        // nothing rather than for everything.
        let first = repo.events_page(team_id, None, 1).await.unwrap();
        let after_first = repo
            .events_page(team_id, Some(first[0].seq.get()), 1)
            .await
            .unwrap();
        assert_ne!(first[0].seq, after_first[0].seq);
        assert!(repo.events_page(team_id, None, 0).await.unwrap().is_empty());

        // Past the end is an empty page, not an error: a ledger has no
        // final page, only a position nothing has passed yet.
        let past_end = whole.last().unwrap().seq.get();
        assert!(
            repo.events_page(team_id, Some(past_end), 10)
                .await
                .unwrap()
                .is_empty()
        );

        driver.shutdown().await.unwrap();
    }

    fn head_entry(label: &str, marker: &str) -> TagHeadEntry {
        TagHeadEntry::parse(
            &serde_json::json!({
                "schema": teams_core::domain::head_registry::HEAD_ENTRY_SCHEMA_V1,
                "head": label,
                "model_id": "test-model",
                "dim": 4,
                "preprocess_ver": 1,
                "marker": marker,
            })
            .to_string(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn publishing_a_head_entry_supersedes_and_keeps_history() {
        let (repo, isle, driver) = repo().await;

        // Nothing published yet — the read says so rather than erring.
        assert!(repo.current_head_entry().await.unwrap().is_none());

        repo.publish_head_entry(head_entry("head-v1", "first"), T0)
            .await
            .unwrap();
        let current = repo.current_head_entry().await.unwrap().unwrap();
        assert_eq!(current.label(), "head-v1");
        assert!(current.raw().contains("first"));

        // A second publish supersedes: the read answers the new entry,
        // and the old row survives, stamped — rollback stays
        // answerable from the table's own history.
        repo.publish_head_entry(head_entry("head-v2", "second"), T0 + 1)
            .await
            .unwrap();
        let current = repo.current_head_entry().await.unwrap().unwrap();
        assert_eq!(current.label(), "head-v2");

        let (rows, live): (i64, i64) = isle
            .call(|conn| {
                Ok((
                    conn.query_row("SELECT count(*) FROM head_registry_entry", [], |r| r.get(0))?,
                    conn.query_row(
                        "SELECT count(*) FROM head_registry_entry WHERE superseded_at IS NULL",
                        [],
                        |r| r.get(0),
                    )?,
                ))
            })
            .await
            .unwrap();
        assert_eq!((rows, live), (2, 1));

        // No ledger stream sees any of it — instance scope, module doc.
        assert_eq!(ledger_row_count(&isle).await, 0);

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn the_schema_itself_refuses_a_second_live_entry() {
        let (repo, isle, driver) = repo().await;
        repo.publish_head_entry(head_entry("head-v1", "first"), T0)
            .await
            .unwrap();

        // The repository path never writes a second live row; the
        // unique expression index is the backstop against raw SQL
        // doing it (NULLs being distinct is why the column alone
        // would not hold — the migration doc's point, pinned here).
        let refused = isle
            .call(|conn| {
                Ok(conn
                    .execute(
                        "INSERT INTO head_registry_entry (label, entry, published_at)
                         VALUES ('head-v2', '{}', 1)",
                        [],
                    )
                    .is_err())
            })
            .await
            .unwrap();
        assert!(refused, "a second live row must hit the unique index");

        driver.shutdown().await.unwrap();
    }
}
