# asterism-core::domain::merge_plan

`MergePlan` — a person's ruling that a set of rows is one thing, and
which of them survives it.

The automatic half of duplicate resolution folds a **detected pair**:
two rows hold the same bytes, and one of them is folded into the
other. This is the other entry point. Somebody looked at several rows
and decided they are one thing — which needs no fingerprint to agree,
no queue row to have been raised, and is not bound by the exclusions
that stop an *automatic* fold (a person looking at two rows can
see what the rule was protecting).

# Why not part of `duplicate_conflict`

That module is the **question** a fingerprint match raises and the
answer somebody gives it — every value in it is keyed to a detected
pair. A plan reaches the merge verb without any of that: it may name
five rows, it may name rows no fingerprint ever compared, and there
may be no queue row anywhere. Putting it there would make the
module's own doc false, and would suggest to the next reader that a
merge is a conflict resolution with more members — which is exactly
the thing that is not true about it.

# What this type does *not* do

It does not look at the database. Whether a row exists, is already a
headstone, or has been thrown out is state, and state can change
between the moment a person clicks and the moment the transaction
runs — so it is re-read inside that transaction
([`AssetRepository::merge_into`](crate::domain::repository::AssetRepository::merge_into)),
never here. What is checkable without a database is whether the
*declaration* is a declaration at all, and that is the whole job of
this type.

## Types

- `MergePlan` — A checked declaration: these rows are one thing, and this is the one

