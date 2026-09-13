//! Where an importer got to, kept so the next run does not start over.
//!
//! An importer scans a source and hands records to the server. #293 gave
//! it a way to say where it stopped — a checkpoint on the scan's own
//! stream, carried out of a run that earned one — and no way to keep
//! that anywhere. This is the place it is kept, and the whole of what
//! this layer does with it is store it and hand it back.
//!
//! ## The one rule
//!
//! **The offset is opaque here.** It belongs to the adapter that wrote
//! it, and nothing on this side parses, validates, migrates or compares
//! it. That rule is stated in the inbound port's own vocabulary
//! (`asterism-importer-sdk`'s `SyncState`), and this is the layer where
//! breaking it would be easiest and least visible: a column tempting
//! somebody to index, a service tempted to "fix up" a shape it
//! recognises. It is carried as text for that reason as much as for the
//! bindings — text has nothing to be clever about.
//!
//! The **key** is not opaque, because a store that could not compare
//! keys could not find anything. It is a persona and a partition, and
//! both halves are load-bearing. The persona, because the same source
//! imported into two personas has two independent positions and one
//! being ahead says nothing about the other. The partition, because it
//! is the adapter's own statement of what it is scanning — and the
//! adapter names it fully, its own kind included, so two adapters cannot
//! collide inside one persona by both calling something `root=/photos`.
//! That is also why the key is a pair and not a triple with the kind
//! beside it: the adapter owns the encoding, the way it owns the
//! offset's.

use chrono::{DateTime, Utc};

/// What a stored position is a position *in*.
///
/// Compared, never interpreted. An adapter that scans something compound
/// encodes it into the one string and owns that encoding — the same
/// bargain the offset has, one level up.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportStateKey {
    /// Persona the import lands in.
    pub persona_id: String,
    /// The adapter's own name for what it is scanning.
    pub partition: String,
}

impl ImportStateKey {
    /// Builds a key from its two halves.
    pub fn new(persona_id: impl Into<String>, partition: impl Into<String>) -> Self {
        Self {
            persona_id: persona_id.into(),
            partition: partition.into(),
        }
    }
}

/// A resumption point as this side holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportState {
    /// What the position is a position in.
    pub key: ImportStateKey,
    /// The position, as the adapter wrote it, verbatim.
    ///
    /// Text, and never a parsed value: see the module doc. A caller
    /// handing this back to the adapter that wrote it is handing back
    /// the same bytes.
    pub offset_json: String,
    /// When this point was last written.
    ///
    /// Not part of the key and not compared with anything: it is for a
    /// person asking why an import has not moved, which is the question
    /// a stalled adapter raises.
    pub updated_at: DateTime<Utc>,
}
