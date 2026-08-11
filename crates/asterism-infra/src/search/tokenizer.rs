//! Registers the `mixed_body` tokenizer on a Tantivy index.
//!
//! The chain is: `LinderaTokenizer` (IPADIC, Normal segmentation) →
//! `RemoveLongFilter(40)` → `LowerCaser` → `AsciiFoldingFilter` →
//! `Stemmer(English)`. See `search/mod.rs` for the rationale.
//!
//! Lindera does the heavy lifting on Japanese; the later filters are
//! no-ops on Japanese morphemes but do the work on English tokens
//! that fall through unchanged.

use anyhow::Result;
use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera_tantivy::tokenizer::LinderaTokenizer;
use tantivy::tokenizer::{
    AsciiFoldingFilter, Language, LowerCaser, RemoveLongFilter, Stemmer, TextAnalyzer,
    TokenizerManager,
};

/// Tokenizer name registered on the index. Referenced by
/// `TEXT.set_tokenizer(TOKENIZER_NAME)` in the schema builder.
pub const TOKENIZER_NAME: &str = "mixed_body";

/// Hard token length cap. Anything longer is dropped (URLs, base64
/// blobs, minified JS pasted into a message body).
const MAX_TOKEN_LEN: usize = 40;

/// Builds the `mixed_body` tokenizer (lindera + English chain +
/// long-token filter). Kept as a free function so both the index
/// builder and the query-time parser share exactly the same chain.
pub fn mixed_body_analyzer() -> Result<TextAnalyzer> {
    // Lindera IPADIC segmenter (Normal mode = single-best morpheme
    // path; Decompose mode splits compounds further but hurts search
    // recall on person-names / brand-names common in chat text).
    let dictionary = load_dictionary("embedded://ipadic")
        .map_err(|e| anyhow::anyhow!("lindera IPADIC load failed: {e}"))?;
    let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
    let lindera = LinderaTokenizer::from_segmenter(segmenter);

    let analyzer = TextAnalyzer::builder(lindera)
        .filter(RemoveLongFilter::limit(MAX_TOKEN_LEN))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .filter(Stemmer::new(Language::English))
        .build();
    Ok(analyzer)
}

/// Registers `mixed_body` on the given tokenizer manager. Idempotent
/// (register replaces).
pub fn register_on(manager: &TokenizerManager) -> Result<()> {
    let analyzer = mixed_body_analyzer()?;
    manager.register(TOKENIZER_NAME, analyzer);
    Ok(())
}
