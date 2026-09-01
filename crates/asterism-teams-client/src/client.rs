//! Talking to a team server (#148 decisions 16 and 19).
//!
//! ## Served through, never mirrored
//!
//! Decision 16: reads and writes go to the server, there is no local
//! copy of a shared line, and therefore no staleness to reason about.
//! So every method here is a request. There is no cache, no `sync`,
//! and no "refresh" — and there should not be one, because a mirror
//! would be the weaker version of a clone with a cache attached, and a
//! clone is #153's.
//!
//! ## The shape of the surface
//!
//! Decision 19: the transport is the local forge's verbs mirrored
//! under `/teams/{team_id}/forge/*`, plus a content verb scoped to a
//! pursuit, a bulk resolve and a have-check.
//!
//! **This crate implements a subset of that, and the subset is the
//! design.** #152 is a promotion and the reads a member needs around
//! one, so the line and pursuit verbs a promotion walks are here and
//! the conversation verbs are not — nothing in this issue says
//! anything in a thread. What every path here does promise is to be
//! the router's verbatim: a path spelled differently on the two sides
//! is a bug in one of them, and that is the claim worth checking.
//!
//! ## Ids that come back are handles
//!
//! Every id a team states arrives as a
//! [`TeamScopedId`](asterism_core::domain::team_link::TeamScopedId),
//! which has no conversion to or from a local `AssetId` in either
//! direction (#148 decision 6). What crosses in the other direction is
//! a subject and a digest, which is what the decision says may.

use std::path::Path;

use asterism_contract::forge::{
    CloseForgePursuitCommand, ForgeDiscardedDto, ForgeEntryStateDto, ForgeLineActCommand,
    ForgeLineDto, ForgeLineHistoryDto, ForgeOpDto, ForgePursuitDto, OpenForgeLineCommand,
    OpenForgePursuitCommand, PushForgeRoundCommand,
};
use asterism_core::domain::team_link::TeamScopedId;
use asterism_core::error::DomainError;
use asterism_teams_wire::command::{
    CreateTeamCommand, DeviceLoginCommand, HaveContentCommand, LoginCommand,
    MintDeviceTokenCommand, ResolveContentCommand,
};
use asterism_teams_wire::dto::{
    ContentEnteredDto, DeviceTokenMintedDto, DeviceTokensDto, HeldContentDto, LedgerPageDto,
    MyTeamsDto, ResolvedContentDto, RosterDto, SessionDto, TeamCreatedDto,
};
use asterism_teams_wire::projection::{
    EntryProjectionDto, EntryProjectionEnvelope, WithProjections,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::AsyncWriteExt as _;
use tokio_util::io::ReaderStream;

/// What can go wrong between here and a team.
#[derive(Debug, thiserror::Error)]
pub enum TeamsClientError {
    /// The request never got an answer.
    #[error("talking to {url}: {source}")]
    Transport {
        /// What was being asked for.
        url: String,
        /// The underlying failure.
        #[source]
        source: reqwest::Error,
    },

    /// The server answered, and the answer was a refusal.
    ///
    /// Carries `reason` because the forge's conflicts do (`blocked`,
    /// `raced`, `settled`, `clashes`), and it is the token that tells a
    /// caller whether retrying is worth anything. Dropping it would
    /// leave a client with a 409 it cannot act on.
    #[error("{method} {url}: HTTP {status} {kind}: {message}")]
    Refused {
        /// The verb that was refused.
        method: &'static str,
        /// The path it was aimed at.
        url: String,
        /// The status code.
        status: u16,
        /// The house error body's `kind`.
        kind: String,
        /// The house error body's `message`.
        message: String,
        /// The conflict token, when the refusal was a conflict.
        reason: Option<String>,
    },

    /// The answer did not decode into what the route promises.
    #[error("decoding the answer to {url}: {message}")]
    Decode {
        /// What was being asked for.
        url: String,
        /// What went wrong.
        message: String,
    },

    /// A call that needs a session was made before [`TeamsClient::login`].
    #[error("no session: log in before calling {what}")]
    NoSession {
        /// The call that wanted one.
        what: &'static str,
    },

    /// Something local refused before anything was sent.
    #[error(transparent)]
    Local(#[from] DomainError),
}

/// A client bound to one team server, holding at most one session.
#[derive(Clone)]
pub struct TeamsClient {
    base_url: String,
    http: reqwest::Client,
    session: Option<SessionDto>,
}

/// Written by hand because the derived one prints the bearer token.
///
/// A session token is a credential with a live server behind it, and
/// `Debug` is what ends up in a panic message, a test failure and a
/// log line — the three places a value is copied out of by someone who
/// was looking at something else. So the token is not here, and what
/// is here is what a person debugging actually needs: which server,
/// and whether there is a session at all.
impl std::fmt::Debug for TeamsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeamsClient")
            .field("base_url", &self.base_url)
            .field(
                "session",
                &self
                    .session
                    .as_ref()
                    .map_or("none", |_| "held (token not shown)"),
            )
            .finish()
    }
}

impl TeamsClient {
    /// Points a client at a server. No request is made here.
    ///
    /// `base_url` is the origin without a trailing slash, e.g.
    /// `http://127.0.0.1:8787`.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
            session: None,
        }
    }

    /// The session, if there is one. Its token must not be logged, and
    /// [`SessionDto`]'s hand-written `Debug` is what keeps it out of
    /// the places a value gets copied from by accident.
    pub const fn session(&self) -> Option<&SessionDto> {
        self.session.as_ref()
    }

    /// Who the server says this client is, as a subject.
    ///
    /// The account's user id rather than its display name: decision 6
    /// puts author subjects and viewer subjects in one namespace, and a
    /// display name moves.
    pub fn user_id(&self) -> Option<&str> {
        self.session.as_ref().map(|it| it.user_id.as_str())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn token(&self, what: &'static str) -> Result<&str, TeamsClientError> {
        self.session
            .as_ref()
            .map(|it| it.token.as_str())
            .ok_or(TeamsClientError::NoSession { what })
    }

    // ------------------------------------------------------------------
    // Session (#83 §5), and the device token that makes one without a
    // password (#204).
    // ------------------------------------------------------------------

    /// Logs in, and keeps the session for every call after this one.
    ///
    /// A wrong password and an unknown login are the same `401`; the
    /// API does not say which half failed, and neither does this.
    pub async fn login(
        &mut self,
        login: &str,
        password: &str,
    ) -> Result<SessionDto, TeamsClientError> {
        let url = self.url("/teams/auth/login");
        let response = self
            .http
            .post(&url)
            .json(&LoginCommand {
                login: login.to_string(),
                password: password.to_string(),
            })
            .send()
            .await
            .map_err(|source| TeamsClientError::Transport {
                url: url.clone(),
                source,
            })?;
        let session: SessionDto = read_json("POST", &url, response).await?;
        self.session = Some(session.clone());
        Ok(session)
    }

    /// Logs in from a stored device token, and keeps the session for
    /// every call after this one (#204).
    ///
    /// The other half of [`Self::mint_device_token`], and the reason
    /// the pair exists: a restart reconnects from what the keychain
    /// held instead of asking for a password. What comes back is an
    /// ordinary session — the same shape [`Self::login`] returns — so
    /// nothing below this method knows which arm was used.
    ///
    /// An unknown, revoked and expired token are the same `401`. That
    /// is the client's cue to fall back to the password form, and the
    /// one thing it must not do on that path is keep the stored token:
    /// deciding when a stored credential is discarded belongs to
    /// whoever owns the keychain entry, so this method changes nothing
    /// on disk in either direction.
    pub async fn login_with_device_token(
        &mut self,
        token: &str,
    ) -> Result<SessionDto, TeamsClientError> {
        let url = self.url("/teams/auth/device/login");
        let response = self
            .http
            .post(&url)
            .json(&DeviceLoginCommand {
                token: token.to_string(),
            })
            .send()
            .await
            .map_err(|source| TeamsClientError::Transport {
                url: url.clone(),
                source,
            })?;
        let session: SessionDto = read_json("POST", &url, response).await?;
        self.session = Some(session.clone());
        Ok(session)
    }

    /// Asks the server for a device token this machine may keep
    /// (#204).
    ///
    /// Needs a session, and any live one will do — which is what lets
    /// a client reach this after logging in whichever way the instance
    /// verifies people.
    ///
    /// **The returned token is the only copy that will ever exist
    /// outside this process.** The server holds its SHA-256 and can
    /// answer with it again as little as the caller can recover it, so
    /// a caller that means to keep it has to store it before dropping
    /// the response. Where is the caller's own question, and this
    /// crate does not answer it — the desktop puts it in the OS
    /// keychain and states why in `stored_connection`; another caller
    /// may have somewhere else, or nowhere.
    pub async fn mint_device_token(
        &self,
        label: &str,
    ) -> Result<DeviceTokenMintedDto, TeamsClientError> {
        self.post(
            "/teams/auth/device",
            "mint_device_token",
            &MintDeviceTokenCommand {
                label: label.to_string(),
            },
        )
        .await
    }

    /// The device tokens this session's account holds — labels,
    /// handles and times, never a value.
    ///
    /// Owner-scoped by the route: there is no argument for whose
    /// tokens to list, because the answer is always the caller's.
    pub async fn list_device_tokens(&self) -> Result<DeviceTokensDto, TeamsClientError> {
        self.get("/teams/auth/device", "list_device_tokens").await
    }

    /// Revokes one of this account's device tokens by the handle
    /// [`list_device_tokens`](Self::list_device_tokens) named it by.
    ///
    /// Succeeds for a handle that names nothing, exactly as the route
    /// does — so a client reconciling a keychain against a listing can
    /// revoke without first checking, and learns nothing about ids
    /// that are not its own.
    ///
    /// Sessions already minted from the token live out their own TTL;
    /// revoking stops the next one.
    pub async fn revoke_device_token(&self, id: &str) -> Result<(), TeamsClientError> {
        self.delete(&format!("/teams/auth/device/{id}"), "revoke_device_token")
            .await
    }

    /// Ends the session, here and on the server.
    ///
    /// The local half happens whatever the server says: a session this
    /// client will not present again is over from its own point of
    /// view, and a network failure on the way out must not leave a
    /// token sitting in memory.
    pub async fn logout(&mut self) -> Result<(), TeamsClientError> {
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        let url = self.url("/teams/auth/logout");
        let response = self
            .http
            .post(&url)
            .bearer_auth(&session.token)
            .send()
            .await
            .map_err(|source| TeamsClientError::Transport {
                url: url.clone(),
                source,
            })?;
        refusal("POST", &url, response).await
    }

    // ------------------------------------------------------------------
    // A team.
    // ------------------------------------------------------------------

    /// Founds a team (`POST /teams/create`).
    pub async fn create_team(
        &self,
        owner_user_id: Option<&str>,
    ) -> Result<TeamCreatedDto, TeamsClientError> {
        self.post(
            "/teams/create",
            "create_team",
            &CreateTeamCommand {
                owner_user_id: owner_user_id.map(str::to_string),
            },
        )
        .await
    }

    /// The teams this session's account is a member of.
    ///
    /// Takes no [`TeamScopedId`], because it is what a caller asks
    /// before they have one. What it answers — membership rather than
    /// reach — is the route's decision and is argued there.
    pub async fn my_teams(&self) -> Result<MyTeamsDto, TeamsClientError> {
        self.get("/teams", "my_teams").await
    }

    /// The team's current membership set.
    pub async fn roster(&self, team: TeamScopedId) -> Result<RosterDto, TeamsClientError> {
        self.get(&format!("/teams/{team}/roster"), "roster").await
    }

    /// One page of the team's stream, seq ascending (#148 decision 18).
    ///
    /// `after` is the `next_after` of the page before, or nothing for
    /// the first. A page whose `next_after` is `null` says nothing lay
    /// past here *when it was taken* — a ledger has no final page, so a
    /// caller following a live stream keeps the last seq it saw and
    /// asks again.
    pub async fn events(
        &self,
        team: TeamScopedId,
        after: Option<i64>,
        limit: Option<u32>,
    ) -> Result<LedgerPageDto, TeamsClientError> {
        let mut path = format!("/teams/{team}/events");
        let mut sep = '?';
        if let Some(after) = after {
            path.push_str(&format!("{sep}after={after}"));
            sep = '&';
        }
        if let Some(limit) = limit {
            path.push_str(&format!("{sep}limit={limit}"));
        }
        self.get(&path, "events").await
    }

    // ------------------------------------------------------------------
    // The shared lines, served through (#148 decision 16).
    // ------------------------------------------------------------------

    /// Every line this team hosts, without its history.
    ///
    /// The panel the UI puts these in is its own, separate from the
    /// local lines — which is what having two sources honestly looks
    /// like (decision 16).
    pub async fn lines(&self, team: TeamScopedId) -> Result<Vec<ForgeLineDto>, TeamsClientError> {
        self.get(&format!("/teams/{team}/forge/lines"), "lines")
            .await
    }

    /// Opens a line on the team's forge. A member's act (#148 revision
    /// 5); the name is unique within the team.
    pub async fn open_line(
        &self,
        team: TeamScopedId,
        name: &str,
        strategy_id: &str,
    ) -> Result<ForgeLineDto, TeamsClientError> {
        self.post(
            &format!("/teams/{team}/forge/lines"),
            "open_line",
            &OpenForgeLineCommand {
                name: name.to_string(),
                strategy_id: strategy_id.to_string(),
                // The three attribution fields the mirror refuses: on a
                // team's forge the author is the authenticated member,
                // and a command that stated one would be a second
                // answer to a settled question (#148 revision 6).
                author_kind: None,
                author_subject: None,
                operator_ai: None,
            },
        )
        .await
    }

    /// Marks the line finished with. A member's act, and reversible by
    /// [`Self::reopen_line`].
    ///
    /// Here because the discard needs it: the forge drops a line from
    /// the archive rather than from active use, so archiving is the
    /// step before. Its inverse is here for the same reason a door
    /// that opens should close — a surface that can archive and cannot
    /// reopen would leave a caller stuck at a state it reached by
    /// accident.
    pub async fn archive_line(
        &self,
        team: TeamScopedId,
        line: TeamScopedId,
    ) -> Result<ForgeLineDto, TeamsClientError> {
        self.act_on_line(team, line, "archive", "archive_line")
            .await
    }

    /// Takes the line back out of the archive.
    pub async fn reopen_line(
        &self,
        team: TeamScopedId,
        line: TeamScopedId,
    ) -> Result<ForgeLineDto, TeamsClientError> {
        self.act_on_line(team, line, "reopen", "reopen_line").await
    }

    async fn act_on_line(
        &self,
        team: TeamScopedId,
        line: TeamScopedId,
        verb: &str,
        what: &'static str,
    ) -> Result<ForgeLineDto, TeamsClientError> {
        self.post(
            &format!("/teams/{team}/forge/lines/{line}/{verb}"),
            what,
            &ForgeLineActCommand {
                line_id: line.to_string(),
                author_kind: None,
                author_subject: None,
                operator_ai: None,
            },
        )
        .await
    }

    /// Takes the line, its history and every piece of work against it.
    ///
    /// **The one verb on this surface that asks more than membership**
    /// (#148 revision 5): it is the verb that takes the log with it, so
    /// the server wants an owner and answers `403` to anyone else.
    ///
    /// The response names the assets the forge was holding and is not
    /// holding any more, and after this write there is no record left
    /// to derive them from — which is also why a member's relation rows
    /// for this line all dangle afterwards, and why
    /// [`verify`](crate::link::verify) exists.
    pub async fn discard_line(
        &self,
        team: TeamScopedId,
        line: TeamScopedId,
    ) -> Result<ForgeDiscardedDto, TeamsClientError> {
        self.post(
            &format!("/teams/{team}/forge/lines/{line}/discard"),
            "discard_line",
            &ForgeLineActCommand {
                line_id: line.to_string(),
                author_kind: None,
                author_subject: None,
                operator_ai: None,
            },
        )
        .await
    }

    /// A line and its whole history.
    pub async fn line_history(
        &self,
        team: TeamScopedId,
        line: TeamScopedId,
    ) -> Result<ForgeLineHistoryDto, TeamsClientError> {
        self.get(&format!("/teams/{team}/forge/lines/{line}"), "line_history")
            .await
    }

    /// What is on the line now, folded from the chain.
    ///
    /// The read a verify uses for its team half: an entry that is not
    /// here any more is one a link row may be dangling against
    /// (#148 decision 9).
    pub async fn line_states(
        &self,
        team: TeamScopedId,
        line: TeamScopedId,
    ) -> Result<Vec<ForgeEntryStateDto>, TeamsClientError> {
        self.get(
            &format!("/teams/{team}/forge/lines/{line}/states"),
            "line_states",
        )
        .await
    }

    /// Every piece of work against a line, open and ended alike.
    pub async fn pursuits_of_line(
        &self,
        team: TeamScopedId,
        line: TeamScopedId,
    ) -> Result<Vec<ForgePursuitDto>, TeamsClientError> {
        self.get(
            &format!("/teams/{team}/forge/lines/{line}/pursuits"),
            "pursuits_of_line",
        )
        .await
    }

    /// Opens work against a line.
    pub async fn open_pursuit(
        &self,
        team: TeamScopedId,
        line: TeamScopedId,
        title: Option<&str>,
        note: Option<&str>,
    ) -> Result<ForgePursuitDto, TeamsClientError> {
        self.post(
            &format!("/teams/{team}/forge/pursuits"),
            "open_pursuit",
            &OpenForgePursuitCommand {
                line_id: line.to_string(),
                parent_id: None,
                title: title.map(str::to_string),
                note: note.map(str::to_string),
                author_kind: None,
                author_subject: None,
                operator_ai: None,
            },
        )
        .await
    }

    /// The work, whole.
    pub async fn pursuit(
        &self,
        team: TeamScopedId,
        pursuit: TeamScopedId,
    ) -> Result<ForgePursuitDto, TeamsClientError> {
        self.get(
            &format!("/teams/{team}/forge/pursuits/{pursuit}"),
            "pursuit",
        )
        .await
    }

    /// Writes a round, carrying whatever descriptions ride with it.
    ///
    /// The projections are a separate argument rather than a field of
    /// the ops for the reason decision 12 gives: a projection is
    /// captured beside the forge rather than being part of what the
    /// forge records. On the wire they flatten into the same body, so a
    /// push with none is byte-for-byte the mirror's own request.
    pub async fn push_round(
        &self,
        team: TeamScopedId,
        pursuit: TeamScopedId,
        ops: Vec<ForgeOpDto>,
        note: Option<&str>,
        projections: Vec<EntryProjectionEnvelope>,
    ) -> Result<ForgePursuitDto, TeamsClientError> {
        self.post(
            &format!("/teams/{team}/forge/pursuits/{pursuit}/push"),
            "push_round",
            &WithProjections {
                push: PushForgeRoundCommand {
                    pursuit_id: pursuit.to_string(),
                    ops,
                    note: note.map(str::to_string),
                    author_kind: None,
                    author_subject: None,
                    operator_ai: None,
                },
                projections,
            },
        )
        .await
    }

    /// Ends the work, and puts what it says on the line if it says
    /// anything.
    pub async fn close_pursuit(
        &self,
        team: TeamScopedId,
        pursuit: TeamScopedId,
        outcome: &str,
        note: Option<&str>,
    ) -> Result<ForgePursuitDto, TeamsClientError> {
        self.post(
            &format!("/teams/{team}/forge/pursuits/{pursuit}/close"),
            "close_pursuit",
            &CloseForgePursuitCommand {
                pursuit_id: pursuit.to_string(),
                outcome: outcome.to_string(),
                note: note.map(str::to_string),
                author_kind: None,
                author_subject: None,
                operator_ai: None,
            },
        )
        .await
    }

    // ------------------------------------------------------------------
    // Content — the verbs hosting adds (#148 decisions 5 and 19).
    // ------------------------------------------------------------------

    /// Streams a material into the team against open work, and answers
    /// with the `TeamAsset` the team minted for it.
    ///
    /// **The entry point is a forge op and there is exactly one**
    /// (decision 5), which is why this takes a pursuit: the team never
    /// holds an Asset that is not attached to work, and the content is
    /// there before the round that names it.
    ///
    /// The file is streamed rather than read: a material is whatever a
    /// person put in their library, and the largest allocation in a
    /// promoting process should not be the size of the largest file it
    /// promotes.
    ///
    /// `digest` is what the caller declares the bytes to be, in
    /// `sha256:<64hex>`. The server hashes while it writes and refuses
    /// the whole operation on a mismatch — no blob, no asset, no event.
    pub async fn enter_content(
        &self,
        team: TeamScopedId,
        pursuit: TeamScopedId,
        digest: &str,
        material: &Path,
    ) -> Result<ContentEnteredDto, TeamsClientError> {
        let token = self.token("enter_content")?.to_string();
        let path = format!("/teams/{team}/forge/pursuits/{pursuit}/content?digest={digest}",);
        let url = self.url(&path);
        let file = tokio::fs::File::open(material).await.map_err(|err| {
            TeamsClientError::Local(DomainError::Infra(anyhow::anyhow!(
                "opening {} to promote it: {err}",
                material.display()
            )))
        })?;
        let response = self
            .http
            .put(&url)
            .bearer_auth(token)
            .body(reqwest::Body::wrap_stream(ReaderStream::new(file)))
            .send()
            .await
            .map_err(|source| TeamsClientError::Transport {
                url: url.clone(),
                source,
            })?;
        read_json("PUT", &url, response).await
    }

    /// Which of these digests the team already holds.
    ///
    /// **Its only purpose is to let a client skip a send.** What it can
    /// reveal is what the asker could learn by uploading, one round
    /// trip earlier, which is why it is safe inside a team and would
    /// not be a route away from one (#83 §3).
    pub async fn have_content(
        &self,
        team: TeamScopedId,
        digests: Vec<String>,
    ) -> Result<HeldContentDto, TeamsClientError> {
        self.post(
            &format!("/teams/{team}/forge/content/have"),
            "have_content",
            &HaveContentCommand { digests },
        )
        .await
    }

    /// Streams the bytes behind a digest onto this machine, and answers
    /// with how many arrived.
    ///
    /// The other direction of [`enter_content`](Self::enter_content),
    /// and the verb a clone is built on (#148 decision 10): working on
    /// a shared line needs no copy, and this is for when the copy is
    /// the point. Streamed for the reason the upload is — the file at
    /// the far end is whatever somebody promoted, and a clone that read
    /// it into a `Vec` first would size the process by the largest
    /// thing any member ever brought.
    ///
    /// The digest is the identifier, not the team asset id: the team
    /// mints an asset per promotion over one stored copy (decision 7),
    /// so several assets share a digest and the bytes are the digest's.
    /// [`resolve_content`](Self::resolve_content) is what turns the id a
    /// round names into the digest to ask for here.
    ///
    /// `into` is created and truncated. A refusal leaves nothing behind
    /// — the file is opened only once the server has answered, so a
    /// caller never has to tell a failed download from an empty one.
    pub async fn fetch_content(
        &self,
        team: TeamScopedId,
        digest: &str,
        into: &Path,
    ) -> Result<u64, TeamsClientError> {
        let token = self.token("fetch_content")?.to_string();
        let url = self.url(&format!("/teams/{team}/blobs/{digest}"));
        let mut response = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|source| TeamsClientError::Transport {
                url: url.clone(),
                source,
            })?;
        // Read the status before the file exists. A 404 here is the
        // ordinary answer for a digest this team does not hold, and it
        // must not also leave a truncated file where the clone was
        // going. The refusal is decoded the long way rather than
        // through `refusal`, which takes the response whole — and this
        // one is a stream that is about to be consumed a chunk at a
        // time.
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(refused("GET", &url, status.as_u16(), &body));
        }
        let mut file = tokio::fs::File::create(into).await.map_err(|err| {
            TeamsClientError::Local(DomainError::Infra(anyhow::anyhow!(
                "creating {} to clone into: {err}",
                into.display()
            )))
        })?;
        let mut written = 0u64;
        while let Some(chunk) =
            response
                .chunk()
                .await
                .map_err(|source| TeamsClientError::Transport {
                    url: url.clone(),
                    source,
                })?
        {
            file.write_all(&chunk).await.map_err(|err| {
                TeamsClientError::Local(DomainError::Infra(anyhow::anyhow!(
                    "writing {} while cloning: {err}",
                    into.display()
                )))
            })?;
            written += chunk.len() as u64;
        }
        file.flush().await.map_err(|err| {
            TeamsClientError::Local(DomainError::Infra(anyhow::anyhow!(
                "finishing {} while cloning: {err}",
                into.display()
            )))
        })?;
        Ok(written)
    }

    /// Which of these team asset ids the team holds, and what each was
    /// converted from.
    ///
    /// The ids are the team's own — an id this client minted locally is
    /// not one of these, which is what the type says (#148 decision 6).
    pub async fn resolve_content(
        &self,
        team: TeamScopedId,
        asset_ids: &[TeamScopedId],
    ) -> Result<ResolvedContentDto, TeamsClientError> {
        self.post(
            &format!("/teams/{team}/forge/content/resolve"),
            "resolve_content",
            &ResolveContentCommand {
                asset_ids: asset_ids.iter().map(ToString::to_string).collect(),
            },
        )
        .await
    }

    /// What a promoter said about one entry, or nothing.
    ///
    /// Absent is an ordinary answer, not an error: an entry may have
    /// been named by a client that captured no description, and a
    /// projection may be lost without the line lying (#148 decision
    /// 12). The body comes back as the string it was written as — this
    /// client does not parse it either, and the mapper is the one place
    /// that does.
    pub async fn entry_projection(
        &self,
        team: TeamScopedId,
        line: TeamScopedId,
        entry: TeamScopedId,
    ) -> Result<Option<EntryProjectionDto>, TeamsClientError> {
        let path = format!("/teams/{team}/forge/lines/{line}/entries/{entry}/projection");
        match self
            .get::<EntryProjectionDto>(&path, "entry_projection")
            .await
        {
            Ok(found) => Ok(Some(found)),
            Err(TeamsClientError::Refused { status: 404, .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    // ------------------------------------------------------------------
    // The shared request shapes. Most calls above go through one of
    // these; `enter_content` and `fetch_content` build their own,
    // because bytes are neither JSON in nor JSON out.
    // ------------------------------------------------------------------

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        what: &'static str,
    ) -> Result<T, TeamsClientError> {
        let token = self.token(what)?.to_string();
        let url = self.url(path);
        let response = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|source| TeamsClientError::Transport {
                url: url.clone(),
                source,
            })?;
        read_json("GET", &url, response).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        what: &'static str,
        body: &B,
    ) -> Result<T, TeamsClientError> {
        let token = self.token(what)?.to_string();
        let url = self.url(path);
        let response = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .map_err(|source| TeamsClientError::Transport {
                url: url.clone(),
                source,
            })?;
        read_json("POST", &url, response).await
    }

    /// A verb that answers with nothing — the only one of these whose
    /// success has no body to decode.
    async fn delete(&self, path: &str, what: &'static str) -> Result<(), TeamsClientError> {
        let token = self.token(what)?.to_string();
        let url = self.url(path);
        let response = self
            .http
            .delete(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|source| TeamsClientError::Transport {
                url: url.clone(),
                source,
            })?;
        refusal("DELETE", &url, response).await
    }
}

/// Reads a body as text first, then decides.
///
/// Text first rather than `resp.json()` because a refusal and an answer
/// arrive on the same channel and only one of them is the shape the
/// route promises: reading the text keeps the server's own `message`
/// and `reason` available for the error, instead of reporting "expected
/// struct X" about a body that was telling the caller what went wrong.
async fn read_json<T: DeserializeOwned>(
    method: &'static str,
    url: &str,
    response: reqwest::Response,
) -> Result<T, TeamsClientError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|source| TeamsClientError::Transport {
            url: url.to_string(),
            source,
        })?;
    if !status.is_success() {
        return Err(refused(method, url, status.as_u16(), &body));
    }
    serde_json::from_str(&body).map_err(|err| TeamsClientError::Decode {
        url: url.to_string(),
        message: format!("{err}: {}", body.chars().take(200).collect::<String>()),
    })
}

/// The same for a route that answers with nothing.
async fn refusal(
    method: &'static str,
    url: &str,
    response: reqwest::Response,
) -> Result<(), TeamsClientError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(refused(method, url, status.as_u16(), &body))
}

/// Turns the house error body into the error type, keeping `reason`.
fn refused(method: &'static str, url: &str, status: u16, body: &str) -> TeamsClientError {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let field = |name: &str| -> Option<String> {
        parsed
            .as_ref()?
            .get(name)?
            .as_str()
            .map(std::string::ToString::to_string)
    };
    TeamsClientError::Refused {
        method,
        url: url.to_string(),
        status,
        kind: field("kind").unwrap_or_else(|| "Unknown".to_string()),
        message: field("message").unwrap_or_else(|| body.to_string()),
        reason: field("reason"),
    }
}
