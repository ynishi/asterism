# asterism-ui::stored_connection

What this machine remembers about a team server between windows
(#204).

# The invariant

**The disk never holds a primary credential.** Not the password,
not anything a verifier issued. The one credential this machine may
keep is a device token the server minted — expiring, listable and
revocable — and it lives in the OS keychain, never in a file.
Everything else worth remembering is not a credential at all, and
that half lives in the profile home as ordinary JSON.

Two stores, and the split is the rule rather than a convenience:
anything reaching [`write_metadata`] is by construction something a
reader of the file may see, and the only value that does not go
through it is the token, which has no path to a file from anywhere
in this module.

# Why the pair is keyed by server and login

One stored connection per server URL and login is the shape the
issue settles on, so [`account_key`] is the whole identity of an
entry: presenting a token minted for one account to another server
is not a case this module can reach, because the key that found the
token names both.

# Which connection a verb acts on

**A verb touches the stored state only when the stored metadata
names the connection in hand** — same server, same login, compared
by [`StoredConnection::names`]. The two stores answer different
questions and only one of them is about the window: the file says
what this machine was last told to remember, and the session says
what it is talking to now. They agree on the ordinary path and come
apart on a real one — a remembered server that was down when the
panel opened leaves its metadata in place, the connect form is
editable over the values it pre-filled, and the next thing signed
in to is somebody else.

Without the rule, a verb reading the file alone acts on the wrong
connection twice over: it sends one server's revocation handle to
another, where it is an idempotent `204` that deletes nothing, and
it drops the first server's entry while the row it names lives out
its ninety days. Signing out would then revoke nothing and forget
everything. So the file is a claim to be checked rather than an
instruction to be followed, and a verb that cannot match it does
the half it is sure of — end the session — and leaves the rest
alone.

[`remember_this_device`](crate::commands) and
[`disconnect_team_server`](crate::commands) are where this is
applied, and `revoke_team_device_token` is the third: it revokes on
the live client, which is the connection in hand by construction,
and consults the file only to decide whether the row it just
dropped was this machine's own.

# What a remember retires, and in what order

**The file is the sole index of the keychain.** Nothing here
enumerates entries, and nothing else in the app holds a list of
them, so an entry the file does not name is reachable from neither
store — the "credential nothing knows how to revoke" [`forget`]
exists to prevent, arrived at by a third door. That makes the order
of a remember's writes part of the invariant rather than a detail
of one function: the displaced entry goes first, the file second,
and the entry the file has just named last.

A crash inside that order leaves one of two states, and which one
turns on whether the pair being written is the pair being
displaced. A **new pair** lands in the state the next launch
already repairs — a file naming a pair whose entry is missing,
which `connect_team_server_stored` answers by dropping the metadata
and showing the password form.

**The same pair remembered twice is the exception**, because
[`retire_replaced`] returns early there by design: nothing takes
the entry away, so a run that dies between the file and the
keychain leaves the entry holding the *previous* token while the
file's `token_id` names the row just minted. Nothing is orphaned —
the entry is the one the file names — and what it leaves is a
handle that is ahead of the credential behind it. The next launch
reconnects on the old token and works; a disconnect after it
revokes the row the file names, which is the new one, and the old
row — whose token this machine was actually presenting — lives out
its fixed expiry with its handle written down nowhere. That is a
row in the owner's listing rather than a credential on this
machine, it is bounded by an expiry nobody has to act for, and the
next remember of that pair overwrites both halves. Cheaper than the
alternative, which is retiring an entry the write is about to
replace and losing a working credential to the same crash.

An entry nothing names cannot be created at all, because the
keychain is written only after the file says what is about to go
into it.

The order used to be the other one: the token reached the keychain
before anything reached the file, so that no metadata could name a
token nothing held. That was true of the pair being written and
silent about the pair being displaced, where the file went on
naming the previous connection and the next launch reconnected to
it — leaving the entry just written where nothing would ever look
again. The two costs are not alike: a missing entry is repaired on
the next launch by a person typing a password once more, and an
entry nothing names is repaired by nothing.

# The row behind a displaced pair

Retiring an entry takes away this machine's copy of a credential
and says nothing to the server that minted it, so the
`device_token` row stays live under the same [`device_label`] as
the one that replaced it. Two cases, and what separates them is
whether the session in hand can reach the row:

- **The same pair remembered again.** The live session is that
  server's, so the displaced handle — read out of the file before
  it is overwritten, which is what [`superseded_row`] answers — can
  be revoked over it. Best effort: a remember that stored the
  credential did what was asked, and a revoke that failed must not
  turn it into a refusal. What that costs is the old row left to
  its expiry, in a listing where it can still be picked out.
- **A different server remembered.** **Known limitation:** that row
  cannot be revoked from here. The handle names a row on a server
  this window is not talking to, and the session in hand belongs to
  the new one. The row lives out its fixed expiry, listed where it
  was issued and revocable from any window signed in there. The
  keychain entry is retired either way, so what is left behind is a
  row in somebody's listing rather than a credential on this
  machine.

A crashed remember leaves an unrevoked row of its own, too — the
mint lands and the writes do not. It ends up where the other two
do: on the server, in the owner's listing, which is the whole
reason that listing is a screen.

# Failing quietly on the way in, loudly on the way out

Every read degrades to "nothing is stored" **in what it puts in
front of a person**. A locked keychain, a denied prompt, a profile
home somebody moved and a file half a disk wrote all end at the
password form, because a reader who is told their keychain is
unavailable can do nothing the form does not already ask of them.

What a read may not degrade is the answer it gives this module. The
file is the sole index of the keychain, so a caller deciding
whether to drop the file needs the one distinction the person does
not: an entry that is gone says the file is stale, and a keychain
that would not answer says nothing at all. [`read_token`] therefore
answers a [`StoredToken`] rather than an `Option`. Folding those
two together is how one denied prompt at launch deletes the index
of an entry that is still sitting there — the state the section
above says a remember cannot create, reached by a deletion instead.

A write is the opposite. Somebody ticked a box asking to be
remembered, and a keychain that refuses has to say so: the
alternative is an app that silently forgets and asks again next
week, with no way to tell that from a revoked token.

## Functions

- `account_key` — The keychain account name for one server-and-login pair.
- `delete_metadata` — Removes the non-secret half. Best effort, for [`delete_token`]'s
- `delete_token` — Takes the device token out of the keychain.
- `device_label` — What this device would call itself when asking for a token.
- `forget` — Drops both halves for one server and login.
- `read_metadata` — What was stored about the last server this machine was told to
- `read_token` — The device token stored for this server and login.
- `retire_replaced` — Takes away the keychain entry of the pair a newly remembered one
- `superseded_row` — The handle of the `device_token` row a fresh mint for this pair
- `write_metadata` — Writes the non-secret half, replacing what was there.
- `write_token` — Puts a device token in the keychain, replacing whatever this

## Types

- `StoredConnection` — What this machine remembers about a team server, none of which
- `StoredToken` — What the keychain said when asked for one pair's token.

