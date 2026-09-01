//! The team plane's shapes, as they cross this app's own boundary.
//!
//! A topic module for the same reason [`forge`](crate::forge) is one:
//! the team plane answers to a model of its own, and a reader of
//! [`TeamLedgerPageDto`] needs the types beside it rather than the
//! asset DTOs two hundred lines up.
//!
//! # Why these exist here as well as on the wire
//!
//! Two boundaries, not one. `asterism-teams-wire` carries what a
//! member's client and a team server say to each other over HTTP; this
//! carries what the desktop app says to its own frontend. They meet at
//! one place — the Tauri command — and the shapes look alike there
//! because the same facts cross both.
//!
//! Alike is not the same as shared. This crate imports no other
//! Asterism crate, which is what keeps it a leaf and what stops a
//! dependency cycle; and the frontend has one vocabulary rather than
//! two, which is what `bindings.ts` being a projection of *this* crate
//! means. A command that returned a wire type would hand the second
//! vocabulary to every screen that called it.
//!
//! So the duplication is the boundary, and the mapping lives in the
//! command that crosses it.
//!
//! # What the ledger's shape carries
//!
//! An append-only record of what a team did, in what capacity. The
//! read is paged over `seq` rather than whole, because the forge
//! writes a row per push and a table that grew by the occasional
//! membership gesture does not any more.
//!
//! Two properties of it decide how a screen may read it, and both are
//! stated on the fields they belong to: a null cursor is not an end,
//! and an actor's display name is a snapshot rather than a lookup.

use serde::{Deserialize, Serialize};

use schema_bridge::SchemaBridge;

/// One page of a team's ledger, oldest first.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamLedgerPageDto {
    /// The events on this page, `seq` ascending.
    pub events: Vec<TeamLedgerEventDto>,
    /// Where the next page resumes, or `null`.
    ///
    /// **A null cursor is not the end of the ledger.** A page that
    /// filled the limit it asked for always carries one — even when it
    /// happened to end on the last event there is, because whether
    /// anything follows is only answerable by asking. So null means a
    /// short page, and a short page says nothing lay past here *when it
    /// was taken*. A ledger has no final page; a caller following one
    /// keeps the last `seq` it saw and asks again.
    pub next_after: Option<i64>,
}

/// One act, as the ledger recorded it.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamLedgerEventDto {
    /// Position within the team's stream, from 1.
    pub seq: i64,
    /// Globally unique id of the event.
    pub event_id: String,
    /// The stream it belongs to.
    pub team_id: String,
    /// `"member"` or `"admin"`.
    ///
    /// The capacity an act was performed in, kept apart from who
    /// performed it: an instance admin acting inside a team without a
    /// membership row is recorded as an admin and never as a member.
    /// A surface showing the actor without this is answering half the
    /// question the ledger exists for.
    pub actor_kind: String,
    /// Who acted.
    pub actor_user_id: String,
    /// The actor's display name **as it read when the act happened**.
    ///
    /// A snapshot rather than a reference. Resolving it against
    /// whatever the name is now would let a rename change a record of
    /// something that already happened.
    pub actor_display_name: String,
    /// When, epoch ms.
    pub occurred_at_ms: i64,
    /// The namespaced and versioned kind, e.g.
    /// `"teams.membership.role_changed/1"`.
    ///
    /// The version is part of the identity rather than decoration, and
    /// the set grows — a caller meeting a kind it has never seen is an
    /// ordinary event and not an error.
    pub kind: String,
    /// The typed references this act makes.
    pub subjects: Vec<TeamSubjectRefDto>,
    /// The kind-versioned body, serialised.
    ///
    /// Opaque here on purpose: what a body carries is fixed by its
    /// kind, and a type that named the fields of one kind would have to
    /// name them for every kind the server ever adds.
    pub payload_json: String,
}

/// One typed reference an act makes.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamSubjectRefDto {
    /// The reference's kind: `"digest"`, `"user"`, `"blob"` or
    /// `"forge_identity"`.
    pub ref_type: String,
    /// The reference's value in its storage spelling.
    pub value: String,
}

/// The teams this window's account belongs to.
///
/// The roster's question turned around: that one takes a team and
/// answers with users, this takes the account and answers with teams.
///
/// **Membership rather than reach**, which matters for one caller: an
/// admin acts inside a team without a membership row (#83 §1), so an
/// admin who joined nothing sees an empty list while keeping every
/// capacity they had. Stated here rather than left to the route
/// because it is a screen that draws the wrong conclusion: an empty
/// list must not be read as "no access".
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MyTeamsDto {
    /// One row per membership, oldest team first.
    pub teams: Vec<MyTeamDto>,
}

/// One team the account belongs to.
///
/// **No name, because a team has none**, which the team plane's own
/// `TeamMembership` argues: naming one is a change to that model
/// rather than a field this shape is missing. What follows here is
/// that a screen over these rows shows ids.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct MyTeamDto {
    /// The team, and what every team-scoped read is named by.
    pub team_id: String,
    /// What this account is in it: `"owner"` or `"member"`.
    pub role: String,
    /// When the team was created, unix epoch milliseconds.
    ///
    /// The team's own, not the membership's — a membership row carries
    /// no time, so this is the only order these rows have.
    pub created_at_ms: i64,
}

/// Who is in a team, and in what role.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamRosterDto {
    /// The team the roster describes.
    pub team_id: String,
    /// One row per member.
    pub members: Vec<TeamRosterMemberDto>,
}

/// One membership row.
///
/// **No display name.** A membership is a row about an account rather
/// than about a person, and the name a ledger event carries is a
/// snapshot the *act* took — there is nothing equivalent to read here.
/// A surface listing these shows ids, and saying why is cheaper than
/// leaving a reader to wonder how one screen has names and the other
/// does not.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamRosterMemberDto {
    /// The member.
    pub user_id: String,
    /// `"owner"` or `"member"`.
    ///
    /// The authority table's distinction, and what decides which
    /// membership verbs a viewer may reach: an owner's are not a
    /// member's.
    pub role: String,
}

/// What founding a team answers with.
///
/// The id alone. The wire's version carries the ledger event the
/// creation appended as well, and a screen has somewhere better to
/// read that — the ledger tab, where every act of the team's is. What
/// a create form needs is the id, because that is what the field above
/// the tabs wants next.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamCreatedDto {
    /// The new team's id — the path segment of every team-scoped read.
    pub team_id: String,
}

/// What a promotion left behind, as a screen needs to read it.
///
/// The client's own outcome carries more — the relation key whole, and
/// the pursuit as it now reads — and neither crosses. The key's team
/// and line are what the caller passed in, so answering with them
/// would be handing back an argument; the pursuit is a `ForgePursuitDto`
/// the work surface re-reads for itself, and returning a second copy
/// would give two screens two answers about one piece of work.
///
/// What is here is what only the promotion knows.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct PromotedAssetDto {
    /// The entry the round named, minted by this client (#148
    /// decision 8). What the relation row at home is keyed on, and
    /// what a screen would need to find this entry on the line again.
    pub entry_id: String,
    /// The `TeamAsset` the team minted, or nothing when this was a
    /// repeat and nothing was sent.
    ///
    /// An opaque handle, never read as a local asset id (#148
    /// decision 6).
    pub team_asset_id: Option<String>,
    /// What the material hashed to at promote time.
    ///
    /// Read then rather than taken from what was stored, for the
    /// reason `promotion::hash_at_promote_time` gives. It is read
    /// before the client knows whether anything will be sent, so on a
    /// repeat this is the file as it is now and not what the team
    /// took.
    pub digest: String,
    /// Whether the team already held those bytes when asked, or
    /// nothing when the question was never put.
    ///
    /// Three states rather than two, and a screen has to keep them
    /// apart: "the team already has these", "the team did not", and
    /// "nobody asked" — the last on a repeat, where nothing was going
    /// to be sent. Reported rather than acted on, for the reason
    /// `asterism-teams-client`'s promotion module sets out under "The
    /// have-check, honestly".
    pub bytes_already_held: Option<bool>,
    /// Whether this machine had already promoted this asset onto this
    /// line, in which case nothing was sent and nothing was written.
    ///
    /// **Not a guarantee that the team holds it once.** It is a read
    /// of a relation row this machine wrote, so it says what this
    /// machine did rather than what the team has: another member's
    /// promotion of the same asset is a second `TeamAsset` by
    /// decision 7, and this cannot see it.
    ///
    /// A repeat also carries nothing across. A description edited
    /// since the first promotion stays home, because replacing a
    /// projection is a forge op and needs a round of its own.
    pub already_promoted: bool,
}

/// The device tokens this window's account holds, on whatever machines
/// (#204).
///
/// Owner-scoped, which the route decides and the wire's own
/// `DeviceTokensDto` argues: there is no admin-facing sibling, and
/// nothing here takes an account to ask about.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamDeviceTokensDto {
    /// One row per live token, oldest mint first.
    pub tokens: Vec<TeamDeviceTokenDto>,
}

/// One device token as its owner sees it.
///
/// **Nothing here authenticates anybody** — not the token, not its
/// hash, not a prefix of either — which is what lets a listing exist
/// at all. The wire shape this maps from states the same property; it
/// is repeated because it is the reason a screen may draw these rows
/// and the reason `id` is safe in a file.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct TeamDeviceTokenDto {
    /// The handle to revoke by.
    pub id: String,
    /// What the client called that device when it asked.
    pub label: String,
    /// When it was minted, unix epoch milliseconds.
    pub created_at_ms: i64,
    /// When it was last presented, unix epoch milliseconds.
    ///
    /// Absent for a token nobody has used yet, which is a different
    /// fact from "used at the moment it was made" and is why this is
    /// not seeded from the mint time.
    pub last_used_at_ms: Option<i64>,
    /// When it stops resolving, unix epoch milliseconds.
    pub expires_at_ms: i64,
}

/// What this machine remembers about a team server, none of which
/// authenticates anybody (#204).
///
/// The connect form pre-fills from this. Where the halves live and why
/// the token is not among them is on `stored_connection` in the
/// desktop binary, which owns both stores.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct StoredTeamConnectionDto {
    /// The server the token was minted by, as it was typed.
    pub base_url: String,
    /// The account it was minted for.
    pub login: String,
    /// The stored token's revocation handle, so a listing can mark
    /// which row is the machine reading it.
    pub token_id: String,
    /// What this device asked to be called in that listing.
    pub label: String,
}

/// What came of trying to connect from what this machine had stored
/// (#204).
///
/// Three outcomes rather than two, and the third is why this is a
/// shape rather than a `boolean`: a window that was never remembered
/// and one whose token has been revoked both end at the password form,
/// but only the second is worth telling somebody about.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaBridge)]
pub struct StoredTeamConnectDto {
    /// Which of the three happened.
    pub outcome: StoredTeamConnectOutcome,
    /// Whose account the server named. Present for `connected` and for
    /// nothing else.
    pub user: Option<String>,
}

/// The three ends of a silent reconnect.
///
/// **None of them is a failure.** A reconnect nobody asked for is
/// attempted whenever the panel opens, so a refusal reported as an
/// error would put a message in front of somebody who did nothing. A
/// server that is unreachable or shouting is a different matter and
/// does reach the caller as an error, because that is a fact about the
/// attempt rather than about the stored credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SchemaBridge)]
#[serde(rename_all = "snake_case")]
pub enum StoredTeamConnectOutcome {
    /// Connected, on the session shape the password arm also yields.
    Connected,
    /// Nothing was tried, because nothing this machine could present
    /// was in hand.
    ///
    /// Three ways in, and they are one answer because the caller's
    /// next act is the same in all of them — show the password form:
    /// no metadata at all; metadata whose keychain entry is gone; and
    /// metadata whose keychain would not answer, which is a locked
    /// store, a prompt somebody dismissed, or a session with no
    /// credential store behind it.
    ///
    /// **What became of the file differs between them, and is not
    /// reported here.** An entry that is gone takes the metadata with
    /// it, since a file describing a connection that cannot be made
    /// pre-fills a form into a reconnect nothing can complete. A
    /// keychain that would not answer leaves the file where it was:
    /// the file is the sole index of the keychain, the entry it names
    /// is most likely still sitting there, and one dismissed prompt
    /// must not be what deletes the only record of it. No field here
    /// tells the two apart, because a caller that needs to know reads
    /// `stored_team_connection` back afterwards — the same rule the
    /// panel follows at every other site that changes what is stored —
    /// rather than being handed a fact by a command that is done with
    /// it.
    ///
    /// What separates this from [`Self::Rejected`] is who said no. Here
    /// no server was asked; there one was, and refused.
    Nothing,
    /// What was stored no longer resolves. Both halves of it are
    /// forgotten by the time this is answered.
    Rejected,
}
