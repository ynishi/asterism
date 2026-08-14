# asterism-core::domain::attribution

Attribution — *who* a record is by, *what* operated on their behalf,
and *through which channel* that answer arrived.

This module is the source of truth for the attribution doctrine. One
triple travels together: `(author, operator, via)`. It is carried on
the write path by [`AttributionContext`] and restored from the stored
columns by [`PersistedAttribution`]; the individual values are
[`Author`], [`OperatorRef`] and [`AttributionChannel`].

- [`Author`] — the subject a record is attributed to. The write-side
  mirror of [`Viewer`](crate::domain::value::Viewer): the reader asks
  "who is looking", this answers "who was writing", and both name a
  subject with the same token.
- [`OperatorRef`] — the agent that performed the operation
  (`claude-code`, `codex`, `asterism-ui`, …). Under a single
  authenticated subject the interesting question is often not *who*
  but *through what*, and one field cannot hold both answers.
- [`AttributionChannel`] — the channel the pair above arrived
  through. Derived from the entry point, never asserted.

# Five roles, of which two are attribution

Several types in this codebase name somebody. Only the first two
answer "whose write is this":

| concept | role | attribution |
|---|---|---|
| **author** ([`Author`]) | whose write this is — a person or a subject on a service | first axis |
| **operator** ([`OperatorRef`]) | the agent that carried the operation out | second axis |
| **register** (persona) | a choice of voice at presentation time — [`CommentAuthor::Persona`](crate::domain::asset_comment::CommentAuthor::Persona), [`thread::Author::Persona`](crate::domain::thread::Author::Persona). Same sense as [`Asset::register_note`](crate::domain::asset::Asset::register_note) ("the asset's register / tone"), not the *registry* sense the word carries elsewhere in this codebase | out of scope — speaking in a voice is not authorship |
| **the persona an asset belongs to** ([`Asset::persona_id`](crate::domain::asset::Asset::persona_id)) | every asset belongs to exactly one persona. Membership says nothing about who wrote it | out of scope — the persona shown on a card is where the asset is filed, not who made it |
| **transcript role** (`ChatRole` in `asterism-importer-sdk`) | a role written *inside* imported conversation material (`user` / `assistant` / …). A fact about the content | out of scope — the `user` of an imported chat is not necessarily this instance's owner |

This is why [`Author::parse`] refuses a `"persona"` kind: a persona
is a voice something can be said in, and a place an asset belongs to.
Neither is a subject a write is attributed to.

The mapping for [`thread::Author`](crate::domain::thread::Author),
which folds "human vs agent" into one enum, is: `Human` ≈
`(author = Owner, operator = none)`, `ClaudeCode` / `Agent(s)` ≈
`(author = unrecorded, operator = s)`, `Persona` ≈ register.
[`CommentAuthor::User`](crate::domain::asset_comment::CommentAuthor::User)
is the comment-side alias of [`Author::Owner`]. Unifying the types
is deliberately not attempted here — the doctrine (which value means
what) comes first.

# Who the owner is

[`Author::Owner`] is an indirect reference to the single
[`InstanceIdentity`](crate::domain::instance::InstanceIdentity) row:
one profile database has exactly one owner. Today
`instance.owner_subject` is unbound, so `Owner` reads as "whoever
this instance belongs to" and resolves to no token
([`InstanceIdentity::resolve_owner`](crate::domain::instance::InstanceIdentity::resolve_owner)).
Authentication binds the subject once, and only then does `Owner`
resolve to a name. Sharing adds subjects; it never adds owners.

Author subjects and viewer (sharing) subjects are **one namespace**.
"shared with alice" and "written by alice" must be the same alice,
or a hosted deployment cannot reconcile who may look with who wrote.

# `None` means unrecorded

An absent author is **not** "authored by the owner". Defaulting to
the owner would make the assertion and the default indistinguishable
the moment a second subject exists, which is the state a hosted
migration would have to un-guess. The absence is the same kind of
absence as [`content_hash`](crate::domain::content_hash) `NULL`: a
question nobody has answered yet, not a value. Splitting "operated
by a human directly" out of "unrecorded" is not modelled today; it
can land later as a reserved marker (the shape
[`UNHASHABLE`](crate::domain::content_hash::UNHASHABLE) uses)
without disturbing the columns.

# The same shape as `_trace.source`, on a different subject

[`provenance::source`](crate::domain::provenance::source) records
which channel a **provenance claim** arrived through (embedded /
pushed / manual). [`AttributionChannel`] records which channel an
**attribution** arrived through. Both are bookkeeping of arrival,
derived from the entry point rather than asserted; they are not two
answers to one question, they are the same question asked about two
different values, and one request can legitimately write both (a
provenance verb stamps `source = manual` on the claim and
`via = asserted` on the operator it records).

## Types

- `AttributionChannel` — The channel an attribution arrived through.
- `AttributionContext` — The attribution a write carries — request-scoped, chosen by the
- `Author` — The subject a record is attributed to.
- `OperatorRef` — The agent that performed an operation — `claude-code`, `codex`,
- `PersistedAttribution` — An attribution triple restored from the stored columns.

