//! What this machine remembers about a team server between windows
//! (#204).
//!
//! # The invariant
//!
//! **The disk never holds a primary credential.** Not the password,
//! not anything a verifier issued. The one credential this machine may
//! keep is a device token the server minted — expiring, listable and
//! revocable — and it lives in the OS keychain, never in a file.
//! Everything else worth remembering is not a credential at all, and
//! that half lives in the profile home as ordinary JSON.
//!
//! Two stores, and the split is the rule rather than a convenience:
//! anything reaching [`write_metadata`] is by construction something a
//! reader of the file may see, and the only value that does not go
//! through it is the token, which has no path to a file from anywhere
//! in this module.
//!
//! # Why the pair is keyed by server and login
//!
//! One stored connection per server URL and login is the shape the
//! issue settles on, so [`account_key`] is the whole identity of an
//! entry: presenting a token minted for one account to another server
//! is not a case this module can reach, because the key that found the
//! token names both.
//!
//! # Which connection a verb acts on
//!
//! **A verb touches the stored state only when the stored metadata
//! names the connection in hand** — same server, same login, compared
//! by [`StoredConnection::names`]. The two stores answer different
//! questions and only one of them is about the window: the file says
//! what this machine was last told to remember, and the session says
//! what it is talking to now. They agree on the ordinary path and come
//! apart on a real one — a remembered server that was down when the
//! panel opened leaves its metadata in place, the connect form is
//! editable over the values it pre-filled, and the next thing signed
//! in to is somebody else.
//!
//! Without the rule, a verb reading the file alone acts on the wrong
//! connection twice over: it sends one server's revocation handle to
//! another, where it is an idempotent `204` that deletes nothing, and
//! it drops the first server's entry while the row it names lives out
//! its ninety days. Signing out would then revoke nothing and forget
//! everything. So the file is a claim to be checked rather than an
//! instruction to be followed, and a verb that cannot match it does
//! the half it is sure of — end the session — and leaves the rest
//! alone.
//!
//! [`remember_this_device`](crate::commands) and
//! [`disconnect_team_server`](crate::commands) are where this is
//! applied, and `revoke_team_device_token` is the third: it revokes on
//! the live client, which is the connection in hand by construction,
//! and consults the file only to decide whether the row it just
//! dropped was this machine's own.
//!
//! # What a remember retires, and in what order
//!
//! **The file is the sole index of the keychain.** Nothing here
//! enumerates entries, and nothing else in the app holds a list of
//! them, so an entry the file does not name is reachable from neither
//! store — the "credential nothing knows how to revoke" [`forget`]
//! exists to prevent, arrived at by a third door. That makes the order
//! of a remember's writes part of the invariant rather than a detail
//! of one function: the displaced entry goes first, the file second,
//! and the entry the file has just named last.
//!
//! A crash inside that order leaves one of two states, and which one
//! turns on whether the pair being written is the pair being
//! displaced. A **new pair** lands in the state the next launch
//! already repairs — a file naming a pair whose entry is missing,
//! which `connect_team_server_stored` answers by dropping the metadata
//! and showing the password form.
//!
//! **The same pair remembered twice is the exception**, because
//! [`retire_replaced`] returns early there by design: nothing takes
//! the entry away, so a run that dies between the file and the
//! keychain leaves the entry holding the *previous* token while the
//! file's `token_id` names the row just minted. Nothing is orphaned —
//! the entry is the one the file names — and what it leaves is a
//! handle that is ahead of the credential behind it. The next launch
//! reconnects on the old token and works; a disconnect after it
//! revokes the row the file names, which is the new one, and the old
//! row — whose token this machine was actually presenting — lives out
//! its fixed expiry with its handle written down nowhere. That is a
//! row in the owner's listing rather than a credential on this
//! machine, it is bounded by an expiry nobody has to act for, and the
//! next remember of that pair overwrites both halves. Cheaper than the
//! alternative, which is retiring an entry the write is about to
//! replace and losing a working credential to the same crash.
//!
//! An entry nothing names cannot be created at all, because the
//! keychain is written only after the file says what is about to go
//! into it.
//!
//! The order used to be the other one: the token reached the keychain
//! before anything reached the file, so that no metadata could name a
//! token nothing held. That was true of the pair being written and
//! silent about the pair being displaced, where the file went on
//! naming the previous connection and the next launch reconnected to
//! it — leaving the entry just written where nothing would ever look
//! again. The two costs are not alike: a missing entry is repaired on
//! the next launch by a person typing a password once more, and an
//! entry nothing names is repaired by nothing.
//!
//! # The row behind a displaced pair
//!
//! Retiring an entry takes away this machine's copy of a credential
//! and says nothing to the server that minted it, so the
//! `device_token` row stays live under the same [`device_label`] as
//! the one that replaced it. Two cases, and what separates them is
//! whether the session in hand can reach the row:
//!
//! - **The same pair remembered again.** The live session is that
//!   server's, so the displaced handle — read out of the file before
//!   it is overwritten, which is what [`superseded_row`] answers — can
//!   be revoked over it. Best effort: a remember that stored the
//!   credential did what was asked, and a revoke that failed must not
//!   turn it into a refusal. What that costs is the old row left to
//!   its expiry, in a listing where it can still be picked out.
//! - **A different server remembered.** **Known limitation:** that row
//!   cannot be revoked from here. The handle names a row on a server
//!   this window is not talking to, and the session in hand belongs to
//!   the new one. The row lives out its fixed expiry, listed where it
//!   was issued and revocable from any window signed in there. The
//!   keychain entry is retired either way, so what is left behind is a
//!   row in somebody's listing rather than a credential on this
//!   machine.
//!
//! A crashed remember leaves an unrevoked row of its own, too — the
//! mint lands and the writes do not. It ends up where the other two
//! do: on the server, in the owner's listing, which is the whole
//! reason that listing is a screen.
//!
//! # Failing quietly on the way in, loudly on the way out
//!
//! Every read degrades to "nothing is stored" **in what it puts in
//! front of a person**. A locked keychain, a denied prompt, a profile
//! home somebody moved and a file half a disk wrote all end at the
//! password form, because a reader who is told their keychain is
//! unavailable can do nothing the form does not already ask of them.
//!
//! What a read may not degrade is the answer it gives this module. The
//! file is the sole index of the keychain, so a caller deciding
//! whether to drop the file needs the one distinction the person does
//! not: an entry that is gone says the file is stale, and a keychain
//! that would not answer says nothing at all. [`read_token`] therefore
//! answers a [`StoredToken`] rather than an `Option`. Folding those
//! two together is how one denied prompt at launch deletes the index
//! of an entry that is still sitting there — the state the section
//! above says a remember cannot create, reached by a deletion instead.
//!
//! A write is the opposite. Somebody ticked a box asking to be
//! remembered, and a keychain that refuses has to say so: the
//! alternative is an app that silently forgets and asks again next
//! week, with no way to tell that from a revoked token.

use std::path::PathBuf;

use asterism_core::DomainError;
use serde::{Deserialize, Serialize};

use crate::error::UiError;

/// The keychain service every entry this app writes sits under — the
/// bundle identifier, so what a person finds in Keychain Access names
/// the app that put it there.
const SERVICE: &str = "dev.ynishi.asterism";

/// The file under the profile home holding the non-secret half.
const METADATA_FILE: &str = "teams-connection.json";

/// What this machine remembers about a team server, none of which
/// authenticates anybody.
///
/// Every field here is safe in a file, and the module header says why
/// that is a property of the type rather than of its current fields:
/// the token has no path into this struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredConnection {
    /// The server the token was minted by, as it was typed.
    pub base_url: String,
    /// The account it was minted for.
    pub login: String,
    /// The handle the mint answered with, which
    /// `DELETE /teams/auth/device/{id}` takes. Not a secret — it is the
    /// revocation handle, and `DeviceTokenMintedDto::id` says so where
    /// the wire defines it.
    pub token_id: String,
    /// What this device asked to be called in the owner's listing.
    pub label: String,
}

impl StoredConnection {
    /// Whether what was stored names this server and login — the
    /// module header's "which connection a verb acts on", as the one
    /// question every verb asks before touching either store.
    ///
    /// Compared through [`account_key`] rather than field by field, so
    /// the two ways of typing one server answer the same here as they
    /// do in the keychain. A comparison that disagreed with the key
    /// would be the worse defect of the two: a verb would decide the
    /// pair matched and then reach an entry it does not name.
    pub fn names(&self, base_url: &str, login: &str) -> bool {
        account_key(&self.base_url, &self.login) == account_key(base_url, login)
    }
}

/// The keychain account name for one server-and-login pair.
///
/// Written to be read: macOS shows this string in Keychain Access, so
/// a person auditing what an app has stored sees which account on
/// which server rather than a hash.
///
/// Trailing slashes come off so that a server typed with one and a
/// server typed without reach the same entry. Nothing else is
/// normalised — a login's case is the server's business, and folding
/// it here would be this side deciding two accounts are one.
pub fn account_key(base_url: &str, login: &str) -> String {
    format!("{login} @ {}", base_url.trim_end_matches('/'))
}

/// What this device would call itself when asking for a token.
///
/// It names the app and the platform, not the machine: reaching a
/// hostname from a bundled desktop app costs a dependency, and this
/// string's job is to let somebody reading their own listing decide
/// what to revoke. Two machines of the same kind therefore carry the
/// same label, and the mint time beside them in the listing is what
/// tells those two rows apart.
pub fn device_label() -> String {
    format!("Asterism on {}", std::env::consts::OS)
}

/// What the keychain said when asked for one pair's token.
///
/// Three answers rather than two, and the third is the module header's
/// "what a read may not degrade": both of the ways there is no token
/// in hand end at the password form, but only one of them is news
/// about the file. A caller that treats them alike drops the index of
/// an entry that is still there the first time somebody dismisses a
/// keychain prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum StoredToken {
    /// The token this pair's entry holds.
    Held(String),
    /// There is no entry for this pair — never written, or removed in
    /// Keychain Access since. The keychain answered, and its answer is
    /// that the file names a connection this machine cannot make.
    NoEntry,
    /// The keychain would not say. A locked store, a prompt somebody
    /// dismissed, a session with no credential store configured, a
    /// platform failure underneath — the entry may be sitting there
    /// intact, and nothing here has learnt otherwise.
    Unavailable,
}

/// The device token stored for this server and login.
///
/// The keychain call runs on a blocking thread because it is one: the
/// first read of an entry can put a prompt in front of the person and
/// wait for them, and an async runtime thread is not somewhere to do
/// that. A thread that does not come back at all is [`StoredToken::Unavailable`]
/// for the same reason a locked keychain is — it is a fact about this
/// read rather than about the entry.
pub async fn read_token(base_url: &str, login: &str) -> StoredToken {
    let account = account_key(base_url, login);
    match tokio::task::spawn_blocking(move || {
        keyring::Entry::new(SERVICE, &account)?.get_password()
    })
    .await
    {
        Ok(answered) => keychain_answer(answered),
        Err(_) => StoredToken::Unavailable,
    }
}

/// Which of the three a keychain answer is.
///
/// Split out from [`read_token`] because it is the half that can be
/// tested: the reading of `keyring`'s error is where the distinction
/// this module depends on is either made or lost, and no test can put
/// a locked keychain in front of the other half.
///
/// Everything that is not [`keyring::Error::NoEntry`] is
/// [`StoredToken::Unavailable`], which is also the safe way to read an
/// error type that is `#[non_exhaustive]`: a variant added upstream is
/// a keychain that did not answer until somebody here decides
/// otherwise, and that way round costs a password rather than an
/// index.
fn keychain_answer(answered: Result<String, keyring::Error>) -> StoredToken {
    match answered {
        Ok(token) => StoredToken::Held(token),
        Err(keyring::Error::NoEntry) => StoredToken::NoEntry,
        Err(_) => StoredToken::Unavailable,
    }
}

/// Puts a device token in the keychain, replacing whatever this
/// server and login had there.
///
/// Refusals reach the caller, for the reason the module header gives:
/// this is somebody's request to be remembered, and one that did not
/// happen has to be visible.
pub async fn write_token(base_url: &str, login: &str, token: &str) -> Result<(), UiError> {
    let account = account_key(base_url, login);
    let token = token.to_string();
    tokio::task::spawn_blocking(move || {
        keyring::Entry::new(SERVICE, &account)?.set_password(&token)
    })
    .await
    .map_err(|err| {
        UiError::from(DomainError::Infra(anyhow::anyhow!(
            "the keychain write did not finish: {err}"
        )))
    })?
    .map_err(|err| {
        UiError::from(DomainError::Infra(anyhow::anyhow!(
            "this machine's keychain would not hold the device token: {err}"
        )))
    })
}

/// Takes the device token out of the keychain.
///
/// Says nothing about whether there was one. Forgetting is reached
/// from a logout and from a token the server rejected, and neither has
/// anything to do differently on learning the entry was already gone.
pub async fn delete_token(base_url: &str, login: &str) {
    let account = account_key(base_url, login);
    let _ = tokio::task::spawn_blocking(move || {
        keyring::Entry::new(SERVICE, &account).map(|entry| entry.delete_credential())
    })
    .await;
}

/// Where the non-secret half lives.
fn metadata_path() -> Result<PathBuf, UiError> {
    Ok(asterism_infra::paths::asterism_home()?.join(METADATA_FILE))
}

/// What was stored about the last server this machine was told to
/// remember, or `None`.
///
/// `None` covers an absent file, an unreadable one and one holding
/// something this type does not parse — a shape that changed under a
/// file nobody migrates is a connection to forget, not a startup to
/// fail.
pub async fn read_metadata() -> Option<StoredConnection> {
    let path = metadata_path().ok()?;
    let raw = tokio::fs::read(&path).await.ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Writes the non-secret half, replacing what was there.
///
/// One connection, which is the v1 shape: a second server remembered
/// is this file rewritten, and the entry the previous one held has
/// been taken away by [`retire_replaced`] before this lands — the file
/// indexes the keychain, so it never names one pair while another's
/// entry sits outside it.
pub async fn write_metadata(stored: &StoredConnection) -> Result<(), UiError> {
    let path = metadata_path()?;
    let body = serde_json::to_vec_pretty(stored).map_err(|err| {
        UiError::from(DomainError::Infra(anyhow::anyhow!(
            "could not write down the connection: {err}"
        )))
    })?;
    tokio::fs::write(&path, body).await.map_err(|err| {
        UiError::from(DomainError::Infra(anyhow::anyhow!(
            "could not write {}: {err}",
            path.display()
        )))
    })
}

/// Takes away the keychain entry of the pair a newly remembered one
/// has just displaced, if it displaced any.
///
/// The file holds one connection and the keychain holds an entry per
/// pair, so remembering a second server — or the same server under a
/// second login, which [`account_key`] does not fold — retires a
/// metadata row and would leave an entry behind it. That entry is the
/// "credential nothing knows how to revoke" [`forget`] exists to
/// prevent, arrived at by the other door.
///
/// Runs **before** the new metadata lands, which is the module
/// header's "what a remember retires, and in what order": the file is
/// the index of the keychain, so no entry may outlive the file's
/// mention of it. A run that dies between this and the write leaves
/// metadata describing a token nothing holds, and the next launch
/// drops that metadata and asks for a password — the cost the header
/// accepts, and the credential thrown away is the one the person has
/// just asked to replace.
///
/// Takes `previous` as an argument rather than reading the file,
/// because the caller has to read it before overwriting it in any
/// case, and by reference because it is needed again afterwards:
/// [`superseded_row`] is the other half of a displacement, and what it
/// answers cannot be recovered once the file is rewritten.
///
/// Returns having done nothing when the displaced pair is the pair
/// being written. That entry is not orphaned by the write, it is
/// overwritten by it, and deleting it here would take away the entry
/// the file is about to name. The server row that displacement leaves
/// behind is [`superseded_row`]'s.
///
/// Best effort, for [`delete_token`]'s reason.
pub async fn retire_replaced(previous: Option<&StoredConnection>, base_url: &str, login: &str) {
    let Some(previous) = previous else {
        return;
    };
    if previous.names(base_url, login) {
        return;
    }
    delete_token(&previous.base_url, &previous.login).await;
}

/// The handle of the `device_token` row a fresh mint for this pair
/// displaces on the server, or `None` when this displacement leaves no
/// row anything here can revoke.
///
/// The other half of [`retire_replaced`], and the two are exclusive by
/// construction — which is the point of reading them together. A
/// displaced pair is either the pair being written, where the keychain
/// entry is overwritten and the row is what is left over, or a
/// different one, where the entry is retired above and the row sits on
/// a server the session in hand is not talking to. So this answers
/// `Some` in exactly the case `retire_replaced` returns early in, and
/// the module header's "the row behind a displaced pair" says what the
/// caller does with each.
///
/// Answered here rather than at the call site because it is the same
/// question [`StoredConnection::names`] asks everywhere else, and a
/// second comparison written by hand is how the two come to disagree.
pub fn superseded_row<'a>(
    previous: Option<&'a StoredConnection>,
    base_url: &str,
    login: &str,
) -> Option<&'a str> {
    previous
        .filter(|previous| previous.names(base_url, login))
        .map(|previous| previous.token_id.as_str())
}

/// Removes the non-secret half. Best effort, for [`delete_token`]'s
/// reason.
pub async fn delete_metadata() {
    if let Ok(path) = metadata_path() {
        let _ = tokio::fs::remove_file(path).await;
    }
}

/// Drops both halves for one server and login.
///
/// The pair is what a stored connection is, so the two are forgotten
/// together — a keychain entry outliving its metadata is a credential
/// nothing knows how to revoke, and metadata outliving its entry is a
/// form that pre-fills into a reconnect that cannot work.
pub async fn forget(base_url: &str, login: &str) {
    delete_token(base_url, login).await;
    delete_metadata().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key is the whole identity of an entry, so the two ways of
    /// typing one server have to reach the same one, and two accounts
    /// must not.
    #[test]
    fn one_server_typed_two_ways_is_one_entry() {
        assert_eq!(
            account_key("http://127.0.0.1:8787/", "alice"),
            account_key("http://127.0.0.1:8787", "alice"),
        );
        assert_ne!(
            account_key("http://127.0.0.1:8787", "alice"),
            account_key("http://127.0.0.1:8787", "bob"),
        );
        assert_ne!(
            account_key("http://127.0.0.1:8787", "alice"),
            account_key("http://example.test", "alice"),
        );
    }

    /// A login's case is the server's to decide, and folding it here
    /// would be this side answering that question for it.
    #[test]
    fn a_login_keeps_its_case() {
        assert_ne!(
            account_key("http://127.0.0.1:8787", "Alice"),
            account_key("http://127.0.0.1:8787", "alice"),
        );
    }

    /// What goes in the file is what comes out of it, and the fields
    /// are the ones the connect form and the listing read.
    #[test]
    fn the_file_survives_a_round_trip() {
        let stored = StoredConnection {
            base_url: "http://127.0.0.1:8787".into(),
            login: "alice".into(),
            token_id: "dt_01234".into(),
            label: "Asterism on macos".into(),
        };
        let json = serde_json::to_string(&stored).expect("a struct of strings");
        let back: StoredConnection = serde_json::from_str(&json).expect("what was just written");
        assert_eq!(stored, back);
    }

    /// The invariant this module exists for, asked of the bytes rather
    /// than of the type: whatever the shape grows, nothing that
    /// authenticates anybody may reach the file.
    ///
    /// A token is not a field here, so this cannot fail today. It is
    /// written for the change that adds one — the id and the token are
    /// a handle and a secret that arrive in the same response, and
    /// putting the wrong one in the file is the mistake with no other
    /// guard on it.
    #[test]
    fn nothing_authenticating_reaches_the_file() {
        let stored = StoredConnection {
            base_url: "http://127.0.0.1:8787".into(),
            login: "alice".into(),
            token_id: "dt_01234".into(),
            label: "Asterism on macos".into(),
        };
        let json = serde_json::to_value(&stored).expect("a struct of strings");
        let keys: Vec<&str> = json
            .as_object()
            .expect("a struct serialises as an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["base_url", "login", "token_id", "label"]);
        for forbidden in ["token", "password", "secret"] {
            assert!(
                !keys.contains(&forbidden),
                "`{forbidden}` reached the file this module promises holds no credential",
            );
        }
    }

    /// A stored connection for one pair, asked about another.
    ///
    /// This is the question every verb asks before touching either
    /// store, and the three answers it has to get right are the three
    /// ways the file and the session come apart: a second server, a
    /// second login on the one server, and the same pair typed with a
    /// slash it did not have last time.
    #[test]
    fn a_stored_pair_names_itself_and_nothing_else() {
        let stored = StoredConnection {
            base_url: "http://127.0.0.1:8787".into(),
            login: "alice".into(),
            token_id: "dt_01234".into(),
            label: "Asterism on macos".into(),
        };
        assert!(stored.names("http://127.0.0.1:8787", "alice"));
        assert!(
            stored.names("http://127.0.0.1:8787/", "alice"),
            "the trailing slash the keychain key folds has to fold here too, \
             or a verb decides a pair matches and then reaches an entry it \
             does not name",
        );
        assert!(!stored.names("http://example.test", "alice"));
        assert!(!stored.names("http://127.0.0.1:8787", "bob"));
        assert!(!stored.names("http://127.0.0.1:8787", "Alice"));
    }

    /// Which server row a remember supersedes, which is the half of a
    /// displacement the keychain does not answer for.
    ///
    /// The same pair remembered again overwrites its entry and leaves
    /// its row, so the row is what there is to revoke; a different pair
    /// leaves a row on a server the session in hand cannot reach, and
    /// saying `None` here is what keeps a handle from being sent to a
    /// server that never issued it — where it is an idempotent `204`
    /// that deletes nothing.
    #[test]
    fn a_re_mint_supersedes_the_row_it_replaces_and_no_other() {
        let stored = StoredConnection {
            base_url: "http://127.0.0.1:8787".into(),
            login: "alice".into(),
            token_id: "dt_01234".into(),
            label: "Asterism on macos".into(),
        };
        assert_eq!(
            superseded_row(Some(&stored), "http://127.0.0.1:8787", "alice"),
            Some("dt_01234"),
        );
        assert_eq!(
            superseded_row(Some(&stored), "http://127.0.0.1:8787/", "alice"),
            Some("dt_01234"),
            "the trailing slash folds here as it does in the key, or one \
             way of typing a server re-mints without retiring the row the \
             other way left",
        );
        assert_eq!(
            superseded_row(Some(&stored), "http://example.test", "alice"),
            None,
        );
        assert_eq!(
            superseded_row(Some(&stored), "http://127.0.0.1:8787", "bob"),
            None,
        );
        assert_eq!(
            superseded_row(None, "http://127.0.0.1:8787", "alice"),
            None,
            "a first remember displaces nothing",
        );
    }

    /// The distinction the file's life depends on: an entry that is
    /// gone against a keychain that would not answer.
    ///
    /// `connect_team_server_stored` drops the metadata on the first
    /// and keeps it on the second, so a mapping that folded them would
    /// delete the sole index of a live entry the first time somebody
    /// dismissed a keychain prompt — the one state the module header
    /// says nothing can create.
    #[test]
    fn an_absent_entry_is_not_an_unreadable_keychain() {
        assert_eq!(
            keychain_answer(Ok("dt_secret".into())),
            StoredToken::Held("dt_secret".into()),
        );
        assert_eq!(
            keychain_answer(Err(keyring::Error::NoEntry)),
            StoredToken::NoEntry,
        );
        assert_eq!(
            keychain_answer(Err(keyring::Error::NoStorageAccess(
                "the keychain is locked".into()
            ))),
            StoredToken::Unavailable,
            "a locked keychain is not news that the connection is gone",
        );
        assert_eq!(
            keychain_answer(Err(keyring::Error::NoDefaultStore)),
            StoredToken::Unavailable,
            "a session with no credential store has not looked at the entry",
        );
        assert_eq!(
            keychain_answer(Err(keyring::Error::PlatformFailure(
                "the store fell over".into()
            ))),
            StoredToken::Unavailable,
        );
    }

    /// The label is what a person picks a row to revoke by, so it has
    /// to say something rather than be blank.
    #[test]
    fn the_label_names_something() {
        let label = device_label();
        assert!(label.starts_with("Asterism on "), "{label}");
        assert!(label.len() > "Asterism on ".len(), "{label}");
    }
}
