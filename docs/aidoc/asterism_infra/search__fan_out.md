# asterism-infra::search::fan_out

One [`AssetIndexer`] over several.

An asset's body feeds two indexes with different jobs: the SQL
`asset_fts` trigram index answers the Query-side `text_match`
predicate (an exact set), and the Tantivy index answers Retrieval
(a ranked shortlist). They are separate because the questions are
separate — but they go stale together, so keeping them in step is
one concern, not two.

Fanning out here rather than at each call site means every path
that already maintains the index (rebuild / trash / fold / purge)
maintains both, with no chance of a new path remembering one and
forgetting the other.

# Failure is not partial-silent

Every member runs even when an earlier one fails, and the **first**
error is returned. Stopping at the first failure would leave the
remaining indexes untouched with no signal about which ones, which
is a worse state to recover from than "all attempted, one reported".
The recovery path is the same either way: re-run `IndexRebuild`.

## Types

- `FanOutIndexer` — Fans one indexer call out to every member, in order.

