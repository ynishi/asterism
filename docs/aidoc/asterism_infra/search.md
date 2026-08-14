# asterism-infra::search

Retrieval adapter — Tantivy on-disk index + Lindera Japanese
tokenizer + Porter English stemmer.

## Position

`AssetRetriever` / `AssetIndexer` (declared in
`asterism-core::domain::repository`) are the ports this implements.
The index is a rank-time projection: the durable truth is `asset_body` in
SQLite (`AssetBodyRepository`). Tantivy lives outside the SQLite
tx; index drift is recovered by re-running the `IndexRebuild`
job against `asset_body`.

## Storage

Index directory: `~/.asterism/tantivy/` (override via
`$ASTERISM_HOME`). Tantivy owns the layout inside (`meta.json`
plus segment files); `asterism-infra::paths::tantivy_index_dir`
only picks the parent.

## Tokenizer chain

Single tokenizer name `mixed_body` registered on the index:

```text
Lindera (IPADIC, Normal mode) → LowerCaser → AsciiFoldingFilter
  → Stemmer(Language::English) → RemoveLongFilter (40 char cap)
```

- Japanese text is segmented by lindera into morphemes.
- English words fall through as whitespace-separated tokens, are
  lowercased, accent-stripped (`café` → `cafe`), and Porter-stemmed
  (`typing` / `types` → `type`).
- Long tokens (URLs, base64, etc.) are dropped by the 40-char
  cap so the term dictionary stays compact.

## Query time

[`TantivyIndex::retrieve`] wraps each user query term as
`TermQuery OR FuzzyTermQuery(distance=1)` so single-char English
typos (`teh` → `the`) still hit even when the stemmer cannot
rewrite them. The whole query is combined with `SHOULD` under a
`BooleanQuery` so multi-word queries rank by BM25 sum.

