//! Claude Code session import adapter — turns a session's JSONL log
//! into `Footprint::ChatMessage`s (plus `Footprint::Image` for pasted
//! images).
//!
//! One of the per-modality adapters behind the unified
//! `asterism-importer` CLI: [`parser::CcSessionParser`] implements the
//! importer SDK's `SourceParser`, the SDK pipeline walks the source and
//! pushes the resulting footprints to a running `asterism-server` over
//! HTTP. The JSONL shape, the addressing rules, and their edge cases
//! are documented in [`parser`].

pub mod parser;

pub use parser::CcSessionParser;
