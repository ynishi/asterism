//! The forge's wire shapes — a line, what is on it, and what a caller
//! asks it to do.
//!
//! A topic module rather than rows in [`command`](crate::command) and
//! [`dto`](crate::dto): the forge's vocabulary answers to a model of
//! its own, and a reader of [`ForgeLineDto`] needs the types beside it
//! more than it needs the asset DTO two hundred lines up.
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
    /// What the line is called. An instance has the lines somebody
    /// made on purpose; where there is only ever one, it is `ROOT`.
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
/// table that spoke about that axis said. Two shapes cannot exist,
/// because the model refuses to build them: a row that says nothing at
/// all, and a row that takes an entry off the line while also naming
/// or filling it.
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

// -------------------------------------------------------------------
// Work against a line — a pursuit, its rounds, and what it collides
// with.
// -------------------------------------------------------------------

/// A piece of work against a line, whole: how it opened, every round
/// it wrote, and how it ended if it has.
///
/// The log is the order. Nothing carries a sequence number, and
/// `rounds` arrives in the order the chain holds them — each round
/// naming the node before it in `parent_id`.
///
/// There is no separate "summary" shape. A pursuit is small (an
/// opening, a handful of rounds, at most one close) and a screen that
/// lists work wants the intent and the outcome, which are here. The
/// line's history is the read that needed splitting; this one does
/// not.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgePursuitDto {
    /// Pursuit id (UUID hyphenated).
    pub id: String,
    /// The line this work is against (UUID hyphenated).
    pub line_id: String,
    /// The work this was opened from, when it was opened from another
    /// (UUID hyphenated).
    pub parent_id: Option<String>,
    /// The change point this work was cut from (UUID hyphenated).
    pub base_id: String,
    /// The node the log currently ends at (UUID hyphenated). The
    /// opening when nothing has been written yet.
    pub head_id: String,
    /// A short name for the work, when it was given one.
    pub title: Option<String>,
    /// Anything else said about why the work was opened.
    pub note: Option<String>,
    /// When it was opened (unix epoch ms).
    pub opened_at_ms: i64,
    /// `"user"` or `"system"`.
    pub opened_by_kind: String,
    /// Who opened it (UUID hyphenated).
    pub opened_by_id: String,
    /// Every round, oldest first.
    pub rounds: Vec<ForgeRoundDto>,
    /// How it ended, absent while it is still open.
    pub close: Option<ForgeCloseDto>,
}

/// One round of work — what it asks the line to say, and who asked.
///
/// A round is a request rather than a landing. Nothing here is on the
/// line: what the line says is [`ForgeEntryStateDto`], and what put it
/// there is [`ForgeChangePointDto`].
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeRoundDto {
    /// Round id (UUID hyphenated).
    pub id: String,
    /// The node before it in the log (UUID hyphenated).
    pub parent_id: String,
    /// When it was written (unix epoch ms).
    pub at_ms: i64,
    /// `"user"` or `"system"` — a round a rule wrote is the system's.
    pub actor_kind: String,
    /// Who wrote it (UUID hyphenated).
    pub actor_id: String,
    /// Anything said about the round.
    pub note: Option<String>,
    /// What it asks for, one per operation.
    pub ops: Vec<ForgeOpDto>,
}

/// One operation of a round.
///
/// A verb and its entry, unlike [`ForgeChangeRowDto`], which is the
/// same information after the model has folded it into what a landing
/// *said*. Both exist because both are real: a caller writes verbs,
/// and a line records statements.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeOpDto {
    /// The entry this operates on (UUID hyphenated).
    ///
    /// **Required, including for `"add"`.** The model can mint one,
    /// and this does not use that: a round that forks an entry and
    /// then fills the fork has to name it twice, so the id must exist
    /// before the round is sent. It also means a caller knows what it
    /// created without reading the response back.
    pub entry_id: String,
    /// `"add"`, `"replace"`, `"rename"` or `"remove"`.
    pub kind: String,
    /// The content it puts there, for `"add"` and `"replace"` (an
    /// asset id, UUID hyphenated).
    pub content_asset_id: Option<String>,
    /// The name it gives, for `"add"` and `"rename"`.
    pub name: Option<String>,
}

/// How a piece of work ended.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeCloseDto {
    /// Close id (UUID hyphenated).
    pub id: String,
    /// The node before it in the log (UUID hyphenated).
    pub parent_id: String,
    /// `"satisfied"` or `"abandoned"`.
    pub outcome: String,
    /// Anything said about the ending.
    pub note: Option<String>,
    /// When it ended (unix epoch ms).
    pub at_ms: i64,
    /// `"user"` or `"system"`.
    pub actor_kind: String,
    /// Who ended it (UUID hyphenated).
    pub actor_id: String,
}

/// One axis of one entry that this work asks to move and the line has
/// already moved.
///
/// Derived on every read from the two logs, so it cannot go stale and
/// there is no flag for anybody to clear. What clears a collision is
/// the work asking for something else.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeCollisionDto {
    /// The entry both moved (UUID hyphenated).
    pub entry_id: String,
    /// `"existence"`, `"content"` or `"name"` — the axis both moved.
    pub axis: String,
    /// The change point that moved it, which this work has not seen
    /// (UUID hyphenated).
    pub moved_in_id: String,
}

/// What `resolve` did.
///
/// **Both answers are ordinary, and both are 200.** A rule that leaves
/// a collision to a person writes nothing, which is an outcome rather
/// than a failure — the collision stays where somebody can see it. So
/// `round` absent is the rule declining, not an error, and a caller
/// distinguishes the two by whether it is there.
///
/// `collisions` is what is left either way, which is the question a
/// screen asks next in both cases: after a resolution, whether any
/// remain; after a decline, what the person now has to settle.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeResolvedDto {
    /// The round the rule wrote, absent when it wrote nothing.
    pub round: Option<ForgeRoundDto>,
    /// What this work still collides with, after whatever was written.
    pub collisions: Vec<ForgeCollisionDto>,
}

/// Opens work against a line.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct OpenForgePursuitCommand {
    /// The line to work against (UUID hyphenated).
    pub line_id: String,
    /// The work this is opened from, when it is opened from another
    /// (UUID hyphenated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// A short name for the work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Anything else worth saying about why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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

/// Writes a round.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PushForgeRoundCommand {
    /// Target pursuit id (UUID hyphenated). Taken from the path over
    /// HTTP.
    #[serde(default)]
    pub pursuit_id: String,
    /// What the round asks for. At least one — a round that says
    /// nothing is refused.
    pub ops: Vec<ForgeOpDto>,
    /// Anything worth saying about the round.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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

/// Ends a piece of work.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct CloseForgePursuitCommand {
    /// Target pursuit id (UUID hyphenated). Taken from the path over
    /// HTTP.
    #[serde(default)]
    pub pursuit_id: String,
    /// `"satisfied"` or `"abandoned"`. Satisfied puts what the work
    /// says on the line; abandoned puts nothing there and leaves
    /// everything the work wrote readable.
    pub outcome: String,
    /// Anything worth saying about the ending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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

/// Asks the line's rule to answer whatever this work collides with.
///
/// One command rather than a second empty one beside `close`: resolve
/// takes nothing but who is asking, and the pursuit comes from the
/// path.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgePursuitActCommand {
    /// Target pursuit id (UUID hyphenated). Taken from the path over
    /// HTTP.
    #[serde(default)]
    pub pursuit_id: String,
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

// -------------------------------------------------------------------
// What was said about work — a conversation and what is in it.
// -------------------------------------------------------------------

/// A conversation, whole.
///
/// **Every message and every correction.** Not the current text alone:
/// a correction the reader does not see is a sentence still attributed
/// to somebody who withdrew it, and the model keeps both for that
/// reason. Shaping this for the convenience of a screen that only
/// renders the latest would misreport what people said.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeThreadDto {
    /// Thread id (UUID hyphenated).
    pub id: String,
    /// What the conversation hangs off.
    pub anchor: ForgeAnchorDto,
    /// A name for it, when it was given one.
    pub title: Option<String>,
    /// Everything said, oldest first.
    pub messages: Vec<ForgeMessageDto>,
}

/// What a conversation is about.
///
/// One shape for four anchors, with the fields each needs and the rest
/// absent. `kind` says which, and which ids are present follows from
/// it: `"pursuit"` fills `pursuit_id`, `"round"` fills `node_id`,
/// `"entry"` fills `node_id` and `entry_id`, `"change"` fills
/// `change_point_id`.
///
/// A caller does not build one of these. An anchor is resolved from the
/// path — the service reads the pursuit or the line and the model makes
/// the anchor — so this is a read shape only, and a wrong combination
/// is not something a request can express.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeAnchorDto {
    /// `"pursuit"`, `"round"`, `"entry"` or `"change"`.
    pub kind: String,
    /// The work, for `"pursuit"`.
    pub pursuit_id: Option<String>,
    /// The round, for `"round"` and `"entry"` (UUID hyphenated).
    pub node_id: Option<String>,
    /// The entry that round touched, for `"entry"` (UUID hyphenated).
    pub entry_id: Option<String>,
    /// What landed, for `"change"` (UUID hyphenated).
    pub change_point_id: Option<String>,
}

/// One thing said, with every correction to it.
///
/// `said` is what it says now and `first_said` is what it said when it
/// was written; they are equal until somebody corrects it. `revisions`
/// carries each correction in the order they were made, so a reader can
/// show the change rather than only its result.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeMessageDto {
    /// Message id (UUID hyphenated).
    pub id: String,
    /// What it replies to, when it replies to something (UUID
    /// hyphenated).
    pub parent_id: Option<String>,
    /// What it says now — the last correction, or the original when
    /// there has been none.
    pub said: String,
    /// What it said when it was written.
    pub first_said: String,
    /// When it was written (unix epoch ms).
    pub at_ms: i64,
    /// `"user"` or `"system"`.
    pub actor_kind: String,
    /// Who wrote it (UUID hyphenated).
    pub actor_id: String,
    /// Every correction, oldest first.
    pub revisions: Vec<ForgeRevisionDto>,
}

/// One correction to something said.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct ForgeRevisionDto {
    /// What it says from here on.
    pub said: String,
    /// When it was corrected (unix epoch ms).
    pub at_ms: i64,
    /// `"user"` or `"system"`.
    pub actor_kind: String,
    /// Who corrected it (UUID hyphenated).
    pub actor_id: String,
}

/// Opens a conversation about something in the forge.
///
/// The anchor is named by ids and `kind`, and resolved by the service
/// rather than trusted: it reads the pursuit or the line and the model
/// builds the anchor, so an entry a round never touched is refused
/// rather than recorded.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct OpenForgeThreadCommand {
    /// `"pursuit"`, `"round"`, `"entry"` or `"change"`.
    pub anchor_kind: String,
    /// The work this is about (UUID hyphenated). Required for every
    /// kind but `"change"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pursuit_id: Option<String>,
    /// The line, for `"change"` (UUID hyphenated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_id: Option<String>,
    /// The round, for `"round"` and `"entry"` (UUID hyphenated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// The entry, for `"entry"` (UUID hyphenated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    /// What landed, for `"change"` (UUID hyphenated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_point_id: Option<String>,
    /// A name for the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The first thing said. A conversation is what was said in it, so
    /// there is no opening one empty.
    pub said: String,
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

/// Says something in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct SayInForgeThreadCommand {
    /// Target thread id (UUID hyphenated). Taken from the path over
    /// HTTP.
    #[serde(default)]
    pub thread_id: String,
    /// What this answers, when it answers something (UUID hyphenated).
    /// A message of another conversation is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replying_to: Option<String>,
    /// What is being said.
    pub said: String,
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

/// Corrects something said. Nothing is overwritten.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct AmendForgeMessageCommand {
    /// Target thread id (UUID hyphenated). Taken from the path over
    /// HTTP.
    #[serde(default)]
    pub thread_id: String,
    /// Which message to correct (UUID hyphenated).
    pub message_id: String,
    /// What it says from here on.
    pub said: String,
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

/// Renames a conversation, or takes its name off.
///
/// `title` absent means take it off, which is why it is not
/// `skip_serializing_if`: an absent field and a field set to null have
/// to mean the same thing here, and both mean "no name".
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct RenameForgeThreadCommand {
    /// Target thread id (UUID hyphenated). Taken from the path over
    /// HTTP.
    #[serde(default)]
    pub thread_id: String,
    /// The new name, or absent to take the name off.
    #[serde(default)]
    pub title: Option<String>,
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
