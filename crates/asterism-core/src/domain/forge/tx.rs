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
use crate::domain::value::{AssetId, LineEntryId, LineEventId, PersonaId, PursuitId, PursuitTxId};
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

/// What an `in` declares about a line entry it is aimed at (#63
/// decisions 4–5): the entry, and optionally the version of it the
/// caller was looking at when they aimed.
///
/// A pin lives *inside* the target rather than beside it, so "a pin
/// with nothing pinned" is a state this type cannot hold. What it
/// cannot hold on its own is that the pinned event belongs to this
/// entry — a cross-row fact no constraint here or in the schema can
/// see, so the service checks it where both rows are visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxTarget {
    /// The entry this `in` declares it is working on.
    pub entry_id: LineEntryId,
    /// The entry's living event at the moment of aiming — what makes a
    /// later merge able to notice that the entry moved underneath.
    /// `None` aims at the entry without claiming a version.
    pub base_event_id: Option<LineEventId>,
}

/// The closed set of ledger gestures. `In` carries its origin because
/// an entry without one is not a statement — the schema enforces the
/// same pairing with a two-way CHECK.
///
/// The payloads sit on the variants that own them (the `LineVerb`
/// stance), which makes three of V85's four pairing rules
/// unrepresentable rather than merely checked: a pin cannot exist
/// without a target, an out-of-scope claim cannot be made by a
/// `remove`, and nothing but an `update` can supersede. The fourth —
/// that only an `existing` origin targets an entry — needs a check,
/// because origin and aim are separate axes that a rule happens to
/// correlate; [`PursuitTx::new`] holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursuitTxKind {
    /// An asset entered the pursuit.
    In {
        /// Where it came from.
        origin: TxOrigin,
        /// The line entry this gesture declares it is aimed at, if any.
        /// That only an `Existing` origin may carry one is a rule of
        /// [`PursuitTx::new`] rather than of this type — the variant
        /// can hold the pair, and the constructor refuses it.
        target: Option<TxTarget>,
        /// The caller's statement that this reached into a living set
        /// outside the pursuit's own project. Recorded at IN time
        /// because it cannot be re-derived later: the set moves.
        out_of_scope: bool,
    },
    /// The asset's content changed while in the pursuit — the
    /// external-edit round-trip's verb (#63). Reserved: parsed and
    /// derived over, but no service verb writes it yet.
    Update {
        /// The member this revises. `None` while the verb is reserved;
        /// P3 decides whether an unbound update means anything.
        supersedes_asset_id: Option<AssetId>,
    },
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
            Self::In { .. } => "in",
            Self::Update { .. } => "update",
            Self::Remove => "remove",
            Self::Unremove => "unremove",
        }
    }

    /// Storage slug for the `origin` column (`None` on everything but
    /// `in`, matching the CHECK).
    pub fn origin_slug(&self) -> Option<&'static str> {
        match self {
            Self::In { origin, .. } => Some(origin.slug()),
            _ => None,
        }
    }

    /// The entry this gesture is aimed at, if it is aimed at one.
    pub fn target(&self) -> Option<TxTarget> {
        match self {
            Self::In { target, .. } => *target,
            _ => None,
        }
    }

    /// Whether this gesture claimed it reached outside its scope.
    pub fn out_of_scope(&self) -> bool {
        matches!(
            self,
            Self::In {
                out_of_scope: true,
                ..
            }
        )
    }

    /// The member this gesture supersedes, if it supersedes one.
    pub fn supersedes_asset_id(&self) -> Option<AssetId> {
        match self {
            Self::Update {
                supersedes_asset_id,
            } => *supersedes_asset_id,
            _ => None,
        }
    }

    /// Parses the stored columns (closed set, two-way on `origin`: an
    /// `in` without an origin and an origin on anything else are both
    /// corrupt rows).
    ///
    /// A payload the reconstructed variant cannot hold is refused, not
    /// discarded. V85's CHECKs already refuse these rows, so reaching
    /// one here means the storage guarantee has broken somewhere — and
    /// the harmful answer to that is a value that reads back as though
    /// the column had been empty all along. Every column is either
    /// placed on the variant that owns it or reported.
    pub fn from_columns(
        kind: &str,
        origin: Option<&str>,
        target_entry_id: Option<LineEntryId>,
        base_event_id: Option<LineEventId>,
        out_of_scope: bool,
        supersedes_asset_id: Option<AssetId>,
    ) -> Result<Self, DomainError> {
        let stray = |column: &str| {
            DomainError::Validation(format!("pursuit tx kind {kind:?} carries no {column}"))
        };
        let target = match (target_entry_id, base_event_id) {
            (Some(entry_id), base_event_id) => Some(TxTarget {
                entry_id,
                base_event_id,
            }),
            (None, None) => None,
            (None, Some(_)) => {
                return Err(DomainError::Validation(
                    "pursuit tx pins a base event without naming a target entry".into(),
                ));
            }
        };
        if kind != "in" {
            if target.is_some() {
                return Err(stray("target entry"));
            }
            if out_of_scope {
                return Err(stray("out-of-scope claim"));
            }
        }
        if kind != "update" && supersedes_asset_id.is_some() {
            return Err(stray("superseded asset"));
        }
        match (kind, origin) {
            ("in", Some(origin)) => Ok(Self::In {
                origin: TxOrigin::parse(origin)?,
                target,
                out_of_scope,
            }),
            ("in", None) => Err(DomainError::Validation(
                "pursuit tx kind 'in' requires an origin".into(),
            )),
            (_, Some(_)) => Err(DomainError::Validation(format!(
                "pursuit tx kind {kind:?} carries no origin"
            ))),
            ("update", None) => Ok(Self::Update {
                supersedes_asset_id,
            }),
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
    ///
    /// The one pairing rule the type cannot hold is checked here:
    /// aiming at a line entry is something only an `Existing` origin
    /// may do. Keeping what the library already holds is the untouched
    /// default, so a `Generated` or `Imported` asset has no entry to be
    /// working *on* — it is arriving, not revising. The other three of
    /// V85's rules are unrepresentable rather than refused.
    pub fn new(
        pursuit_id: PursuitId,
        persona_id: PersonaId,
        kind: PursuitTxKind,
        asset_id: AssetId,
        note: Option<String>,
        now: DateTime<Utc>,
        attribution: &AttributionContext,
    ) -> Result<Self, DomainError> {
        if let PursuitTxKind::In {
            origin,
            target: Some(_),
            ..
        } = kind
            && origin != TxOrigin::Existing
        {
            return Err(DomainError::Validation(format!(
                "a {} in cannot aim at a line entry: only an existing one is revising something",
                origin.slug()
            )));
        }
        Ok(Self {
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
        })
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
            PursuitTxKind::In { origin, .. } => {
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
            PursuitTxKind::Update { .. } => continue,
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
        PursuitTx::new(pursuit, persona, kind, asset, None, at, &ctx()).unwrap()
    }

    fn plain_in(origin: TxOrigin) -> PursuitTxKind {
        PursuitTxKind::In {
            origin,
            target: None,
            out_of_scope: false,
        }
    }

    #[test]
    fn kind_round_trips_through_columns() {
        let entry = LineEntryId::new();
        let base = LineEventId::new();
        let superseded = AssetId::new();
        for kind in [
            plain_in(TxOrigin::Generated),
            plain_in(TxOrigin::Imported),
            plain_in(TxOrigin::Existing),
            PursuitTxKind::In {
                origin: TxOrigin::Existing,
                target: Some(TxTarget {
                    entry_id: entry,
                    base_event_id: None,
                }),
                out_of_scope: true,
            },
            PursuitTxKind::In {
                origin: TxOrigin::Existing,
                target: Some(TxTarget {
                    entry_id: entry,
                    base_event_id: Some(base),
                }),
                out_of_scope: false,
            },
            PursuitTxKind::Update {
                supersedes_asset_id: None,
            },
            PursuitTxKind::Update {
                supersedes_asset_id: Some(superseded),
            },
            PursuitTxKind::Remove,
            PursuitTxKind::Unremove,
        ] {
            let target = kind.target();
            let back = PursuitTxKind::from_columns(
                kind.kind_slug(),
                kind.origin_slug(),
                target.map(|t| t.entry_id),
                target.and_then(|t| t.base_event_id),
                kind.out_of_scope(),
                kind.supersedes_asset_id(),
            )
            .unwrap();
            assert_eq!(back, kind);
        }
        assert!(PursuitTxKind::from_columns("in", None, None, None, false, None).is_err());
        assert!(
            PursuitTxKind::from_columns("remove", Some("generated"), None, None, false, None)
                .is_err()
        );
        assert!(PursuitTxKind::from_columns("merge", None, None, None, false, None).is_err());
        assert!(
            PursuitTxKind::from_columns("in", Some("existing"), None, Some(base), false, None)
                .is_err(),
            "a pin the read path cannot aim is refused rather than dropped"
        );

        // Every column the reconstructed variant cannot hold is
        // reported rather than dropped — a row that read back as
        // though the column had been empty would be the harmful
        // answer to a broken storage guarantee.
        assert!(
            PursuitTxKind::from_columns("remove", None, Some(entry), None, false, None).is_err(),
            "a remove aiming at an entry"
        );
        assert!(
            PursuitTxKind::from_columns("update", None, Some(entry), Some(base), false, None)
                .is_err(),
            "an update aiming at an entry"
        );
        assert!(
            PursuitTxKind::from_columns("unremove", None, None, None, true, None).is_err(),
            "an unremove claiming it reached outside a scope"
        );
        assert!(
            PursuitTxKind::from_columns(
                "in",
                Some("generated"),
                None,
                None,
                false,
                Some(superseded)
            )
            .is_err(),
            "an in superseding something"
        );
    }

    /// The one pairing rule the type cannot hold. Three of V85's four
    /// are unrepresentable — this is the fourth, and it is refused at
    /// construction rather than left to storage, because a caller that
    /// aims a generated asset at an entry has misunderstood what the
    /// aim means, not merely written a bad row.
    #[test]
    fn only_an_existing_in_may_aim_at_an_entry() {
        let aim = |origin| {
            PursuitTx::new(
                PursuitId::new(),
                PersonaId::new(),
                PursuitTxKind::In {
                    origin,
                    target: Some(TxTarget {
                        entry_id: LineEntryId::new(),
                        base_event_id: None,
                    }),
                    out_of_scope: false,
                },
                AssetId::new(),
                None,
                Utc::now(),
                &ctx(),
            )
        };
        assert!(aim(TxOrigin::Existing).is_ok());
        assert!(aim(TxOrigin::Generated).is_err());
        assert!(aim(TxOrigin::Imported).is_err());
    }

    #[test]
    fn latest_gesture_wins_and_unremove_restores() {
        let pursuit = PursuitId::new();
        let persona = PersonaId::new();
        let asset = AssetId::new();
        let t0 = Utc::now();
        let entered = tx(pursuit, persona, plain_in(TxOrigin::Generated), asset, t0);
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
        let first = tx(pursuit, persona, plain_in(TxOrigin::Existing), asset, t0);
        let again = tx(
            pursuit,
            persona,
            plain_in(TxOrigin::Generated),
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
        let entered = tx(pursuit, persona, plain_in(TxOrigin::Imported), asset, t0);
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
            PursuitTxKind::Update {
                supersedes_asset_id: None,
            },
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
            .map(|a| tx(pursuit, persona, plain_in(TxOrigin::Generated), *a, t0))
            .collect();
        let state = ledger(entries.iter());
        assets.sort();
        assert_eq!(state.keys().copied().collect::<Vec<_>>(), assets);
    }
}
