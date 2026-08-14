# asterism-core::application::fold_redirect

Redirecting a named id set through the folds that happened after it
was written down.

[`Asset::folded_into`](crate::domain::asset::Asset::folded_into)
states the read rule in one sentence: **paths that enumerate a row
drop it, paths that name it keep it.** The enumerating half is a
`WHERE` term and lives in the adapter. This module is the naming
half, and it exists because reaching the headstone is only the first
part of the answer — what a caller does *after* reaching it is where
the surfaces disagreed.

# Why redirect rather than drop

Every caller here holds an id set it did not compute just now: a
Snapshot's frozen membership, the ids an export was created against,
the members of the freezes an asset once appeared in. Those sets are
content, not a query result.

- A **fold does not rewrite a Snapshot** (`a_fold_never_rewrites_a_snapshot`
  in the SQLite asset repository: "a content-addressed member set
  must not be edited by a fold"). So the id set still names the
  headstone afterwards, correctly.
- **Dropping** the headstone there loses a member. A four-member
  freeze becomes a three-member freeze because somebody merged two
  rows elsewhere, and the set's identity is gone.
- **Redirecting** collapses it onto the keeper, and onto a keeper the
  set already holds if it holds one — which is what stopped an export
  from receiving the same artefact twice.

Redirecting is also just what a fold means. "This row is now that
row" is the whole content of `folded_into`; a reader that stops at
the headstone has read half of it.

# Where this is called from, and where it is not

Called from every surface that hands
[`AssetRepository::cards_by_ids`](crate::domain::repository::AssetRepository::cards_by_ids)
an id set of its **own**: `snapshot_members`, `mint_snapshot`, the
dispatch runtime's input slice, and the constellation's
`same_selection` synthesis.

**Not** called from `hydrate_cards` (`POST /assets/hydrate`), and
that is a contract rather than an oversight: its caller is the grid,
whose ids come from a `list_index` read that already applied the
enumerating half of the rule. Redirecting them a second time would
be redundant work on the hottest read in the app.

`cards_by_ids` itself is untouched for the reason its own doc gives —
it is the twin of `find` by id, deliberately unfiltered, and the
trash view's hydration depends on that.

## Functions

- `hydrate_named` — [`redirect`] followed by the hydration the caller wanted — the one
- `redirect` — Replaces every headstone in `ids` with the row it was folded into,

## Types

- `NamedCards` — An id set redirected through its folds, and the cards behind it.

