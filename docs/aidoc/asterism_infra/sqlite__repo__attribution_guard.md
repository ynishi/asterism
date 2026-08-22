# asterism-infra::sqlite::repo::attribution_guard

Write-side guard for the attribution columns.

`Author::from_columns` is the read-side check SQLite cannot express
as a CHECK on an `ALTER TABLE` column. This is its write-side twin
for the rule the channel column adds: **a row that records an author
or an operator records the channel that answer arrived through.**

Without it, "author set, channel NULL" would keep being produced by
new writes, and that shape already means something else — it is how
a V47 / V48 row looks, which is precisely the set of rows an
authenticated deployment cannot resolve. A new row landing in the
legacy bucket would be indistinguishable from one that predates the
column.

Called by the row builders in [`super::asset`] and
[`super::dispatch`], at the point where the values about to be
bound are visible as the columns themselves.

[`attribution_columns`] lives here for the same reason: it is the
encoding half of the same concern, wanted by every table that
carries the triple, and a home in any one adapter would make the
next one reach sideways into a sibling.

## Functions

- `assert_channel_recorded` — Rejects a row that records somebody without recording how that
- `attribution_columns` — Encodes an entity's attribution triple into the column values,

## Types

- `AttributionColumns` — The four attribution column values in write order:

