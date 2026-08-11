//! [`AssetRetriever`] + [`AssetIndexer`] adapter backed by an on-disk
//! Tantivy index.
//!
//! ## Schema
//!
//! Three fields:
//! - `asset_id` — STRING (whole term, stored). Uuid v7 hyphenated;
//!   used as the doc term for `remove` and returned in [`Candidate`].
//! - `persona_id` — STRING (whole term, stored). Denormalised so a
//!   persona-scope filter can be pushed into the query with a
//!   `TermQuery` (no SQL round-trip per hit).
//! - `body` — TEXT with the `mixed_body` tokenizer, stored so the
//!   `SnippetGenerator` can render a highlight window without
//!   re-fetching the SQLite row.
//!
//! ## Concurrency
//!
//! One `IndexWriter` is held behind a `Mutex` (write serialisation
//! is what tantivy expects; `IndexWriter::add_document` is `&mut
//! self`). Readers are lock-free — a fresh `IndexReader::searcher`
//! is opened per query, which is O(µs) after the first commit
//! [estimated, tantivy 0.26].
//!
//! Writer heap: 50 MB (tantivy default; smaller heaps flush more
//! often and hurt bulk backfill throughput).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use asterism_core::domain::repository::{
    AssetIndexer, AssetRetriever, Candidate, Evidence, IndexDoc, RetrievalIntent, RetrievalQuery,
    Retrieved,
};
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, FuzzyTermQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, SchemaBuilder, TextFieldIndexing,
    TextOptions, Value,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::tokenizer::{TOKENIZER_NAME, register_on};

/// Writer heap size, bytes. Tantivy default (50 MB). Smaller values
/// flush more often and slow bulk backfill; larger values delay
/// visibility on `commit`.
const WRITER_HEAP_BYTES: usize = 50_000_000;

/// Query-time candidate cap. Same number the port states, imported
/// rather than restated so this adapter cannot silently clamp below
/// what callers were told they would get.
use asterism_core::domain::repository::RETRIEVAL_K_CEILING as MAX_QUERY_LIMIT;

/// Fuzzy edit distance for query-time typo tolerance. `1` catches
/// single-char English typos (`teh` → `the`, `recieve` → `receive`)
/// while keeping the term-dictionary walk cheap.
const FUZZY_DISTANCE: u8 = 1;

/// Minimum term length before fuzzy is applied. Short tokens
/// (`the`, `is`, `を`) would match too many neighbours at distance
/// 1 and drown the ranking.
const FUZZY_MIN_LEN: usize = 4;

/// Snippet window width, chars. Tantivy default is 150 which reads
/// as a full sentence; keep it tight for hover-burst preview UX.
const SNIPPET_MAX_CHARS: usize = 200;

struct SchemaBundle {
    schema: Schema,
    asset_id: Field,
    persona_id: Field,
    body: Field,
}

impl SchemaBundle {
    fn build() -> Self {
        let mut builder: SchemaBuilder = Schema::builder();
        let asset_id = builder.add_text_field("asset_id", STRING | STORED);
        let persona_id = builder.add_text_field("persona_id", STRING | STORED);
        let body_indexing = TextFieldIndexing::default()
            .set_tokenizer(TOKENIZER_NAME)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions);
        let body_options = TextOptions::default()
            .set_indexing_options(body_indexing)
            .set_stored();
        let body = builder.add_text_field("body", body_options);
        Self {
            schema: builder.build(),
            asset_id,
            persona_id,
            body,
        }
    }
}

/// Tantivy-backed [`AssetRetriever`] + [`AssetIndexer`] implementation.
///
/// Cheap to clone (`Arc` inside); one instance per process is enough.
#[derive(Clone)]
pub struct TantivyIndex {
    inner: Arc<TantivyInner>,
}

struct TantivyInner {
    index: Index,
    reader: IndexReader,
    /// `None` when opened read-only (see [`TantivyIndex::open_read_only`]):
    /// the write face (`upsert` / `remove` / `flush`) then rejects
    /// with a read-only error. `Some` after [`TantivyIndex::open`].
    writer: Option<Mutex<IndexWriter>>,
    fields: SchemaBundle,
}

impl TantivyIndex {
    /// Opens (creating on demand) the Tantivy index at `dir`. Registers
    /// the `mixed_body` tokenizer and prepares one writer + reader
    /// handle for the process lifetime.
    pub fn open(dir: PathBuf) -> Result<Self> {
        Self::open_inner(dir, true)
    }

    /// Opens the Tantivy index at `dir` for reading only. Identical to
    /// [`open`](Self::open) but the `IndexWriter` is not acquired, so
    /// only one process needs the exclusive writer lock. The write face
    /// (`upsert` / `remove` / `flush`) rejects with a read-only error;
    /// retrieval / reader paths are unaffected.
    pub fn open_read_only(dir: PathBuf) -> Result<Self> {
        Self::open_inner(dir, false)
    }

    fn open_inner(dir: PathBuf, writable: bool) -> Result<Self> {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create tantivy dir: {}", dir.display()))?;
        let fields = SchemaBundle::build();
        let mmap_dir = tantivy::directory::MmapDirectory::open(&dir)
            .with_context(|| format!("cannot mmap tantivy dir: {}", dir.display()))?;
        let index = Index::open_or_create(mmap_dir, fields.schema.clone())
            .context("open_or_create tantivy index")?;
        // Tokenizer must be registered before any add / search call.
        register_on(index.tokenizers())?;
        let writer = if writable {
            Some(Mutex::new(
                index
                    .writer(WRITER_HEAP_BYTES)
                    .context("acquire tantivy writer")?,
            ))
        } else {
            None
        };
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("build tantivy reader")?;
        Ok(Self {
            inner: Arc::new(TantivyInner {
                index,
                reader,
                writer,
                fields,
            }),
        })
    }

    /// Compiles a user query text into a Tantivy `Query` tree:
    /// `(TermQuery OR FuzzyTermQuery)` per token, combined with
    /// `SHOULD` under a `BooleanQuery`; the whole tree is `MUST`-ed
    /// with the persona filter when present.
    fn build_query(
        &self,
        text: &str,
        persona_filter: Option<&PersonaId>,
    ) -> Result<Box<dyn Query>> {
        // Reuse the QueryParser's tokenizer path so the query is
        // segmented / stemmed with the same `mixed_body` chain used
        // at index time.
        let parser = QueryParser::for_index(&self.inner.index, vec![self.inner.fields.body]);
        let terms = tokenize_body_terms(&self.inner.index, self.inner.fields.body, text);
        if terms.is_empty() {
            // Fallback to the plain parser so phrase-only queries or
            // syntax like `"quoted phrase"` still work.
            let q = parser
                .parse_query(text)
                .map_err(|e| anyhow!("tantivy query parse: {e}"))?;
            return match persona_filter {
                Some(pid) => Ok(Box::new(and_persona(q, self.inner.fields.persona_id, pid))),
                None => Ok(q),
            };
        }
        let mut per_term_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::with_capacity(terms.len());
        for term_text in terms {
            let term = Term::from_field_text(self.inner.fields.body, &term_text);
            let exact: Box<dyn Query> =
                Box::new(TermQuery::new(term.clone(), IndexRecordOption::WithFreqs));
            let branch: Box<dyn Query> = if term_text.chars().count() >= FUZZY_MIN_LEN {
                let fuzzy: Box<dyn Query> =
                    Box::new(FuzzyTermQuery::new(term, FUZZY_DISTANCE, true));
                Box::new(BooleanQuery::from(vec![
                    (Occur::Should, exact),
                    (Occur::Should, fuzzy),
                ]))
            } else {
                exact
            };
            per_term_clauses.push((Occur::Should, branch));
        }
        let text_query: Box<dyn Query> = Box::new(BooleanQuery::new(per_term_clauses));
        match persona_filter {
            Some(pid) => Ok(Box::new(and_persona(
                text_query,
                self.inner.fields.persona_id,
                pid,
            ))),
            None => Ok(text_query),
        }
    }
}

/// AND-combines a text query with a persona-scope `TermQuery`.
fn and_persona(text_query: Box<dyn Query>, persona_field: Field, pid: &PersonaId) -> BooleanQuery {
    let pid_term = Term::from_field_text(persona_field, &pid.to_string());
    let persona_query: Box<dyn Query> =
        Box::new(TermQuery::new(pid_term, IndexRecordOption::Basic));
    BooleanQuery::from(vec![
        (Occur::Must, text_query),
        (Occur::Must, persona_query),
    ])
}

/// Runs the registered `mixed_body` analyzer over the query text and
/// returns the surface form of each emitted token, deduped. The
/// per-token `FuzzyTermQuery` needs the same tokenization as the
/// indexed body so lindera-segmented Japanese and Porter-stemmed
/// English both match.
fn tokenize_body_terms(index: &Index, body_field: Field, text: &str) -> Vec<String> {
    let _ = body_field;
    let mut analyzer = match index.tokenizer_for_field(body_field) {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };
    let mut stream = analyzer.token_stream(text);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    while let Some(tok) = stream.next() {
        if tok.text.is_empty() {
            continue;
        }
        if seen.insert(tok.text.clone()) {
            out.push(tok.text.clone());
        }
    }
    out
}

#[async_trait]
impl AssetIndexer for TantivyIndex {
    async fn upsert(&self, doc_in: &IndexDoc) -> Result<(), DomainError> {
        let id_str = doc_in.asset_id.to_string();
        let pid_str = doc_in.persona_id.to_string();
        // An asset with no resolved body still gets a document: the
        // identity fields keep persona scoping and idempotent replace
        // working, and a later route (caption / derived tags) fills
        // the rest in without a schema change here.
        let body_owned = doc_in.text.clone().unwrap_or_default();
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let Some(writer_mutex) = inner.writer.as_ref() else {
                return Err(anyhow!("index is read-only in this process"));
            };
            let writer = writer_mutex.blocking_lock();
            // Delete any prior doc with the same asset_id so re-index
            // is idempotent.
            let id_term = Term::from_field_text(inner.fields.asset_id, &id_str);
            writer.delete_term(id_term);
            let mut doc = TantivyDocument::default();
            doc.add_text(inner.fields.asset_id, &id_str);
            doc.add_text(inner.fields.persona_id, &pid_str);
            doc.add_text(inner.fields.body, &body_owned);
            writer
                .add_document(doc)
                .map_err(|e| anyhow!("tantivy add_document: {e}"))?;
            Ok(())
        })
        .await
        .map_err(|e| DomainError::Infra(anyhow!("spawn_blocking join: {e}")))?
        .map_err(DomainError::Infra)?;
        Ok(())
    }

    async fn remove(&self, asset_id: &AssetId) -> Result<(), DomainError> {
        let id_str = asset_id.to_string();
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let Some(writer_mutex) = inner.writer.as_ref() else {
                return Err(anyhow!("index is read-only in this process"));
            };
            let writer = writer_mutex.blocking_lock();
            let id_term = Term::from_field_text(inner.fields.asset_id, &id_str);
            writer.delete_term(id_term);
            Ok(())
        })
        .await
        .map_err(|e| DomainError::Infra(anyhow!("spawn_blocking join: {e}")))?
        .map_err(DomainError::Infra)?;
        Ok(())
    }

    async fn flush(&self) -> Result<(), DomainError> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let Some(writer_mutex) = inner.writer.as_ref() else {
                return Err(anyhow!("index is read-only in this process"));
            };
            let mut writer = writer_mutex.blocking_lock();
            writer
                .commit()
                .map_err(|e| anyhow!("tantivy commit: {e}"))?;
            Ok(())
        })
        .await
        .map_err(|e| DomainError::Infra(anyhow!("spawn_blocking join: {e}")))?
        .map_err(DomainError::Infra)?;
        Ok(())
    }
}

#[async_trait]
impl AssetRetriever for TantivyIndex {
    async fn retrieve(&self, q: &RetrievalQuery) -> Result<Retrieved, DomainError> {
        let empty = || Retrieved {
            candidates: Vec::new(),
            truncated: false,
        };
        // `Similar` needs a vector space this adapter does not have.
        // Answering it with a text search over the asset's own body
        // would be a different question wearing the same name, so
        // decline instead — the caller can tell the route apart.
        let text = match &q.intent {
            RetrievalIntent::Text(t) => t.clone(),
            RetrievalIntent::Similar(_) => {
                return Err(DomainError::Validation(
                    "similar-asset retrieval has no backing index in this build".into(),
                ));
            }
        };
        if text.trim().is_empty() {
            return Ok(empty());
        }
        let persona_filter = q.scope.as_ref();
        let limit = q.k.clamp(1, MAX_QUERY_LIMIT) as usize;
        let query = self
            .build_query(&text, persona_filter)
            .map_err(DomainError::Infra)?;
        let inner = self.inner.clone();
        let body_text_owned = text.to_string();
        let persona_owned = persona_filter.copied();
        let _ = persona_owned;
        let hits = tokio::task::spawn_blocking(move || -> Result<Vec<Candidate>> {
            let searcher = inner.reader.searcher();
            let top = searcher
                .search(&*query, &TopDocs::with_limit(limit))
                .map_err(|e| anyhow!("tantivy search: {e}"))?;
            let snippet_gen = SnippetGenerator::create(&searcher, &*query, inner.fields.body)
                .map(|mut g| {
                    g.set_max_num_chars(SNIPPET_MAX_CHARS);
                    g
                })
                .ok();
            let mut out = Vec::with_capacity(top.len());
            for (score, doc_addr) in top {
                let doc: TantivyDocument = match searcher.doc(doc_addr) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let asset_id_str = match doc
                    .get_first(inner.fields.asset_id)
                    .and_then(|v| v.as_str())
                {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let persona_id_str = match doc
                    .get_first(inner.fields.persona_id)
                    .and_then(|v| v.as_str())
                {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let asset_uuid = match Uuid::parse_str(&asset_id_str) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                let persona_uuid = match Uuid::parse_str(&persona_id_str) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                let snippet = snippet_gen
                    .as_ref()
                    .map(|g| g.snippet_from_doc(&doc))
                    .and_then(|s| {
                        let html = s.to_html();
                        if html.is_empty() { None } else { Some(html) }
                    });
                out.push(Candidate {
                    asset_id: AssetId::from_uuid(asset_uuid),
                    persona_id: PersonaId::from_uuid(persona_uuid),
                    score,
                    evidence: match snippet {
                        Some(s) => Evidence::Snippet(s),
                        None => Evidence::None,
                    },
                });
            }
            let _ = body_text_owned;
            Ok(out)
        })
        .await
        .map_err(|e| DomainError::Infra(anyhow!("spawn_blocking join: {e}")))?
        .map_err(DomainError::Infra)?;
        // A full shortlist means the heap was still discarding
        // candidates when it stopped, so there is more behind it.
        // Docs skipped above (unparsable id / missing field) only make
        // this conservative, never over-claiming.
        let truncated = hits.len() >= limit;
        Ok(Retrieved {
            candidates: hits,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(asset_id: AssetId, persona_id: PersonaId, text: &str) -> IndexDoc {
        IndexDoc {
            asset_id,
            persona_id,
            text: Some(text.to_string()),
        }
    }

    fn text_query(text: &str, k: u32) -> RetrievalQuery {
        RetrievalQuery {
            intent: RetrievalIntent::Text(text.to_string()),
            scope: None,
            k,
        }
    }

    #[tokio::test]
    async fn read_only_rejects_writes() {
        let dir = tempfile::tempdir().unwrap();
        let index = TantivyIndex::open_read_only(dir.path().to_path_buf()).unwrap();
        let asset_id = AssetId::new();
        let persona_id = PersonaId::new();

        let add = index
            .upsert(&doc(asset_id, persona_id, "hello world"))
            .await;
        assert!(add.is_err());
        assert!(add.unwrap_err().to_string().contains("read-only"));

        let del = index.remove(&asset_id).await;
        assert!(del.is_err());
        assert!(del.unwrap_err().to_string().contains("read-only"));

        let flushed = index.flush().await;
        assert!(flushed.is_err());
        assert!(flushed.unwrap_err().to_string().contains("read-only"));
    }

    #[tokio::test]
    async fn read_only_retrieval_reads_committed_docs() {
        let dir = tempfile::tempdir().unwrap();
        let asset_id = AssetId::new();
        let persona_id = PersonaId::new();

        // Write + flush through a writable handle first.
        {
            let writer = TantivyIndex::open(dir.path().to_path_buf()).unwrap();
            writer
                .upsert(&doc(asset_id, persona_id, "constellation stargazing notes"))
                .await
                .unwrap();
            writer.flush().await.unwrap();
        }

        // A read-only handle on the same dir can retrieve the committed doc.
        let reader = TantivyIndex::open_read_only(dir.path().to_path_buf()).unwrap();
        let found = reader
            .retrieve(&text_query("stargazing", 10))
            .await
            .unwrap();
        assert_eq!(found.candidates.len(), 1);
        assert_eq!(found.candidates[0].asset_id, asset_id);
        assert!(
            !found.truncated,
            "one candidate under a ceiling of ten is not a truncated shortlist"
        );
    }

    /// `truncated` is the signal callers use to avoid presenting a
    /// shortlist as the whole answer, so it has to be set by the
    /// shortlist filling up — not by how many documents exist.
    #[tokio::test]
    async fn a_full_shortlist_reports_itself_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let persona_id = PersonaId::new();
        {
            let writer = TantivyIndex::open(dir.path().to_path_buf()).unwrap();
            for _ in 0..3 {
                writer
                    .upsert(&doc(AssetId::new(), persona_id, "orbit orbit orbit"))
                    .await
                    .unwrap();
            }
            writer.flush().await.unwrap();
        }
        let index = TantivyIndex::open_read_only(dir.path().to_path_buf()).unwrap();

        // k below the number of matches: the heap was still discarding
        // when it stopped.
        let clipped = index.retrieve(&text_query("orbit", 2)).await.unwrap();
        assert_eq!(clipped.candidates.len(), 2);
        assert!(clipped.truncated, "a full shortlist has more behind it");

        // k above it: everything that matches fits, nothing is hidden.
        let whole = index.retrieve(&text_query("orbit", 10)).await.unwrap();
        assert_eq!(whole.candidates.len(), 3);
        assert!(!whole.truncated);
    }

    /// The adapter must not answer a `Similar` query with a text search
    /// over the asset's own body: that is a different question, and
    /// answering it silently would make the caller believe an
    /// embedding route exists.
    #[tokio::test]
    async fn similar_is_refused_rather_than_answered_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let index = TantivyIndex::open_read_only(dir.path().to_path_buf()).unwrap();
        let err = index
            .retrieve(&RetrievalQuery {
                intent: RetrievalIntent::Similar(AssetId::new()),
                scope: None,
                k: 10,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("similar-asset retrieval"));
    }
}
