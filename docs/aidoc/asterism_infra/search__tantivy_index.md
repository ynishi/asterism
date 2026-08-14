# asterism-infra::search::tantivy_index

[`AssetRetriever`] + [`AssetIndexer`] adapter backed by an on-disk
Tantivy index.

## Schema

Three fields:
- `asset_id` — STRING (whole term, stored). Uuid v7 hyphenated;
  used as the doc term for `remove` and returned in [`Candidate`].
- `persona_id` — STRING (whole term, stored). Denormalised so a
  persona-scope filter can be pushed into the query with a
  `TermQuery` (no SQL round-trip per hit).
- `body` — TEXT with the `mixed_body` tokenizer, stored so the
  `SnippetGenerator` can render a highlight window without
  re-fetching the SQLite row.

## Concurrency

One `IndexWriter` is held behind a `Mutex` (write serialisation
is what tantivy expects; `IndexWriter::add_document` is `&mut
self`). Readers are lock-free — a fresh `IndexReader::searcher`
is opened per query, which is O(µs) after the first commit
[estimated, tantivy 0.26].

Writer heap: 50 MB (tantivy default; smaller heaps flush more
often and hurt bulk backfill throughput).

## Types

- `TantivyIndex` — Tantivy-backed [`AssetRetriever`] + [`AssetIndexer`] implementation.

