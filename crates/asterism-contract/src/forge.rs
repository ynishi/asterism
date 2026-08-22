//! The forge's wire shapes — a line, what is on it, and what a caller
//! asks it to do.
//!
//! A topic module rather than rows in [`command`](crate::command) and
//! [`dto`](crate::dto), for the reason [`query_group`](crate::query_group)
//! is one: the forge's vocabulary answers to a model of its own, and a
//! reader of `LineDto` needs the four or five types beside it more than
//! it needs the asset DTO two hundred lines up.
//!
//! # The model these are a projection of
//!
//! A line is a repository with one canonical history: a genesis and a
//! chain of change points, each carrying a table keyed by entry and
//! axis over three axes — existence, content, name. Nothing here holds
//! what that history answers. [`ForgeLineDto`] carries the line's own
//! fields and the id of its head; what is *on* the line is
//! [`ForgeEntryStateDto`], folded on read, and the chain that produced
//! it is [`ForgeLineHistoryDto`].
//!
//! Two reads exist because both are real questions, and a screen wants
//! the fold. The history grows with the line and is for something
//! showing how a line got where it is.

use serde::{Deserialize, Serialize};

use schema_bridge::SchemaBridge;

/// A line, without what is on it.
///
/// `head_id` is the change point the chain currently ends at. It is
/// derived rather than stored — no column holds it — and it is here
/// because a caller that wants to name where it read from has nothing
/// else to name.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeLineDto {
    /// Line id (UUID hyphenated).
    pub id: String,
    /// What the line is called. One per instance today, named `ROOT`.
    pub name: String,
    /// The rule this line answers a collision with. A slug rather than
    /// a UUID — the forge does not know the set of rules, and a line
    /// stores which one it points at rather than the rule itself.
    pub strategy_id: String,
    /// `"open"` or `"archived"`. An archived line takes no landing and
    /// is the only one a drop can reach.
    pub standing: String,
    /// The change point the chain ends at (UUID hyphenated).
    pub head_id: String,
    /// When the line was opened (unix epoch ms).
    pub created_at_ms: i64,
    /// When the line's own description last moved: a rename, a
    /// strategy change, an archive or a reopen. Not a landing — that
    /// moves the history, which is a different question (unix epoch
    /// ms).
    pub updated_at_ms: i64,
}

/// Where one entry stands on a line, folded from the whole chain.
///
/// The three axes derive independently, so an entry that is off the
/// line still carries the last name and content anything said about
/// it. That is not a leftover: a name off the line is readable and
/// merely available again.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeEntryStateDto {
    /// Entry id (UUID hyphenated).
    pub entry_id: String,
    /// Whether the latest existence axis leaves it on the line.
    pub alive: bool,
    /// The latest name stated, if any table stated one.
    pub name: Option<String>,
    /// The latest content stated, if any table stated one — an asset
    /// id, which is the one reference the forge holds into the layer
    /// below.
    pub content_asset_id: Option<String>,
}

/// One node of a line's history.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeChangePointDto {
    /// Change point id (UUID hyphenated).
    pub id: String,
    /// The node this one sits on — the genesis, or the change point
    /// before it (UUID hyphenated).
    pub parent_id: String,
    /// The work this landing came out of (UUID hyphenated).
    pub from_pursuit_id: String,
    /// The node of that work which ended it (UUID hyphenated).
    pub by_node_id: String,
    /// When it landed (unix epoch ms).
    pub at_ms: i64,
    /// `"user"` or `"system"` — a rule's landing is the system's.
    pub actor_kind: String,
    /// Who landed it (UUID hyphenated).
    pub actor_id: String,
    /// What this landing said, one row per entry and axis.
    pub table: Vec<ForgeChangeRowDto>,
}

/// What one landing said about one entry.
///
/// Three axes, each stated or left alone — this is not a verb. A row
/// that moves only the name leaves `existence` and `content_asset_id`
/// absent, and a reader folding the chain keeps whatever the last
/// table that spoke about that axis said. A row that says nothing at
/// all cannot exist: the model refuses to build one.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeChangeRowDto {
    /// Entry id (UUID hyphenated).
    pub entry_id: String,
    /// `"present"` or `"absent"` when this landing moved the existence
    /// axis, absent when it left it alone.
    pub existence: Option<String>,
    /// What the entry holds from here on (an asset id), when this
    /// landing stated one.
    pub content_asset_id: Option<String>,
    /// What it answers to from here on, when this landing stated one.
    pub name: Option<String>,
}

/// A line's whole history: where it began and every landing since.
///
/// The chain is the order. Nothing here carries a sequence number
/// beside it, and `changes` arrives walked from the genesis.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeLineHistoryDto {
    /// The line itself.
    pub line: ForgeLineDto,
    /// The node the line began at (UUID hyphenated).
    pub genesis_id: String,
    /// When it began (unix epoch ms).
    pub genesis_at_ms: i64,
    /// Every landing, in the chain's order.
    pub changes: Vec<ForgeChangePointDto>,
}

/// A rule a line can be pointed at.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeStrategyDto {
    /// The slug a line points at.
    pub id: String,
    /// What it is called.
    pub name: String,
    /// What it does with a divergence, in one sentence.
    pub summary: String,
}

/// What dropping a line released.
///
/// **These asset ids are the point of the response.** The forge held
/// them while the line existed, and nothing holds them now — a caller
/// that ignores this leaks bytes that no line, no work log and no
/// foreign key is keeping alive any more.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeDiscardedDto {
    /// The line that went (UUID hyphenated).
    pub line_id: String,
    /// The assets the drop released (UUID hyphenated).
    pub released_asset_ids: Vec<String>,
}

/// Opens a line.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct OpenForgeLineCommand {
    /// What to call it.
    pub name: String,
    /// The rule it answers a collision with, by slug.
    pub strategy_id: String,
    /// See [`AddAssetCommand::author_kind`](crate::command::AddAssetCommand::author_kind).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_kind: Option<String>,
    /// See [`AddAssetCommand::author_subject`](crate::command::AddAssetCommand::author_subject).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_subject: Option<String>,
    /// See [`AddAssetCommand::operator_ai`](crate::command::AddAssetCommand::operator_ai).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Renames a line. The name is the line's own description, so this is
/// not a landing and puts nothing on the chain.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RenameForgeLineCommand {
    /// Target line id (UUID hyphenated). Taken from the path over
    /// HTTP; carried here so the same command serves every surface.
    #[serde(default)]
    pub line_id: String,
    /// What to call it now.
    pub name: String,
    /// See [`OpenForgeLineCommand::author_kind`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_kind: Option<String>,
    /// See [`OpenForgeLineCommand::author_subject`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_subject: Option<String>,
    /// See [`OpenForgeLineCommand::operator_ai`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Points a line at a different rule.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SetForgeLineStrategyCommand {
    /// Target line id (UUID hyphenated). Taken from the path over HTTP.
    #[serde(default)]
    pub line_id: String,
    /// The rule to answer collisions with from now on, by slug. A rule
    /// this deployment does not carry is refused.
    pub strategy_id: String,
    /// See [`OpenForgeLineCommand::author_kind`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_kind: Option<String>,
    /// See [`OpenForgeLineCommand::author_subject`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_subject: Option<String>,
    /// See [`OpenForgeLineCommand::operator_ai`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}

/// Archives a line, reopens one, or drops one — the three verbs whose
/// whole input is which line and who is asking.
///
/// One command rather than three identical ones. They differ by the
/// route that carries them, which is where the difference belongs:
/// `archive` and `reopen` move a field, and `discard` is the only
/// thing in the forge that deletes.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeLineActCommand {
    /// Target line id (UUID hyphenated). Taken from the path over HTTP.
    #[serde(default)]
    pub line_id: String,
    /// See [`OpenForgeLineCommand::author_kind`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_kind: Option<String>,
    /// See [`OpenForgeLineCommand::author_subject`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_subject: Option<String>,
    /// See [`OpenForgeLineCommand::operator_ai`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_ai: Option<String>,
}
