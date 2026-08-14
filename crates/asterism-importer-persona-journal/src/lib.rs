//! persona-journal import adapter — turns a persona-journal SQLite row
//! into `Footprint::JournalEntry`.
//!
//! One of the per-modality adapters behind the unified
//! `asterism-importer` CLI: [`parser::PersonaJournalParser`] implements
//! the importer SDK's `SourceParser`, the SDK pipeline walks the source
//! and pushes the resulting footprints to a running `asterism-server`
//! over HTTP. The row schema it reads and the mapping it applies are
//! documented in [`parser`].

pub mod parser;

pub use parser::PersonaJournalParser;
