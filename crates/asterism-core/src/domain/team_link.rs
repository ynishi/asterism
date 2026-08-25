//! The relation between a local Asset and what a team made of it
//! (#148 decisions 6, 8 and 9).
//!
//! ## It lives here and nowhere else
//!
//! A promotion hands an Asset over and the team converts it into
//! something of its own. Neither side holds a reference to the other's
//! object: the server holds no reference to a local Asset, in either
//! direction, and this module is the whole of what the member's
//! machine keeps about the correspondence.
//!
//! Its key is `(team_id, line_id, entry_id)` — all three fixed by the
//! client rather than learned back from the server, which is the shape
//! offline-first sync converges on. Within one member and one team the
//! relation is 1:1 by construction; across teams one Asset has as many
//! rows as teams; across members one team entry has a row on each
//! machine that promoted it. There is no global 1:1 and nothing needs
//! one — each row is a weak reference to its own team, and reading the
//! row for the team you are looking at is the whole of it.
//!
//! ## Advisory, and attended
//!
//! Either end can vanish independently and neither may break the
//! other (#148 decision 9). So there is no foreign key under any of
//! the four ids, and the pair of verbs that make "advisory" different
//! from "unattended" are
//! [`AssetLinkRepository::dangling_locally`](crate::domain::repository::AssetLinkRepository::dangling_locally)
//! and
//! [`AssetLinkRepository::reap`](crate::domain::repository::AssetLinkRepository::reap).
//! Why no key rather than one of the two SQLite offers is argued from
//! the storage side, in the V104 migration.
//!
//! ## A clone writes no row
//!
//! Only a promotion does. A cloned Asset is a detached copy and says
//! where it came from through `source_kind` / `source_locator` the way
//! every other import does (#148 decision 10) — which is also why a
//! row here means "I put this there" and never merely "I have seen
//! this".

use uuid::Uuid;

use crate::domain::value::AssetId;
use crate::error::DomainError;

/// An id another plane minted, held here as an opaque handle.
///
/// **The type is the enforcement.** #148 decision 6 lets a team-scoped
/// id ride the wire as a handle for talking to that team, and forbids
/// exactly one thing: a client reading one as a local `AssetId`. There
/// is no `From`, no `TryFrom` and no accessor that yields an
/// [`AssetId`] here, in either direction, so the forbidden read is not
/// something a caller has to remember not to do.
///
/// What it *is* good for is being handed back to the team it came
/// from. [`Self::as_uuid`] exists for that — writing the handle to
/// storage and spelling it into a URL — and for nothing else.
///
/// Which plane's namespace a value belongs to is not carried in the
/// type: two handles from two teams are both `TeamScopedId`, and what
/// distinguishes them is the [`AssetLinkKey::team_id`] beside them.
/// That is the same weak-reference discipline decision 8 describes,
/// and widening this type to carry its team would imply a registry of
/// teams that the local plane deliberately does not keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TeamScopedId(Uuid);

impl TeamScopedId {
    /// Takes a handle a team stated.
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    /// Mints one — what a client does for the ids decision 8 says it
    /// fixes rather than learns: a line it is about to open, an entry
    /// it is about to name.
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Reads a handle out of the spelling a team used on the wire.
    pub fn parse(raw: &str, what: &'static str) -> Result<Self, DomainError> {
        Uuid::parse_str(raw)
            .map(Self)
            .map_err(|_| DomainError::Validation(format!("{what} {raw:?} is not a UUID")))
    }

    /// The handle's value, for handing back to the team that minted
    /// it or for storing beside the Asset it corresponds to.
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for TeamScopedId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TeamScopedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// What identifies one promotion: the team, the line, and the entry on
/// it (#148 decision 8).
///
/// The Asset is deliberately not part of the key. Two promotions of
/// the same Asset onto two lines are two rows, and a promotion is
/// identified by where it landed rather than by what it was — which is
/// also what makes the key usable as the correlation key the client
/// fixes before the server has seen anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetLinkKey {
    /// The team the line is hosted by.
    pub team_id: TeamScopedId,
    /// The line the entry is on.
    pub line_id: TeamScopedId,
    /// The entry the promotion named.
    pub entry_id: TeamScopedId,
}

/// One promotion, recorded at home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetLink {
    /// Where it landed.
    pub key: AssetLinkKey,
    /// What was promoted. Not a foreign key: the Asset may be deleted
    /// out from under this row, and finding that is
    /// [`dangling_locally`]'s job rather than the database's.
    ///
    /// [`dangling_locally`]: crate::domain::repository::AssetLinkRepository::dangling_locally
    pub local_asset_id: AssetId,
    /// When the promotion happened, epoch ms.
    pub pushed_at_ms: i64,
}

impl AssetLink {
    /// Records a promotion.
    pub const fn new(key: AssetLinkKey, local_asset_id: AssetId, pushed_at_ms: i64) -> Self {
        Self {
            key,
            local_asset_id,
            pushed_at_ms,
        }
    }
}
