//! `Footprint` — the typed, plugin-facing shape a parser hands back.
//!
//! A footprint is *one* thing the user collected: one chat message, one
//! image, one doc, one note. This matches the domain intent in
//! `asterism-core::domain::asset::Asset` — an aggregate root for a single
//! footprint. The SDK exposes a typed enum here so plugin authors do not
//! have to guess which fields belong on which modality; the compiler
//! shows them the shape of every variant.
//!
//! # Design notes
//!
//! - **One `Footprint` = one `Asset` on the server.** A parser that
//!   receives one `RawItem` (say, one `.jsonl` file) usually emits many
//!   footprints (one per chat message inside the file).
//! - **Variants are aligned with `Modality` well-known slugs.** New
//!   modalities are added as new variants; the crate's API is still
//!   pre-stability, so breaking additions
//!   are cheap. Aspirationally-open extension via a `Custom` variant is
//!   deferred until an external plugin author actually needs it.
//! - **`extra` is a per-variant escape hatch.** Fields promoted to
//!   variant fields have a `Some`/typed shape enforced by the compiler;
//!   everything else lives in `extra` as raw JSON. When several sources
//!   grow the same key in `extra`, promote it.
//! - **The mapping to `AssetSpec` lives here, not in each parser.** The
//!   SDK owns the cover-hint truncation, the modality slug, the label
//!   normalisation — plugin authors only choose the variant and fill in
//!   the typed fields.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::mapper::AssetSpec;

/// Maximum cover hint length in Unicode scalar values.
pub const COVER_MAX_CHARS: usize = 200;
/// Maximum register-note preview length in Unicode scalar values.
pub const REGISTER_MAX_CHARS: usize = 80;

/// Reference to the raw source of a footprint.
///
/// Distinct from `asterism_core::domain::value::SourceRef` (the domain
/// object) because the SDK layer speaks in slugs / strings so plugin
/// authors do not have to depend on the core crate.
#[derive(Debug, Clone)]
pub struct FootprintSource {
    /// Source kind slug (matches the scanner's `source_kind`; typically
    /// something like `"cc"`, `"sqlite"`, `"fs"`).
    pub kind: String,
    /// Where **this footprint** came from (not the container).
    ///
    /// It says one thing, and it is what its name says: the address the
    /// record was read from. Parsers must produce a value that stays
    /// **stable across re-imports of the same record**, so that a
    /// second arrival is recognisable as the same record arriving
    /// again. Common patterns:
    ///
    /// - `<file-path>` — the raw item is itself one footprint (image,
    ///   standalone doc).
    /// - `<file-path>#<record-uuid>` — one chat message inside a JSONL
    ///   session log.
    /// - `<db-path>?<query-hash>#<row-id>` — one row inside a scanned
    ///   SQLite table.
    ///
    /// **It is not an identity, and it is not unique.** The server
    /// looks the value up before it mints — inside the persona, among
    /// live rows — and a hit is answered by handing back the row that
    /// was already there. That is the default answer, not an enforced
    /// one: two assets may carry one address (a lane that writes every
    /// run's output to the same path says so with
    /// `on_duplicate = separate`), and an address that moves rewrites a
    /// property rather than destroying the row's claim to itself.
    ///
    /// **Watch-mode / append-friendly sources.** When the source is a
    /// container that grows over time (a `.jsonl` session log the
    /// upstream tool keeps appending to),
    /// [`crate::scanner::ScanMode::Watch`] on the FS scanner will
    /// re-emit the whole file every time it changes. That is fine —
    /// **as long as the locator is per-record**: unchanged records are
    /// recognised by a lookup on the same columns, and only genuinely
    /// new records land. The `<file-path>#<record-uuid>` pattern is the
    /// canonical shape for this case; a container-level locator
    /// (`<file-path>` alone) on an append-heavy source makes every
    /// flush one arrival at one address, so the whole file is answered
    /// by the row it already produced and its new records are never
    /// seen. Pick a record-level suffix that is intrinsic to the record
    /// (an id column, a content hash, a line number as a last resort),
    /// never a re-scan-dependent counter.
    ///
    /// The SDK keeps no ledger of its own, so recognition happens
    /// server-side either way — one comparison per emitted record. What
    /// the lookup changed is that the comparison now succeeds instead
    /// of raising, so the cost of a re-emitted record moved off the
    /// error path.
    pub locator: String,
    /// Human-readable name of the platform the footprint originated
    /// from (e.g. `"Claude Code"`, `"Slack"`, `"iMessage"`). Optional.
    pub platform: Option<String>,
    /// What the source itself calls this record — an issue key, a row's
    /// primary key, an upstream API's id.
    ///
    /// **For external linkage, not for dedup.** It lands on
    /// `asset.external_key`, where it is a property the row carries so
    /// that something outside can find its way back in. Nothing about
    /// matching or minting reads it: "have I seen this record" is
    /// answered by [`locator`](Self::locator), and "are these the same
    /// thing" by the digest axes. A source-stated id is a claim from
    /// outside the library, and the library does not let anything
    /// outside it decide what is one thing and what is two.
    ///
    /// `None` when the source states no id of its own, which is most of
    /// them — a filesystem path is an address, not a name the source
    /// gave the record.
    ///
    /// **Repeats are ordinary, and so are collisions.** Nothing refuses
    /// a key the library already holds, and nothing can: an external
    /// record legitimately arrives more than once (sign an image, ingest
    /// it, update it, ingest it again — the source states the same key
    /// both times), and two unrelated platforms both numbering a record
    /// `12345` is normal. V62 took the last UNIQUE off the column for
    /// exactly those two reasons. State whatever the source states; it
    /// is stored as stated.
    ///
    /// The route is `FootprintSource::external_id` →
    /// `AssetSpec::external_key` → `AddAssetCommand::external_key` →
    /// `asset.external_key`, verbatim at every hop.
    pub external_id: Option<String>,
}

/// Role a chat participant played on a specific message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRole {
    /// Human input.
    User,
    /// Model / bot response.
    Assistant,
    /// System / tool-side message (setup, side channel).
    System,
    /// Tool invocation surface (tool_use / tool_result records).
    Tool,
    /// Anything that does not fit the four above; the slug is passed
    /// through to the label as-is.
    Other(String),
}

impl ChatRole {
    /// Label slug used on the produced asset.
    pub fn as_slug(&self) -> &str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Format of a `Doc` footprint.
///
/// Well-known Doc containers get typed variants so the compiler
/// guides plugin authors and the grid UI can facet cleanly. New
/// widely-adopted formats are added here as new variants while the
/// enum can still grow freely; reserve
/// [`DocFormat::Other`] for one-off vendor slugs the SDK
/// does not track.
///
/// Interactive artifacts (Claude Artifacts, ChatGPT Canvas, GitHub
/// Gist) map straight onto [`DocFormat::Html`] / [`DocFormat::Code`]
/// / [`DocFormat::Markdown`] as appropriate — no dedicated variant.
/// Their `Doc.labels` carry the origin service (`"claude-artifact"`,
/// `"gist"`, …) for facet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocFormat {
    /// Markdown source.
    Markdown,
    /// PDF binary (Asterism does not extract; the plugin must pass an
    /// `excerpt` it has already produced).
    Pdf,
    /// HTML page (typically rendered to text before setting `excerpt`).
    /// Covers static `.html` files, Claude Artifacts, and any HTML
    /// blob a downstream renderer needs to display verbatim.
    Html,
    /// Plain text.
    Plain,
    /// Source code; the payload identifies the language (`"rust"`,
    /// `"lua"`, …). Kept distinct from `Plain` because the cover
    /// treatment is different.
    Code(String),
    /// Terminal session log — plain text capture of a shell session
    /// (Unix `script(1)` output, `.log` from tmux capture-pane,
    /// asciinema `.cast` JSON, raw ANSI-escaped console dump). Kept
    /// distinct from [`Plain`](DocFormat::Plain) because the cover UI
    /// benefits from monospace treatment and downstream tooling wants
    /// to strip ANSI escapes before display.
    TermLog,
    /// Charmbracelet VHS `.tape` script — declarative terminal
    /// recording source (commands + timings), not a rendered log.
    /// Encoded output is a separate footprint (Image for GIF, Video
    /// for MP4).
    TermVhs,
    /// Any other format; the slug is passed through. Use for one-off
    /// vendor slugs the SDK does not track (e.g. `"asciinema"` for
    /// the JSON cast wrapper when a caller prefers it over
    /// [`TermLog`](DocFormat::TermLog), `"observable"` for Observable
    /// notebooks, …).
    Other(String),
}

impl DocFormat {
    /// Slug used as an asset label.
    pub fn as_label(&self) -> String {
        match self {
            Self::Markdown => "markdown".into(),
            Self::Pdf => "pdf".into(),
            Self::Html => "html".into(),
            Self::Plain => "plain".into(),
            Self::Code(lang) => format!("code:{lang}"),
            Self::TermLog => "term-log".into(),
            Self::TermVhs => "term-vhs".into(),
            Self::Other(slug) => slug.clone(),
        }
    }
}

/// One chat / dialogue message (Claude Code turn, Slack message,
/// iMessage, …).
///
/// Maps to `Modality::DIALOGUE` on the server. `external_session_key`
/// is **required** for chat because grouping is the primary
/// constellation signal for this modality — the server resolves the
/// key to a `Session.id` through
/// `SessionService::find_or_create_by_external_key`, so re-imports
/// converge on the same Session row.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// Source reference. `locator` must be unique per message
    /// (typically `<file>#<msg-uuid>`).
    pub source: FootprintSource,
    /// Time the message occurred in the outside world.
    pub occurred_at: DateTime<Utc>,
    /// External session identifier the importer knows (Claude Code
    /// session UUID, JSONL file stem, conversation id, thread id, …).
    /// Required — chat without a session context has nothing to
    /// cluster on. The server resolves it to a `Session.id` via the
    /// find-or-create path.
    pub external_session_key: String,
    /// Who authored the message.
    pub role: ChatRole,
    /// Text body. Becomes the cover hint (truncated) and register
    /// note (further truncated).
    pub body: String,
    /// Position within the session (0-indexed line / message number)
    /// when known. Used by the domain as a secondary ordering tie
    /// breaker.
    pub thread_position: Option<u64>,
    /// Parent message id when the source is a thread / tree.
    pub parent_message_id: Option<String>,
    /// Additional free-form labels. `role.as_slug()` is prepended
    /// automatically during `into_asset_spec`.
    pub labels: Vec<String>,
    /// Source-specific extension bag. Kept as JSON; parsers that want
    /// the ergonomic API construct it with `serde_json::json!(...)`.
    pub extra: Value,
}

/// One doc / written work product (Markdown note, PDF, spec, code
/// file, …).
///
/// Maps to `Modality::WORK_PRODUCT`.
#[derive(Debug, Clone)]
pub struct Doc {
    /// Source reference; `locator` is typically the file path (docs
    /// are usually one file = one footprint).
    pub source: FootprintSource,
    /// Time the doc was authored / updated.
    pub occurred_at: DateTime<Utc>,
    /// Optional title (first heading, filename, …). Becomes the
    /// register note when present.
    pub title: Option<String>,
    /// Short excerpt shown on the card cover. Truncated at
    /// `COVER_MAX_CHARS`.
    pub excerpt: String,
    /// Format signal (surfaced as a label).
    pub format: DocFormat,
    /// Optional constellation-edge grouping key — e.g. a book uuid
    /// when the doc is part of a series, or a repo slug when it is
    /// a code file. Modality-agnostic (non-Dialog);
    /// [`ChatMessage`] uses `external_session_key` instead.
    pub bundle_id: Option<String>,
    /// Original file size on disk.
    pub file_size_bytes: Option<u64>,
    /// Word count, when the plugin has cheaply computed it.
    pub word_count: Option<u64>,
    /// Additional free-form labels.
    pub labels: Vec<String>,
    /// Source-specific extension bag.
    pub extra: Value,
}

/// One short note (mood, idea, quick capture).
///
/// Maps to `Modality::MEMORY` — kept distinct from `Doc` because the
/// cover template and grid weighting differ.
#[derive(Debug, Clone)]
pub struct Note {
    /// Source reference.
    pub source: FootprintSource,
    /// Time the note was recorded.
    pub occurred_at: DateTime<Utc>,
    /// Body text. Becomes the cover hint (truncated) and register
    /// note (further truncated).
    pub body: String,
    /// App the note originated from (Apple Notes, Bear, Obsidian, …).
    pub source_app: Option<String>,
    /// Additional free-form labels.
    pub labels: Vec<String>,
    /// Optional constellation-edge grouping key used to link this
    /// note to sibling assets that came out of the same container
    /// (e.g. a character card's slots share one `bundle_id`, derived
    /// via [`crate::bundle::session_id_for`], so `edge_rebuild`
    /// produces a `same-bundle` edge across them).
    pub bundle_id: Option<String>,
    /// Source-specific extension bag.
    pub extra: Value,
}

/// One image (photo, screenshot, drawing).
///
/// Maps to modality slug `"image"` (open slug — not yet a well-known
/// constant on the domain side, but recognised as valid there).
#[derive(Debug, Clone)]
pub struct Image {
    /// Source reference.
    pub source: FootprintSource,
    /// Time the image was captured.
    pub occurred_at: DateTime<Utc>,
    /// Session container binding (asset-model v4 P4). `Some(key)` for
    /// an image that entered a conversation — the server resolves the
    /// key to the same composite the conversation's messages belong
    /// to, so the image lands as a member of that Session (membership
    /// is modality-agnostic). `None` for standalone images.
    pub external_session_key: Option<String>,
    /// Alt text / caption. Becomes the cover hint when present.
    pub alt: Option<String>,
    /// Pixel dimensions when known.
    pub dims: Option<(u32, u32)>,
    /// Original file size.
    pub file_size_bytes: Option<u64>,
    /// Additional free-form labels.
    pub labels: Vec<String>,
    /// Optional constellation-edge grouping key used to link this
    /// image to sibling assets that came out of the same container
    /// (a character card's avatar beside its slots, say).
    ///
    /// An ordinary image import leaves it `None`: the text inside a
    /// PNG is that image's own metadata rather than a sibling asset,
    /// so there is nothing for it to be bundled with.
    pub bundle_id: Option<String>,
    /// Source-specific extension bag (EXIF fields, camera model, …).
    pub extra: Value,
    /// Declared origin (`sidecar` when the file arrived with an
    /// exporter-written `<name>.meta.json` beside it).
    ///
    /// Images are the kind that actually leaves and comes back — an
    /// I2I round trip returns a new file whose bytes say nothing about
    /// where it came from. When the sidecar is still next to it, the
    /// parser can say so and let the server resolve the link.
    pub derived_from: Option<String>,
    /// AlbumMeta statements the parser read out of the file — an
    /// identifier a generator wrote in, a catalogue number, anything
    /// the point of which is to find this row by it later.
    ///
    /// Placed on the three media variants only, following
    /// `derived_from`: the reason is the same one, which is that these
    /// are the artefacts that physically carry a metadata block. A chat
    /// message or a journal entry arrives as a line in a log, and the
    /// identifier it has is the session key it is already filed under.
    ///
    /// **No built-in extractor fills this in**, and that is a finding
    /// rather than a gap: the measured example — see
    /// `asterism_core::domain::album_meta` — is that a ComfyUI graph
    /// carries *no identifier for the artefact at all*. Its node ids
    /// are unique only inside one file and its input reference is a
    /// bare filename, so there is nothing there to state. Seeds and
    /// checkpoint names are generation parameters, and they are already
    /// surfaced as keywords. An adapter pack whose ecosystem does mint
    /// one is the caller this field is here for.
    pub album_meta: std::collections::BTreeMap<String, String>,
}

/// One video (recording, screen capture, AI-generated clip).
///
/// Maps to modality slug `"video"`. Symmetric with [`Image`] plus
/// `duration_ms` / `codec` / `framerate` fields that videos need for
/// grid layout and playback previews.
#[derive(Debug, Clone)]
pub struct Video {
    /// Source reference; `locator` is typically the file path
    /// (videos are one file = one footprint by convention — split at
    /// container-level would confuse timeline UI).
    pub source: FootprintSource,
    /// Time the video was recorded / generated.
    pub occurred_at: DateTime<Utc>,
    /// Caption / title / filename stem. Becomes the cover hint when
    /// present.
    pub alt: Option<String>,
    /// Pixel dimensions of the display track when known.
    pub dims: Option<(u32, u32)>,
    /// Duration in milliseconds when the parser can cheaply extract
    /// it (ISOBMFF `mvhd` box, WebM `Segment/Info/Duration`).
    pub duration_ms: Option<u64>,
    /// Original file size on disk.
    pub file_size_bytes: Option<u64>,
    /// Video codec slug (`"h264"`, `"h265"`, `"av1"`, `"vp9"`, …)
    /// when the container advertises one.
    pub codec: Option<String>,
    /// Frame rate (fps) when known. `f32` because container metadata
    /// commonly encodes it as a rational (30000/1001) that reduces
    /// to a non-integer.
    pub framerate: Option<f32>,
    /// Additional free-form labels.
    pub labels: Vec<String>,
    /// Optional constellation-edge grouping key used to link this
    /// video to sibling assets (transcripts, thumbnails, generation
    /// prompts) that originated from the same physical resource.
    pub bundle_id: Option<String>,
    /// Source-specific extension bag (container-format metadata,
    /// AI generation prompt, camera model, …).
    pub extra: Value,
    /// Declared origin (`sidecar` when the file arrived with an
    /// exporter-written `<name>.meta.json` beside it).
    ///
    /// Videos leave and come back the same way images do — a
    /// generation hop returns a new container whose bytes say nothing
    /// about the frames it was made from, and video embedded metadata
    /// is even less trustworthy than an image's (mp4 workflow blocks
    /// are dropped by common tooling). The sidecar next to the file
    /// is the reliable copy of the link.
    pub derived_from: Option<String>,
    /// AlbumMeta statements the parser read out of the container — see
    /// [`Image::album_meta`], which carries the reasoning for all
    /// three media variants.
    pub album_meta: std::collections::BTreeMap<String, String>,
}

/// One audio clip (voice memo, VoiceLoid / VoiceVox synthesis
/// output, podcast episode, music track, dictation).
///
/// Maps to modality slug `"audio"`. `voice` / `music` /
/// `voice-synth` etc. are conveyed as labels for facet, not as
/// separate variants — the storage shape and UI treatment are the
/// same at the SDK layer.
#[derive(Debug, Clone)]
pub struct Audio {
    /// Source reference; `locator` is typically the file path.
    pub source: FootprintSource,
    /// Time the audio was recorded / synthesised.
    pub occurred_at: DateTime<Utc>,
    /// Caption / title / filename stem. Becomes the cover hint when
    /// present.
    pub alt: Option<String>,
    /// Duration in milliseconds when the parser can cheaply extract
    /// it (MP3 XING header, MP4 `mvhd`, FLAC STREAMINFO, OGG page).
    pub duration_ms: Option<u64>,
    /// Original file size on disk.
    pub file_size_bytes: Option<u64>,
    /// Codec slug (`"mp3"`, `"aac"`, `"flac"`, `"opus"`, `"vorbis"`,
    /// `"pcm"`, …).
    pub codec: Option<String>,
    /// Sample rate in Hz (typically 44100 or 48000).
    pub sample_rate: Option<u32>,
    /// Number of audio channels (`1` = mono, `2` = stereo, …).
    pub channels: Option<u16>,
    /// Additional free-form labels.
    pub labels: Vec<String>,
    /// Optional constellation-edge grouping key (sibling transcript
    /// notes, cover art image, generation prompt).
    pub bundle_id: Option<String>,
    /// Source-specific extension bag (ID3 tags, MP4 metadata,
    /// VoiceLoid / VoiceVox synthesis parameters, …).
    pub extra: Value,
    /// Declared origin (`sidecar` when the file arrived with an
    /// exporter-written `<name>.meta.json` beside it).
    ///
    /// Audio round-trips the same way: a synthesis / conversion hop
    /// returns a new container, and the sidecar is what still points
    /// at the material it was made from.
    pub derived_from: Option<String>,
    /// AlbumMeta statements the parser read out of the container — see
    /// [`Image::album_meta`], which carries the reasoning for all
    /// three media variants.
    pub album_meta: std::collections::BTreeMap<String, String>,
}

/// One terminal-session transcript / Persona Tape (`.tape`, `.cast`, `.log`).
///
/// Maps to `Modality::TAPE`.
#[derive(Debug, Clone)]
pub struct Tape {
    /// Source reference; `locator` is typically the file path or tape id.
    pub source: FootprintSource,
    /// Time the tape session occurred.
    pub occurred_at: DateTime<Utc>,
    /// Optional title / stem.
    pub title: Option<String>,
    /// Excerpt shown on the card cover.
    pub excerpt: String,
    /// Constellation-edge grouping key — file stem for a tape file,
    /// tape id when the source system provides one.
    pub bundle_id: Option<String>,
    /// Original file size on disk.
    pub file_size_bytes: Option<u64>,
    /// Additional free-form labels.
    pub labels: Vec<String>,
    /// Source-specific extension bag.
    pub extra: Value,
}

/// Kind of journal-style entry the plugin is emitting.
///
/// Mirrors the domain's `Modality::STATE / EMO / NON_REM / MEMORY /
/// TICK_LOG` well-known slugs. `Other` is an escape hatch —
/// the slug is passed through verbatim so sources with a bespoke kind
/// (`"dream"`, `"gratitude_log"`, …) can round-trip without an SDK
/// change. The domain's `Modality` slug space is open, so custom
/// values are accepted server-side too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalKind {
    /// Register / mood-state note (`Modality::STATE`).
    State,
    /// Emotional-tilt marker (`Modality::EMO`).
    Emo,
    /// Dream / subconscious fragment (`Modality::NON_REM`).
    NonRem,
    /// Memory / long-term note (`Modality::MEMORY`).
    Memory,
    /// Tick log from a periodic cycle (`Modality::TICK_LOG`).
    TickLog,
    /// Anything else; the slug is passed through as-is.
    Other(String),
}

impl JournalKind {
    /// Slug written to `AssetSpec::modality`.
    pub fn as_modality_slug(&self) -> &str {
        match self {
            Self::State => "state",
            Self::Emo => "emo",
            Self::NonRem => "non_rem",
            Self::Memory => "memory",
            Self::TickLog => "tick_log",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// One journal-style entry — short self-authored text with a
/// domain-specific modality picked from `JournalKind`.
///
/// Distinct from [`Note`]: `Note` hardcodes `Modality::MEMORY` and is
/// meant for sources with no journal semantics (Apple Notes, Bear,
/// Obsidian, …). `JournalEntry` covers persona-journal rows,
/// tick-log emitters, and anything else where the source itself
/// distinguishes state / emo / non_rem / memory / tick_log at row level.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    /// Source reference.
    pub source: FootprintSource,
    /// Time the entry was recorded.
    pub occurred_at: DateTime<Utc>,
    /// Modality slug via [`JournalKind`].
    pub kind: JournalKind,
    /// Body text. Becomes the cover hint (truncated) and register
    /// note (further truncated).
    pub body: String,
    /// Optional constellation-edge grouping key used to link this
    /// entry to sibling assets (see `Note::bundle_id` for the
    /// edge_rebuild semantics).
    pub bundle_id: Option<String>,
    /// Additional free-form labels.
    pub labels: Vec<String>,
    /// Source-specific extension bag.
    pub extra: Value,
}

/// The typed shape a parser hands back.
///
/// Every parser returns `Vec<Footprint>` — the vector may be empty
/// (raw item was noise) or many (one JSONL file yielding many
/// messages). The SDK converts each footprint to an `AssetSpec` before
/// batching to the server; plugin authors do not touch `AssetSpec`
/// directly.
#[derive(Debug, Clone)]
pub enum Footprint {
    /// One chat / dialogue message.
    ChatMessage(ChatMessage),
    /// One written doc / work product.
    Doc(Doc),
    /// One short note (defaults to `Modality::MEMORY`).
    Note(Note),
    /// One journal-style entry with an explicit
    /// [`JournalKind`]-derived modality.
    JournalEntry(JournalEntry),
    /// One image.
    Image(Image),
    /// One video (recording, screen capture, AI-generated clip).
    Video(Video),
    /// One audio clip (voice memo, VoiceLoid / VoiceVox synthesis,
    /// podcast, music, dictation).
    Audio(Audio),
    /// One terminal-session transcript / Persona Tape (`Modality::TAPE`).
    Tape(Tape),
}

impl Footprint {
    /// Convert to the flat `AssetSpec` the mapper feeds to the wire
    /// command. Truncation of cover / register text and the label
    /// prepend for `ChatRole` happen here — plugin authors do not
    /// need to know these rules.
    ///
    /// Attribution (`author_*` / `operator_ai`) comes out `None` from
    /// every arm: a `Footprint` describes what was found, and none of
    /// its variants carries an assertion about who it is by or what
    /// drove the import. An importer that knows sets the fields on the
    /// `AssetSpec` afterwards — filling them in here would be the
    /// pipeline asserting on the caller's behalf.
    ///
    /// `album_meta` is the opposite case and travels through: a parser
    /// reading an identifier out of an artefact *is* describing what it
    /// found, which is what a `Footprint` is for. It rides on the three
    /// media variants ([`Image::album_meta`] carries the reasoning) and
    /// comes out empty from the other five, which have no slot because
    /// their artefacts carry no metadata block to read one from.
    ///
    /// `declared_content_hash` comes out `None` for the same reason
    /// and one more: a `Footprint` holds no bytes. The side that read
    /// the payload is the scanner, and the side that can tell whether
    /// this spec still addresses those bytes is the pipeline, which
    /// fills the field in afterwards when both hold
    /// ([`run_import`](crate::runner::run_import)). A caller driving
    /// the mapping itself may set it on the spec returned here.
    pub fn into_asset_spec(self) -> AssetSpec {
        match self {
            Self::ChatMessage(m) => chat_to_spec(m),
            Self::Doc(d) => doc_to_spec(d),
            Self::Note(n) => note_to_spec(n),
            Self::JournalEntry(j) => journal_to_spec(j),
            Self::Image(i) => image_to_spec(i),
            Self::Video(v) => video_to_spec(v),
            Self::Audio(a) => audio_to_spec(a),
            Self::Tape(t) => tape_to_spec(t),
        }
    }
}

fn chat_to_spec(m: ChatMessage) -> AssetSpec {
    let body = m.body.trim();
    let cover = truncate_chars(body, COVER_MAX_CHARS);
    let register = truncate_chars(body, REGISTER_MAX_CHARS);
    let mut labels = Vec::with_capacity(m.labels.len() + 1);
    labels.push(m.role.as_slug().to_string());
    labels.extend(m.labels);
    AssetSpec {
        source_kind: m.source.kind,
        locator: m.source.locator,
        // Containment is `external_session_key` below; this is the
        // semantic half — what the row *is*. V38 removed the old
        // `dialogue` slug for conflating the two and left messages
        // unclassified, which stopped being tenable once every Asset
        // became a Card: an unclassified row is one no facet reaches
        // (V43).
        modality: Some("message".into()),
        occurred_at: m.occurred_at,
        // Importers never know the server-side `Session.id` — they
        // hand the raw key through `external_session_key` and let
        // the server resolve it via `find_or_create_by_external_key`.
        session_id: None,
        external_session_key: Some(m.external_session_key),
        external_key: m.source.external_id,
        labels,
        register_note: Some(register),
        platform: m.source.platform,
        file_size_bytes: None,
        duration_ms: None,
        // A line in a conversation log has no pixels to measure.
        width_px: None,
        height_px: None,
        bundle_id: None,
        extra_json: Some(m.extra.to_string()),
        cover_hint: Some(cover),
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        album_meta: Default::default(),
        declared_content_hash: None,
    }
}

fn doc_to_spec(d: Doc) -> AssetSpec {
    let cover = truncate_chars(d.excerpt.trim(), COVER_MAX_CHARS);
    let mut labels = Vec::with_capacity(d.labels.len() + 1);
    labels.push(d.format.as_label());
    labels.extend(d.labels);
    AssetSpec {
        source_kind: d.source.kind,
        locator: d.source.locator,
        modality: Some("work_product".into()),
        occurred_at: d.occurred_at,
        session_id: None,
        external_session_key: None,
        external_key: d.source.external_id,
        labels,
        register_note: d
            .title
            .as_deref()
            .map(|t| truncate_chars(t, REGISTER_MAX_CHARS)),
        platform: d.source.platform,
        file_size_bytes: d.file_size_bytes,
        duration_ms: None,
        // A written work product has no pixel canvas of its own — a PDF
        // has pages and a page has a size, which is not this column.
        width_px: None,
        height_px: None,
        bundle_id: d.bundle_id,
        extra_json: Some(d.extra.to_string()),
        cover_hint: Some(cover),
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        album_meta: Default::default(),
        declared_content_hash: None,
    }
}

fn note_to_spec(n: Note) -> AssetSpec {
    let body = n.body.trim();
    let cover = truncate_chars(body, COVER_MAX_CHARS);
    let register = truncate_chars(body, REGISTER_MAX_CHARS);
    AssetSpec {
        source_kind: n.source.kind,
        locator: n.source.locator,
        modality: Some("memory".into()),
        occurred_at: n.occurred_at,
        session_id: None,
        external_session_key: None,
        external_key: n.source.external_id,
        labels: n.labels,
        register_note: Some(register),
        platform: n.source.platform.or_else(|| n.source_app.clone()),
        file_size_bytes: None,
        duration_ms: None,
        // Text capture; nothing to measure.
        width_px: None,
        height_px: None,
        bundle_id: n.bundle_id,
        extra_json: Some(n.extra.to_string()),
        cover_hint: Some(cover),
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        album_meta: Default::default(),
        declared_content_hash: None,
    }
}

fn journal_to_spec(j: JournalEntry) -> AssetSpec {
    let body = j.body.trim();
    let cover = truncate_chars(body, COVER_MAX_CHARS);
    let register = truncate_chars(body, REGISTER_MAX_CHARS);
    AssetSpec {
        source_kind: j.source.kind,
        locator: j.source.locator,
        modality: Some(j.kind.as_modality_slug().to_string()),
        occurred_at: j.occurred_at,
        session_id: None,
        external_session_key: None,
        external_key: j.source.external_id,
        labels: j.labels,
        register_note: Some(register),
        platform: j.source.platform,
        file_size_bytes: None,
        duration_ms: None,
        // Text capture; nothing to measure.
        width_px: None,
        height_px: None,
        bundle_id: j.bundle_id,
        extra_json: Some(j.extra.to_string()),
        cover_hint: Some(cover),
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        album_meta: Default::default(),
        declared_content_hash: None,
    }
}

fn image_to_spec(i: Image) -> AssetSpec {
    let cover = i
        .alt
        .as_deref()
        .map(|s| truncate_chars(s.trim(), COVER_MAX_CHARS));
    let register = i
        .alt
        .as_deref()
        .map(|s| truncate_chars(s.trim(), REGISTER_MAX_CHARS));
    AssetSpec {
        source_kind: i.source.kind,
        locator: i.source.locator,
        // Unclassified (asset-model v4): "is an image" is a data
        // format, not a semantic classification — the server captures
        // it on the material layer from the locator.
        modality: None,
        occurred_at: i.occurred_at,
        session_id: None,
        external_session_key: i.external_session_key,
        external_key: i.source.external_id,
        labels: i.labels,
        register_note: register,
        platform: i.source.platform,
        file_size_bytes: i.file_size_bytes,
        duration_ms: None,
        // The pixel dimensions of the stored bytes, **not** of what a
        // viewer shows: the orientation the parser read is put in `extra`
        // and never applied here, so an Orientation 5-8 photo travels as
        // the landscape pair it is encoded as.
        //
        // Both halves come off the one `Option` the parser filled, so the
        // pair cannot be written half-way by this road — the invariant is
        // the shape of the expression rather than a check after it.
        width_px: i.dims.map(|(width, _)| width),
        height_px: i.dims.map(|(_, height)| height),
        bundle_id: i.bundle_id,
        extra_json: Some(i.extra.to_string()),
        cover_hint: cover,
        derived_from: i.derived_from,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        album_meta: i.album_meta,
        declared_content_hash: None,
    }
}

fn video_to_spec(v: Video) -> AssetSpec {
    let cover = v
        .alt
        .as_deref()
        .map(|s| truncate_chars(s.trim(), COVER_MAX_CHARS));
    let register = v
        .alt
        .as_deref()
        .map(|s| truncate_chars(s.trim(), REGISTER_MAX_CHARS));
    let mut labels = v.labels;
    if let Some(c) = &v.codec {
        labels.push(format!("codec:{c}"));
    }
    AssetSpec {
        source_kind: v.source.kind,
        locator: v.source.locator,
        // Format, not classification (see the Image arm).
        modality: None,
        occurred_at: v.occurred_at,
        session_id: None,
        external_session_key: None,
        external_key: v.source.external_id,
        labels,
        register_note: register,
        platform: v.source.platform,
        file_size_bytes: v.file_size_bytes,
        duration_ms: v.duration_ms,
        // Container **coded** dimensions, on the same pairing as the
        // Image arm. Worse off than that arm, and knowingly: the
        // rotation an mp4 `tkhd` display matrix carries, Matroska's
        // `DisplayWidth` / `DisplayHeight`, and a non-square pixel
        // aspect are all measured by nothing on this road, so a video
        // shot upright records 1920x1080 and no reader — here or later —
        // can recover what it displays as.
        width_px: v.dims.map(|(width, _)| width),
        height_px: v.dims.map(|(_, height)| height),
        bundle_id: v.bundle_id,
        extra_json: Some(v.extra.to_string()),
        cover_hint: cover,
        derived_from: v.derived_from,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        album_meta: v.album_meta,
        declared_content_hash: None,
    }
}

fn audio_to_spec(a: Audio) -> AssetSpec {
    let cover = a
        .alt
        .as_deref()
        .map(|s| truncate_chars(s.trim(), COVER_MAX_CHARS));
    let register = a
        .alt
        .as_deref()
        .map(|s| truncate_chars(s.trim(), REGISTER_MAX_CHARS));
    let mut labels = a.labels;
    if let Some(c) = &a.codec {
        labels.push(format!("codec:{c}"));
    }
    AssetSpec {
        source_kind: a.source.kind,
        locator: a.source.locator,
        // Format, not classification (see the Image arm).
        modality: None,
        occurred_at: a.occurred_at,
        session_id: None,
        external_session_key: None,
        external_key: a.source.external_id,
        labels,
        register_note: register,
        platform: a.source.platform,
        file_size_bytes: a.file_size_bytes,
        duration_ms: a.duration_ms,
        // Audio has a length, not a frame.
        width_px: None,
        height_px: None,
        bundle_id: a.bundle_id,
        extra_json: Some(a.extra.to_string()),
        cover_hint: cover,
        derived_from: a.derived_from,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        album_meta: a.album_meta,
        declared_content_hash: None,
    }
}

fn tape_to_spec(t: Tape) -> AssetSpec {
    let cover = truncate_chars(t.excerpt.trim(), COVER_MAX_CHARS);
    let register = t
        .title
        .as_deref()
        .map(|s| truncate_chars(s.trim(), REGISTER_MAX_CHARS));
    AssetSpec {
        source_kind: t.source.kind,
        locator: t.source.locator,
        modality: Some("tape".into()),
        occurred_at: t.occurred_at,
        session_id: None,
        external_session_key: None,
        external_key: t.source.external_id,
        labels: t.labels,
        register_note: register,
        platform: t.source.platform,
        file_size_bytes: t.file_size_bytes,
        duration_ms: None,
        // A transcript is text.
        width_px: None,
        height_px: None,
        bundle_id: t.bundle_id,
        extra_json: Some(t.extra.to_string()),
        cover_hint: Some(cover),
        derived_from: None,
        author_kind: None,
        author_subject: None,
        operator_ai: None,
        album_meta: Default::default(),
        declared_content_hash: None,
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stub_source() -> FootprintSource {
        FootprintSource {
            kind: "test".into(),
            locator: "file.jsonl#msg-1".into(),
            platform: Some("Test Platform".into()),
            external_id: None,
        }
    }

    #[test]
    fn chat_message_produces_dialogue_asset_with_role_label() {
        let fp = Footprint::ChatMessage(ChatMessage {
            source: stub_source(),
            occurred_at: Utc::now(),
            external_session_key: "s-1".into(),
            role: ChatRole::User,
            body: "hello world".into(),
            thread_position: Some(3),
            parent_message_id: None,
            labels: vec!["extra".into()],
            extra: json!({"foo": 1}),
        });
        let spec = fp.into_asset_spec();
        assert_eq!(
            spec.modality.as_deref(),
            Some("message"),
            "a message says what it is; containment is external_session_key"
        );
        assert_eq!(spec.session_id, None);
        assert_eq!(spec.external_session_key.as_deref(), Some("s-1"));
        assert_eq!(spec.labels, vec!["user", "extra"]);
        assert_eq!(spec.cover_hint.as_deref(), Some("hello world"));
        assert_eq!(spec.register_note.as_deref(), Some("hello world"));
        assert!(spec.extra_json.unwrap().contains("\"foo\":1"));
    }

    #[test]
    fn doc_format_slugs_are_stable() {
        // Guard against silent slug drift — the labels below flow into
        // the DB and get user-visible facet chips, so renaming them is
        // a UX-breaking change even while the API is unstable.
        assert_eq!(DocFormat::Markdown.as_label(), "markdown");
        assert_eq!(DocFormat::Html.as_label(), "html");
        assert_eq!(DocFormat::Code("rust".into()).as_label(), "code:rust");
        assert_eq!(DocFormat::TermLog.as_label(), "term-log");
        assert_eq!(DocFormat::TermVhs.as_label(), "term-vhs");
        assert_eq!(
            DocFormat::Other("observable".into()).as_label(),
            "observable"
        );
    }

    #[test]
    fn doc_uses_format_as_label_and_title_as_register() {
        let fp = Footprint::Doc(Doc {
            source: stub_source(),
            occurred_at: Utc::now(),
            title: Some("My Doc".into()),
            excerpt: "abstract goes here".into(),
            format: DocFormat::Markdown,
            bundle_id: None,
            file_size_bytes: Some(42),
            word_count: None,
            labels: vec![],
            extra: json!({}),
        });
        let spec = fp.into_asset_spec();
        assert_eq!(spec.modality.as_deref(), Some("work_product"));
        assert_eq!(spec.labels, vec!["markdown"]);
        assert_eq!(spec.register_note.as_deref(), Some("My Doc"));
        assert_eq!(spec.cover_hint.as_deref(), Some("abstract goes here"));
    }

    #[test]
    fn note_maps_to_memory_and_prefers_platform_over_source_app() {
        let fp = Footprint::Note(Note {
            source: FootprintSource {
                kind: "notes".into(),
                locator: "id-42".into(),
                platform: None,
                external_id: None,
            },
            occurred_at: Utc::now(),
            body: "quick thought".into(),
            source_app: Some("Apple Notes".into()),
            labels: vec![],
            bundle_id: None,
            extra: json!({}),
        });
        let spec = fp.into_asset_spec();
        assert_eq!(spec.modality.as_deref(), Some("memory"));
        assert_eq!(spec.platform.as_deref(), Some("Apple Notes"));
    }

    #[test]
    fn note_bundle_id_flows_to_spec() {
        let fp = Footprint::Note(Note {
            source: FootprintSource {
                kind: "png-text".into(),
                locator: "/img.png#prompt".into(),
                platform: None,
                external_id: None,
            },
            occurred_at: Utc::now(),
            body: "1girl, solo".into(),
            source_app: None,
            labels: vec![],
            bundle_id: Some("sid-xyz".into()),
            extra: json!({}),
        });
        let spec = fp.into_asset_spec();
        assert_eq!(spec.bundle_id.as_deref(), Some("sid-xyz"));
        assert_eq!(spec.session_id, None);
        assert_eq!(spec.external_session_key, None);
    }

    #[test]
    fn image_bundle_id_flows_to_spec() {
        let fp = Footprint::Image(Image {
            source: FootprintSource {
                kind: "fs".into(),
                locator: "/img.png".into(),
                platform: None,
                external_id: None,
            },
            occurred_at: Utc::now(),
            external_session_key: None,
            alt: Some("stub".into()),
            dims: None,
            file_size_bytes: None,
            labels: vec![],
            bundle_id: Some("sid-xyz".into()),
            extra: json!({}),
            derived_from: None,
            album_meta: Default::default(),
        });
        let spec = fp.into_asset_spec();
        assert_eq!(spec.bundle_id.as_deref(), Some("sid-xyz"));
        assert_eq!(spec.session_id, None);
        assert_eq!(
            spec.modality, None,
            "media format is not a classification (v4)"
        );
    }

    // ---- coded pixel dimensions ----------------------------------
    //
    // The fixture width and height **must differ**. A square one passes
    // a transposed assignment on both sides of every hop below, which is
    // the one mistake two independent `Option<u32>` fields cannot be
    // stopped from making by the type system.

    /// Width of every dimensions fixture here.
    const FIXTURE_W: u32 = 1920;
    /// Height of every dimensions fixture here — deliberately not
    /// `FIXTURE_W`.
    const FIXTURE_H: u32 = 1080;

    fn image_measuring(dims: Option<(u32, u32)>) -> Footprint {
        Footprint::Image(Image {
            source: FootprintSource {
                kind: "fs".into(),
                locator: "/pics/shot.jpg".into(),
                platform: None,
                external_id: None,
            },
            occurred_at: Utc::now(),
            external_session_key: None,
            alt: None,
            dims,
            file_size_bytes: None,
            labels: vec![],
            bundle_id: None,
            extra: json!({}),
            derived_from: None,
            album_meta: Default::default(),
        })
    }

    fn video_measuring(dims: Option<(u32, u32)>) -> Footprint {
        Footprint::Video(Video {
            source: FootprintSource {
                kind: "fs".into(),
                locator: "/clips/take.mp4".into(),
                platform: None,
                external_id: None,
            },
            occurred_at: Utc::now(),
            alt: None,
            dims,
            duration_ms: Some(4_000),
            file_size_bytes: None,
            codec: None,
            framerate: None,
            labels: vec![],
            bundle_id: None,
            extra: json!({}),
            derived_from: None,
            album_meta: Default::default(),
        })
    }

    /// What the parser measured survives **both** hops — `Footprint` →
    /// `AssetSpec` → `AddAssetCommand`.
    ///
    /// The second hop is the one no other assertion in this change
    /// crosses, and it is a hand-written field-by-field literal
    /// (`mapper::spec_to_command`), so a transposed or dropped copy there
    /// is invisible to a test that stops at the spec. This is also the
    /// pair of hops every SDK-driven importer runs on (`runner.rs` calls
    /// `spec_to_command(fp.into_asset_spec(), …)`).
    #[test]
    fn measured_dimensions_survive_both_hops_to_the_wire() {
        assert_ne!(
            FIXTURE_W, FIXTURE_H,
            "a square fixture would pass a transposed copy"
        );
        for (what, footprint) in [
            ("image", image_measuring(Some((FIXTURE_W, FIXTURE_H)))),
            ("video", video_measuring(Some((FIXTURE_W, FIXTURE_H)))),
        ] {
            let spec = footprint.into_asset_spec();
            assert_eq!(spec.width_px, Some(FIXTURE_W), "{what} spec width");
            assert_eq!(spec.height_px, Some(FIXTURE_H), "{what} spec height");

            let command = crate::mapper::spec_to_command(spec, "p-1");
            assert_eq!(command.width_px, Some(FIXTURE_W), "{what} wire width");
            assert_eq!(command.height_px, Some(FIXTURE_H), "{what} wire height");
        }
    }

    /// A parser that could not measure states nothing, all the way to
    /// the wire — not a zero, which would read as a measurement and sort
    /// like one.
    #[test]
    fn an_unmeasured_footprint_states_no_dimensions_at_all() {
        for (what, footprint) in [
            ("image", image_measuring(None)),
            ("video", video_measuring(None)),
        ] {
            let spec = footprint.into_asset_spec();
            assert_eq!((spec.width_px, spec.height_px), (None, None), "{what} spec");
            let command = crate::mapper::spec_to_command(spec, "p-1");
            assert_eq!(
                (command.width_px, command.height_px),
                (None, None),
                "{what} wire"
            );
        }
    }

    /// **No variant can state half a resolution.**
    ///
    /// This is the importer half of the pair invariant the server also
    /// enforces (`AssetService::add` refuses a half). It holds here by
    /// construction rather than by a check — every variant that has
    /// dimensions holds them as one `Option<(u32, u32)>` — and this is
    /// what says the construction was not undone.
    ///
    /// All eight variants, and the two that can measure appear twice:
    /// once measured, once not. Without the measured pair the assertion
    /// would be six `None == None` comparisons and would pass over an
    /// arm that wrote only the width.
    #[test]
    fn no_footprint_variant_states_half_a_resolution() {
        let chat = Footprint::ChatMessage(ChatMessage {
            source: stub_source(),
            occurred_at: Utc::now(),
            external_session_key: "s-1".into(),
            role: ChatRole::User,
            body: "hello".into(),
            thread_position: None,
            parent_message_id: None,
            labels: vec![],
            extra: json!({}),
        });
        let doc = Footprint::Doc(Doc {
            source: stub_source(),
            occurred_at: Utc::now(),
            title: None,
            excerpt: "an excerpt".into(),
            format: DocFormat::Markdown,
            bundle_id: None,
            file_size_bytes: None,
            word_count: None,
            labels: vec![],
            extra: json!({}),
        });
        let note = Footprint::Note(Note {
            source: stub_source(),
            occurred_at: Utc::now(),
            body: "a note".into(),
            source_app: None,
            labels: vec![],
            bundle_id: None,
            extra: json!({}),
        });
        let journal = Footprint::JournalEntry(JournalEntry {
            source: stub_source(),
            occurred_at: Utc::now(),
            kind: JournalKind::State,
            body: "a state".into(),
            bundle_id: None,
            labels: vec![],
            extra: json!({}),
        });
        let audio = Footprint::Audio(Audio {
            source: stub_source(),
            occurred_at: Utc::now(),
            alt: None,
            duration_ms: Some(1_000),
            file_size_bytes: None,
            codec: None,
            sample_rate: None,
            channels: None,
            labels: vec![],
            bundle_id: None,
            extra: json!({}),
            derived_from: None,
            album_meta: Default::default(),
        });
        let tape = Footprint::Tape(Tape {
            source: stub_source(),
            occurred_at: Utc::now(),
            title: None,
            excerpt: "a transcript".into(),
            bundle_id: None,
            file_size_bytes: None,
            labels: vec![],
            extra: json!({}),
        });

        let cases = [
            ("chat", chat),
            ("doc", doc),
            ("note", note),
            ("journal", journal),
            ("audio", audio),
            ("tape", tape),
            (
                "image measured",
                image_measuring(Some((FIXTURE_W, FIXTURE_H))),
            ),
            ("image unmeasured", image_measuring(None)),
            (
                "video measured",
                video_measuring(Some((FIXTURE_W, FIXTURE_H))),
            ),
            ("video unmeasured", video_measuring(None)),
        ];
        let mut measured = 0usize;
        for (what, footprint) in cases {
            let spec = footprint.into_asset_spec();
            assert_eq!(
                spec.width_px.is_some(),
                spec.height_px.is_some(),
                "{what} stated one dimension without the other: \
                 {:?} / {:?}",
                spec.width_px,
                spec.height_px
            );
            if spec.width_px.is_some() {
                measured += 1;
            }
        }
        assert_eq!(
            measured, 2,
            "two of the fixtures have to arrive measured, or this is a \
             row of None == None comparisons"
        );
    }

    #[test]
    fn cover_and_register_truncate_at_char_boundaries() {
        let long = "a".repeat(500);
        let fp = Footprint::ChatMessage(ChatMessage {
            source: stub_source(),
            occurred_at: Utc::now(),
            external_session_key: "s".into(),
            role: ChatRole::Assistant,
            body: long,
            thread_position: None,
            parent_message_id: None,
            labels: vec![],
            extra: json!({}),
        });
        let spec = fp.into_asset_spec();
        assert_eq!(spec.cover_hint.unwrap().chars().count(), COVER_MAX_CHARS);
        assert_eq!(
            spec.register_note.unwrap().chars().count(),
            REGISTER_MAX_CHARS
        );
    }
}
