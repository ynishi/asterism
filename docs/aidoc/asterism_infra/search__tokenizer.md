# asterism-infra::search::tokenizer

Registers the `mixed_body` tokenizer on a Tantivy index.

The chain is: `LinderaTokenizer` (IPADIC, Normal segmentation) →
`RemoveLongFilter(40)` → `LowerCaser` → `AsciiFoldingFilter` →
`Stemmer(English)`. See `search/mod.rs` for the rationale.

Lindera does the heavy lifting on Japanese; the later filters are
no-ops on Japanese morphemes but do the work on English tokens
that fall through unchanged.

## Functions

- `mixed_body_analyzer` — Builds the `mixed_body` tokenizer (lindera + English chain +
- `register_on` — Registers `mixed_body` on the given tokenizer manager. Idempotent

## Constants

- `TOKENIZER_NAME` — Tokenizer name registered on the index. Referenced by

