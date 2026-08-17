//! `PursuitTx` — the pursuit's append-only membership ledger (#22,
//! model on #63): every asset that enters the line of work, every
//! mid-work removal, and every reversal, one row per gesture.
//!
//! The ledger is what makes a cull's "out of what" answerable without
//! being handed in: the candidate set is **what the pursuit
//! accumulated**, derived here and frozen at close — never a
//! caller-supplied snapshot. Mid-work tidying feels like free
//! manipulation on the surface; underneath, every gesture is a ledger
//! entry, which is the difference between a workspace and a record.
//!
//! # Shape
//!
//! - [`PursuitTx`] is one gesture: `in` (with its [`TxOrigin`]),
//!   `remove`, `unremove` — and `update`, the model's reserved verb
//!   for the external-edit round-trip, admitted by the vocabulary but
//!   written by nothing yet.
//! - **Membership is derived on read** by [`ledger`]: latest tx per
//!   asset by `(created_at, id)` wins — `in` / `unremove` mean
//!   present, `remove` means removed, `update` changes nothing. No
//!   row is ever edited.
//! - The asset reference is an id, not a foreign key: the ledger is
//!   history and history outlives the asset (the
//!   `dispatch_job.output_asset_ids` stance). The candidate *set*
//!   survives independently in the snapshot the cull freezes.

use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

use crate::domain::attribution::{AttributionContext, PersistedAttribution};
use crate::domain::value::{AssetId, PersonaId, PursuitId, PursuitTxId};
use crate::error::DomainError;

/// Where an `in` gesture brought its asset from. A fact about the
/// entry, not about the asset: the same asset can enter one pursuit
/// as `existing` and have entered an earlier one as `generated`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxOrigin {
    /// Produced by one of this pursuit's own rounds.
    Generated,
    /// Brought in from outside the library (an import).
    Imported,
    /// Brought in from the existing library — the entry whose cull
    /// verdict is restricted to `reject` (keeping what already exists
    /// is the untouched default, not a statement).
    Existing,
}

impl TxOrigin {
    /// Storage slug.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Imported => "imported",
            Self::Existing => "existing",
        }
    }

    /// Parses a storage slug (closed set — an unknown value is a
    /// corrupt row, not a forward-compat case).
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "generated" => Ok(Self::Generated),
            "imported" => Ok(Self::Imported),
            "existing" => Ok(Self::Existing),
            other => Err(DomainError::Validation(format!(
                "unknown pursuit tx origin: {other:?}"
            ))),
        }
    }
}

/// The closed set of ledger gestures. `In` carries its origin because
/// an entry without one is not a statement — the schema enforces the
/// same pairing with a two-way CHECK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursuitTxKind {
    /// An asset entered the pursuit.
    In(TxOrigin),
    /// The asset's content changed while in the pursuit — the
    /// external-edit round-trip's verb (#63). Reserved: parsed and
    /// derived over, but no service verb writes it yet.
    Update,
    /// Mid-work removal. Reversible ([`Self::Unremove`]) until close;
    /// at close an unreversed removal culls as `reject` unless
    /// salvaged.
    Remove,
    /// Reversal of a removal.
    Unremove,
}

impl PursuitTxKind {
    /// Storage slug for the `kind` column.
    pub fn kind_slug(&self) -> &'static str {
        match self {
            Self::In(_) => "in",
            Self::Update => "update",
            Self::Remove => "remove",
            Self::Unremove => "unremove",
        }
    }

    /// Storage slug for the `origin` column (`None` on everything but
    /// `in`, matching the CHECK).
    pub fn origin_slug(&self) -> Option<&'static str> {
        match self {
            Self::In(origin) => Some(origin.slug()),
            _ => None,
        }
    }

    /// Parses the stored `(kind, origin)` pair (closed set, two-way:
    /// an `in` without an origin and an origin on anything else are
    /// both corrupt rows).
    pub fn from_columns(kind: &str, origin: Option<&str>) -> Result<Self, DomainError> {
        match (kind, origin) {
            ("in", Some(origin)) => Ok(Self::In(TxOrigin::parse(origin)?)),
            ("in", None) => Err(DomainError::Validation(
                "pursuit tx kind 'in' requires an origin".into(),
            )),
            (_, Some(_)) => Err(DomainError::Validation(format!(
                "pursuit tx kind {kind:?} carries no origin"
            ))),
            ("update", None) => Ok(Self::Update),
            ("remove", None) => Ok(Self::Remove),
            ("unremove", None) => Ok(Self::Unremove),
            (other, None) => Err(DomainError::Validation(format!(
                "unknown pursuit tx kind: {other:?}"
            ))),
        }
    }
}

/// One recorded membership gesture.
#[derive(Debug, Clone, PartialEq)]
pub struct PursuitTx {
    /// Surrogate id (UUID v7) — the tie-break in [`ledger`].
    pub id: PursuitTxId,
    /// Pursuit the gesture belongs to.
    pub pursuit_id: PursuitId,
    /// Redundant persona copy for cheap persona-scoped queries and the
    /// purge path (the `pursuit_event.persona_id` precedent).
    pub persona_id: PersonaId,
    /// Which gesture.
    pub kind: PursuitTxKind,
    /// The asset the gesture is about — an id, never a foreign key
    /// (see the module doc).
    pub asset_id: AssetId,
    /// One short free-text slot.
    pub note: Option<String>,
    /// Creation time — the primary derivation sort key.
    pub created_at: DateTime<Utc>,
    operator_ai: Option<crate::domain::attribution::OperatorRef>,
    author: Option<crate::domain::attribution::Author>,
    attributed_via: Option<crate::domain::attribution::AttributionChannel>,
}

impl PursuitTx {
    /// Builds a fresh gesture. Whether the gesture is *legal* against
    /// the current ledger (an `in` of a present member, a `remove` of
    /// an absent one) is checked by the service against [`ledger`] —
    /// only that layer sees the other rows.
    pub fn new(
        pursuit_id: PursuitId,
        persona_id: PersonaId,
        kind: PursuitTxKind,
        asset_id: AssetId,
        note: Option<String>,
        now: DateTime<Utc>,
        attribution: &AttributionContext,
    ) -> Self {
        Self {
            id: PursuitTxId::new(),
            pursuit_id,
            persona_id,
            kind,
            asset_id,
            note: note.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
            created_at: now,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        }
    }

    /// Read-path twin of [`new`](Self::new).
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: PursuitTxId,
        pursuit_id: PursuitId,
        persona_id: PersonaId,
        kind: PursuitTxKind,
        asset_id: AssetId,
        note: Option<String>,
        created_at: DateTime<Utc>,
        attribution: PersistedAttribution,
    ) -> Self {
        Self {
            id,
            pursuit_id,
            persona_id,
            kind,
            asset_id,
            note,
            created_at,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        }
    }

    /// Subject that made this gesture (`None` = unrecorded).
    pub fn author(&self) -> Option<&crate::domain::attribution::Author> {
        self.author.as_ref()
    }

    /// Agent that made this gesture (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&crate::domain::attribution::OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel the pair above arrived through (`None` = unrecorded).
    pub fn attributed_via(&self) -> Option<crate::domain::attribution::AttributionChannel> {
        self.attributed_via
    }

    /// Hands the triple back out whole (see
    /// [`Pursuit::persisted_attribution`](super::pursuit::Pursuit::persisted_attribution)).
    pub fn persisted_attribution(&self) -> PersistedAttribution {
        PersistedAttribution::recorded(
            self.author.clone(),
            self.operator_ai.clone(),
            self.attributed_via,
        )
    }
}

/// One asset's derived position in a pursuit's ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberState {
    /// The origin of the asset's first `in` — how it got here. Later
    /// re-entries do not rewrite it: the first entry is the one that
    /// answers "where did this candidate come from".
    pub origin: TxOrigin,
    /// Whether the latest membership-changing gesture was a `remove`.
    pub removed: bool,
}

/// The derived state of a pursuit's ledger: every asset that ever
/// entered, with its current position. Ordered by asset id (a
/// `BTreeMap`) so the candidate set falls out canonical — ascending,
/// deduplicated — without a second pass, matching the close freeze's
/// calling convention.
pub type Ledger = BTreeMap<AssetId, MemberState>;

/// Derives the ledger state from a pursuit's gestures: latest tx per
/// asset by `(created_at, id)` wins. The input does not need to be
/// sorted; the fold keeps the winner per asset itself. An `update` or
/// a dangling `remove` / `unremove` on an asset that never entered is
/// tolerated on read (the write path refuses to create them) — it
/// yields no membership.
pub fn ledger<'a, I>(txs: I) -> Ledger
where
    I: IntoIterator<Item = &'a PursuitTx>,
{
    // (created_at, id) of the winning membership gesture so far, plus
    // the first `in`'s (created_at, id, origin).
    let mut latest: BTreeMap<AssetId, (DateTime<Utc>, PursuitTxId, bool)> = BTreeMap::new();
    let mut first_in: BTreeMap<AssetId, (DateTime<Utc>, PursuitTxId, TxOrigin)> = BTreeMap::new();
    for tx in txs {
        let removed = match tx.kind {
            PursuitTxKind::In(origin) => {
                let key = (tx.created_at, tx.id, origin);
                first_in
                    .entry(tx.asset_id)
                    .and_modify(|held| {
                        if (key.0, key.1) < (held.0, held.1) {
                            *held = key;
                        }
                    })
                    .or_insert(key);
                false
            }
            PursuitTxKind::Unremove => false,
            PursuitTxKind::Remove => true,
            PursuitTxKind::Update => continue,
        };
        let key = (tx.created_at, tx.id, removed);
        latest
            .entry(tx.asset_id)
            .and_modify(|held| {
                if (key.0, key.1) > (held.0, held.1) {
                    *held = key;
                }
            })
            .or_insert(key);
    }
    latest
        .into_iter()
        .filter_map(|(asset_id, (_, _, removed))| {
            // A membership state without any `in` means the ledger
            // holds only dangling gestures for this asset — no entry,
            // no candidacy.
            first_in.get(&asset_id).map(|(_, _, origin)| {
                (
                    asset_id,
                    MemberState {
                        origin: *origin,
                        removed,
                    },
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::attribution::AttributionContext;
    use chrono::Duration;

    fn ctx() -> AttributionContext {
        AttributionContext::owner_surface()
    }

    fn tx(
        pursuit: PursuitId,
        persona: PersonaId,
        kind: PursuitTxKind,
        asset: AssetId,
        at: DateTime<Utc>,
    ) -> PursuitTx {
        PursuitTx::new(pursuit, persona, kind, asset, None, at, &ctx())
    }

    #[test]
    fn kind_round_trips_through_columns() {
        for kind in [
            PursuitTxKind::In(TxOrigin::Generated),
            PursuitTxKind::In(TxOrigin::Imported),
            PursuitTxKind::In(TxOrigin::Existing),
            PursuitTxKind::Update,
            PursuitTxKind::Remove,
            PursuitTxKind::Unremove,
        ] {
            let back = PursuitTxKind::from_columns(kind.kind_slug(), kind.origin_slug()).unwrap();
            assert_eq!(back, kind);
        }
        assert!(PursuitTxKind::from_columns("in", None).is_err());
        assert!(PursuitTxKind::from_columns("remove", Some("generated")).is_err());
        assert!(PursuitTxKind::from_columns("merge", None).is_err());
    }

    #[test]
    fn latest_gesture_wins_and_unremove_restores() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let asset = AssetId::new();
        let t0 = Utc::now();
        let entered = tx(
            pursuit,
            persona,
            PursuitTxKind::In(TxOrigin::Generated),
            asset,
            t0,
        );
        let removed = tx(
            pursuit,
            persona,
            PursuitTxKind::Remove,
            asset,
            t0 + Duration::seconds(1),
        );
        let state = ledger([&entered, &removed]);
        assert!(state[&asset].removed);
        let restored = tx(
            pursuit,
            persona,
            PursuitTxKind::Unremove,
            asset,
            t0 + Duration::seconds(2),
        );
        let state = ledger([&entered, &removed, &restored]);
        assert!(!state[&asset].removed);
        assert_eq!(state[&asset].origin, TxOrigin::Generated);
    }

    #[test]
    fn the_first_entry_names_the_origin() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let asset = AssetId::new();
        let t0 = Utc::now();
        let first = tx(
            pursuit,
            persona,
            PursuitTxKind::In(TxOrigin::Existing),
            asset,
            t0,
        );
        let again = tx(
            pursuit,
            persona,
            PursuitTxKind::In(TxOrigin::Generated),
            asset,
            t0 + Duration::seconds(1),
        );
        let state = ledger([&first, &again]);
        assert_eq!(state[&asset].origin, TxOrigin::Existing);
    }

    #[test]
    fn an_update_changes_no_membership() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let asset = AssetId::new();
        let t0 = Utc::now();
        let entered = tx(
            pursuit,
            persona,
            PursuitTxKind::In(TxOrigin::Imported),
            asset,
            t0,
        );
        let removed = tx(
            pursuit,
            persona,
            PursuitTxKind::Remove,
            asset,
            t0 + Duration::seconds(1),
        );
        let edited = tx(
            pursuit,
            persona,
            PursuitTxKind::Update,
            asset,
            t0 + Duration::seconds(2),
        );
        let state = ledger([&entered, &removed, &edited]);
        assert!(state[&asset].removed, "update does not unremove");
    }

    #[test]
    fn dangling_gestures_yield_no_candidacy() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let asset = AssetId::new();
        let orphan = tx(pursuit, persona, PursuitTxKind::Remove, asset, Utc::now());
        assert!(ledger([&orphan]).is_empty());
    }

    #[test]
    fn the_candidate_set_falls_out_sorted_and_deduplicated() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let t0 = Utc::now();
        let mut assets = vec![AssetId::new(), AssetId::new(), AssetId::new()];
        let entries: Vec<_> = assets
            .iter()
            .rev()
            .map(|a| {
                tx(
                    pursuit,
                    persona,
                    PursuitTxKind::In(TxOrigin::Generated),
                    *a,
                    t0,
                )
            })
            .collect();
        let state = ledger(entries.iter());
        assets.sort();
        assert_eq!(state.keys().copied().collect::<Vec<_>>(), assets);
    }
}
