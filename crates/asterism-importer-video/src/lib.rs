//! Video import adapter — turns a scanned video file into
//! `Footprint::Video`.
//!
//! One of the per-modality adapters behind the unified
//! `asterism-importer` CLI: [`parser::VideoParser`] implements the
//! importer SDK's `SourceParser`, the SDK pipeline walks the source and
//! pushes the resulting footprints to a running `asterism-server` over
//! HTTP. The format specifics live in [`parser`]; this crate root only
//! re-exports the parser type.

pub mod parser;

pub use parser::VideoParser;
