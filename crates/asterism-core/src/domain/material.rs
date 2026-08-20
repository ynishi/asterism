//! `Material` — the physical-original layer of an asset (asset-model v4).
//!
//! A material is the bytes-side fact record of one original artefact:
//! where it lives (`locator`), how big it is, and what shape its data
//! has (`mime`). It answers "what is this made of" — the question the
//! old `ContentKind` reached through the modality master, conflating
//! data format with user classification and container structure.
//!
//! Materials are aggregate-internal to [`Asset`](crate::domain::asset::Asset):
//! identified by `(owning asset, ord)` — the PhotoKit `PHAssetResource`
//! shape — and never referenced from outside the aggregate. An asset
//! with [`AssetRole::Collection`](crate::domain::value::AssetRole)
//! carries no materials: a container has no bytes of its own, its
//! content is its members.

use std::collections::BTreeMap;

use crate::domain::axis_status::AxisStatus;
use crate::domain::source_locator::SourceLocator;
use crate::domain::value::{AudioFormat, ImageFormat, MimeType, VideoFormat};
use chrono::{DateTime, Utc};

/// One physical original belonging to an asset.
///
/// Invariants:
///
/// - Asterism never writes back to `locator` (the same contract as
///   [`SourceRef`](crate::domain::value::SourceRef)).
/// - P1–P4 operate at exactly one material (`ord == 0`) per item; the
///   `ord` axis is schema room for the RAW+JPEG / Live Photo pattern
///   (1 logical unit : N original resources).
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    /// Position within the owning asset (`0` = primary original).
    pub ord: u32,
    /// Where the original artefact is, taken apart — the same type
    /// [`SourceRef::locator`](crate::domain::value::SourceRef::locator)
    /// carries, and today a denormalised copy of it.
    ///
    /// Typed here rather than left as text precisely *because* it is the
    /// copy. This is the layer where addresses are designed to diverge
    /// (one asset, several original resources), so leaving the string
    /// convention alive here would make it the last place a `#` means
    /// something, at the moment that stops being safe.
    pub locator: SourceLocator,
    /// Size of the original artefact in bytes, when known.
    pub file_size_bytes: Option<u64>,
    /// Data-format fact, parsed. `None` means "unknown", never "not
    /// applicable" — a material always has a format, it just may not
    /// have been captured yet.
    ///
    /// Parsed rather than a string so that the questions asked of it
    /// ("does this tile?", "is this text?") are `match`es instead of
    /// `starts_with` calls each site has to remember to write; see
    /// [`MimeType`] for the two defects that cost.
    pub mime: Option<MimeType>,
    /// Fingerprint of **every byte** of the original
    /// (`crate::domain::content_hash`) — a digest and nothing else, or
    /// `None` when there is no digest.
    /// [`content_hash_status`](Self::content_hash_status) beside it
    /// says why: not computed yet, no bytes to read, a read that
    /// failed.
    ///
    /// `None` means "unknown", never "unique": two materials without a
    /// hash are not duplicates of each other, they are two questions
    /// nobody has answered yet.
    ///
    /// **The name is older than the second axis and now under-describes
    /// it.** This field is the *file* axis; the one named for the
    /// content axis is [`content_region_hash`](Self::content_region_hash)
    /// beside it. Renaming this one to `file_hash` is the honest end
    /// state and is deliberately not done here: the word `content_hash`
    /// is the wire name on the asset DTO, the MCP and HTTP payloads, the
    /// TypeScript bindings and the duplicates panel, so moving it would
    /// put a rename of every surface in the same diff as the axis it is
    /// being renamed to make room for, and neither would be reviewable.
    /// The mitigation is that the two fields name each other.
    pub content_hash: Option<String>,
    /// Why [`content_hash`](Self::content_hash) holds what it holds —
    /// the file axis's status column.
    pub content_hash_status: AxisStatus,
    /// The status's free-text payload, when it carries one: the I/O
    /// error under [`Failed`](AxisStatus::Failed). `None` otherwise.
    pub content_hash_reason: Option<String>,
    /// Fingerprint of only the bytes that decide what the original
    /// decodes to (`crate::domain::content_region`) — a digest and
    /// nothing else.
    /// [`content_region_hash_status`](Self::content_region_hash_status)
    /// says why there is none: no walker handles the format, the walk
    /// found nothing, the deferred migration never opened the file.
    ///
    /// The point of the axis: two exports of one picture that differ
    /// only in a `tEXt` chunk — a workflow blob, an exporter's
    /// timestamp — are different *files* and the same *picture*, and
    /// [`content_hash`](Self::content_hash) can only see the first of
    /// those.
    ///
    /// The axes are filled in together: one read of the file produces
    /// all of them, and one statement writes all of them, so a row with
    /// one axis answered and another pending is a row from a build that
    /// predated the newer column.
    pub content_region_hash: Option<String>,
    /// Why [`content_region_hash`](Self::content_region_hash) holds
    /// what it holds — the content axis's status column.
    ///
    /// A material that predates the column is answered by the migration
    /// that adds it, in two steps:
    /// [`NotWalked`](AxisStatus::NotWalked) first — an answer, so the
    /// ordinary walk leaves the row alone — and then the real value,
    /// computed by the step that reads the file. A row still carrying
    /// that status afterwards is one whose original could not be
    /// opened.
    pub content_region_hash_status: AxisStatus,
    /// The status's free-text payload, when it carries one: the
    /// format's name under [`Unsupported`](AxisStatus::Unsupported),
    /// the I/O error under [`Failed`](AxisStatus::Failed).
    pub content_region_hash_reason: Option<String>,
    /// Fingerprint of the metadata the container carries *about* these
    /// bytes (`crate::domain::material_meta`) — a digest and nothing
    /// else, with [`meta_hash_status`](Self::meta_hash_status) beside
    /// it saying why there is none, in the same vocabulary
    /// [`content_region_hash_status`](Self::content_region_hash_status)
    /// uses.
    ///
    /// The complement of the one above: `content_region` is defined as
    /// the bytes that survive into the decoded result, so what it drops
    /// is exactly what this measures. Two frames off one workflow that
    /// differ only by a seed are the same *making* and different
    /// pictures, and neither of the other two columns can see that.
    ///
    /// Home is `material` rather than `asset` because the metadata is a
    /// fact about *these bytes*: a RAW and its JPEG carry different
    /// embedded metadata and must not be made to share one answer,
    /// which is what the `ord` axis is held open for.
    pub meta_hash: Option<String>,
    /// Why [`meta_hash`](Self::meta_hash) holds what it holds — the
    /// meta axis's status column.
    pub meta_hash_status: AxisStatus,
    /// The status's free-text payload, when it carries one — same
    /// contract as
    /// [`content_region_hash_reason`](Self::content_region_hash_reason).
    pub meta_hash_reason: Option<String>,
    /// The canonical metadata object the digest beside it was taken
    /// over — a JSON object, keys sorted, no whitespace, values exactly
    /// as the container stated them.
    ///
    /// Stored as well as hashed because exact equality is the wrong
    /// question for metadata on its own: a batch off one workflow
    /// differs by a seed, and a digest over the whole of it separates
    /// precisely the rows that belong together. The hash answers "made
    /// identically" cheaply and indexably; the useful question is a
    /// comparison over this.
    ///
    /// `Some` exactly when [`meta_hash`](Self::meta_hash) is a digest.
    /// A marker has no object, and writing `{}` for one would say the
    /// container was read and carried nothing.
    pub meta_kv: Option<String>,
    /// The words the container wrote into these bytes, recovered for
    /// search (`crate::domain::embedded_text`) — the same canonical
    /// object shape [`meta_kv`](Self::meta_kv) holds, so one reader
    /// walks both.
    ///
    /// Its sibling above is the body of a digest and cannot move: two
    /// equal metadata sets must render identically or the meta axis
    /// stops grouping, which is why that reading is frozen at `tEXt`
    /// read lossily. This column is a document. It carries the `zTXt`
    /// and `iTXt` chunks that reading excludes, and the Latin-1 bytes
    /// it replaces with U+FFFD — the sentences a person can see in
    /// their image viewer and could not, until this column, find by
    /// searching for.
    ///
    /// **`{}` is an answer here, and that is the difference from
    /// `meta_kv`.** The three states are `NULL` (nobody has looked),
    /// `{}` (looked, and these bytes carry no words) and an object.
    /// Without the middle one every text-free PNG in the library is a
    /// candidate for every backfill pass, forever.
    pub meta_text: Option<String>,
    /// When this material record was created.
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Material {
    /// Builds the primary (`ord == 0`) material for an item from its
    /// source facts, guessing the mime from the locator.
    pub fn primary(
        locator: SourceLocator,
        file_size_bytes: Option<u64>,
        now: DateTime<Utc>,
    ) -> Self {
        let mime = guess_mime(&locator);
        Self {
            ord: 0,
            locator,
            file_size_bytes,
            mime,
            // Hashing reads the whole file; the ingest path must not.
            // The `material_hash` job fills all of these in afterwards,
            // from one read.
            content_hash: None,
            content_hash_status: AxisStatus::Pending,
            content_hash_reason: None,
            content_region_hash: None,
            content_region_hash_status: AxisStatus::Pending,
            content_region_hash_reason: None,
            meta_hash: None,
            meta_hash_status: AxisStatus::Pending,
            meta_hash_reason: None,
            meta_kv: None,
            meta_text: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// The metadata this material carries, taken apart — or `None` when
    /// no walk produced an object (a marker, or nothing written yet).
    ///
    /// The entry point for the comparison the axis exists for: "made
    /// the same way apart from *this*" is a walk over two of these,
    /// not an equality test on two digests. Parsed on read from
    /// [`meta_kv`](Self::meta_kv) rather than held as a map, so the
    /// stored form stays the one authority — a second in-memory copy
    /// is a second thing to keep in step, and the copy that drifts is
    /// always the one nobody thought was authoritative.
    ///
    /// A column this cannot parse answers `None`: the digest is still
    /// true of whatever was hashed, and a fabricated empty map would
    /// read as "the container carried nothing".
    pub fn meta_fields(&self) -> Option<BTreeMap<String, String>> {
        serde_json::from_str(self.meta_kv.as_deref()?).ok()
    }
}

/// Every `image/*` value [`guess_mime`] can produce.
///
/// Same tripwire contract as [`KNOWN_VIDEO_MIMES`]: the importer
/// scanned these extensions long before the mime map knew them all,
/// and a format the map does not name gets `mime: None` → no
/// thumbnail, no image rendering — imported but invisible. The infra
/// side holds a test that walks this list against the thumbnailer's
/// real capabilities.
pub const KNOWN_IMAGE_MIMES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/heic",
    "image/heif",
    "image/avif",
    "image/tiff",
    "image/bmp",
];

/// Every `video/*` value [`guess_mime`] can produce.
///
/// Exists for the thumbnail pipeline's routing tripwire: the mime map
/// (here) and the frame extractor's real capabilities (infra) are two
/// lists that drift apart silently — a format added here without a
/// deliberate extraction route shows up as an empty tile, not an
/// error. The infra side holds a test that walks this list and fails
/// the moment an entry has no explicit route.
pub const KNOWN_VIDEO_MIMES: &[&str] = &[
    "video/mp4",
    "video/quicktime",
    "video/webm",
    "video/x-matroska",
    "video/x-msvideo",
];

/// Best-effort mime guess from a locator's file extension.
///
/// A [`Record`](SourceLocator::Record) addresses one record *inside* a
/// container, and what the asset stands for is that record's extracted
/// text — never the container's format. Every record locator is
/// therefore `text/plain`, whatever the container extension says.
/// Reading the extension through the record address is what filed a PNG
/// tEXt note as `image/png` and sent the thumbnailer off to open
/// `shot.png#workflow` as a path, which no filesystem has. It is now the
/// variant that answers, not a `contains('#')` test each caller had to
/// remember to write.
///
/// The other three shapes are sniffed over the part of them that can
/// carry an extension, and each asks for it the way its own shape
/// warrants:
///
/// - a [`File`](SourceLocator::File) is a path, so
///   [`Path::extension`](std::path::Path::extension) answers;
/// - a [`Remote`](SourceLocator::Remote) is addressed by a scheme this
///   code does not parse, so its target is read as text with a query
///   string stripped — a strip that used to apply to every locator and
///   is now justified by the one variant that can have a query;
/// - a [`Logical`](SourceLocator::Logical) is a caller-minted name, read
///   the same way, since a name is not a path and `/` in it is the
///   caller's convention rather than a directory separator.
///
/// Textual source containers (`.jsonl` / `.json` / `.md` / `.txt` /
/// `.db`) map to `text/plain` for the same reason even when addressed
/// whole. Unknown extensions return `None` — an unknown fact, refined
/// later, never a fabricated one.
pub fn guess_mime(locator: &SourceLocator) -> Option<MimeType> {
    /// The textual sniff, for the two shapes that are not paths.
    fn extension_of_text(raw: &str) -> Option<&str> {
        let path = raw.split('?').next().unwrap_or(raw);
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        name.rsplit_once('.').map(|(_, ext)| ext)
    }

    let ext = match locator {
        // The record is the artefact; the container's extension answers
        // for the wrong thing.
        SourceLocator::Record(_) => return Some(MimeType::text_plain()),
        SourceLocator::File(path) => path.as_path().extension()?.to_str()?,
        SourceLocator::Remote(remote) => extension_of_text(remote.target().as_str())?,
        SourceLocator::Logical(name) => extension_of_text(name.as_str())?,
    };
    let mime = match ext.to_ascii_lowercase().as_str() {
        "png" => MimeType::Image(ImageFormat::Png),
        "jpg" | "jpeg" => MimeType::Image(ImageFormat::Jpeg),
        "gif" => MimeType::Image(ImageFormat::Gif),
        "webp" => MimeType::Image(ImageFormat::Webp),
        "heic" => MimeType::Image(ImageFormat::Heic),
        "heif" => MimeType::Image(ImageFormat::Heif),
        "avif" => MimeType::Image(ImageFormat::Avif),
        "tiff" | "tif" => MimeType::Image(ImageFormat::Tiff),
        "bmp" => MimeType::Image(ImageFormat::Bmp),
        "mp4" | "m4v" => MimeType::Video(VideoFormat::Mp4),
        "mov" => MimeType::Video(VideoFormat::Quicktime),
        "webm" => MimeType::Video(VideoFormat::Webm),
        "mkv" => MimeType::Video(VideoFormat::Matroska),
        "avi" => MimeType::Video(VideoFormat::Msvideo),
        "mp3" => MimeType::Audio(AudioFormat::Mpeg),
        "wav" => MimeType::Audio(AudioFormat::Wav),
        "m4a" => MimeType::Audio(AudioFormat::Mp4),
        "jsonl" | "json" | "md" | "txt" | "db" => MimeType::text_plain(),
        _ => return None,
    };
    Some(mime)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every fixture below is written the way a *caller* spells a
    /// locator, and reaches `guess_mime` through the boundary that reads
    /// that spelling — so a test that stopped exercising the variant it
    /// names would stop compiling rather than stop meaning anything.
    fn loc(raw: &str) -> SourceLocator {
        SourceLocator::from_wire(raw).expect("locator")
    }

    #[test]
    fn guess_mime_reads_the_extension_case_insensitively() {
        assert_eq!(
            guess_mime(&loc("/pics/Star.PNG")),
            Some(MimeType::Image(ImageFormat::Png))
        );
        assert_eq!(
            guess_mime(&loc("clip.MoV")),
            Some(MimeType::Video(VideoFormat::Quicktime))
        );
    }

    #[test]
    fn a_fragment_locator_is_the_record_not_the_container() {
        assert_eq!(
            guess_mime(&loc("/logs/session.jsonl#0198c1c2-aaaa")),
            Some(MimeType::text_plain())
        );
        assert_eq!(
            guess_mime(&loc("/store/tapes.db#0198c1c2-bbbb")),
            Some(MimeType::text_plain())
        );
        // The container extension is a visual format and the payload
        // still is not: a PNG tEXt chunk is text living inside an
        // image. Answering `image/png` here is what handed the
        // thumbnailer a locator no filesystem can open — and, once the
        // format stopped being a string, what would have put the
        // container's bytes through the full-text reader.
        assert_eq!(
            guess_mime(&loc("/pics/shot.png#workflow")),
            Some(MimeType::text_plain())
        );
        assert_eq!(
            guess_mime(&loc("/clips/reel.mp4#chapter-2")),
            Some(MimeType::text_plain())
        );
        // A container path carrying a `#` of its own is still a record,
        // and still answers for the record. The string test this
        // replaced could not tell the two `#`s apart at all.
        assert_eq!(
            guess_mime(&loc("/pics/a#b.png#note-1")),
            Some(MimeType::text_plain())
        );
    }

    /// A `#` is an ordinary character in a **stored** locator, and stays
    /// ambiguous in the one place a locator is still a delimited string:
    /// the spelling a caller sends.
    ///
    /// **What the storage-form step settled.** The columns hold the
    /// tagged object, so nothing reads a `#` on the way in or out: a
    /// file whose own name carries one is a `file`, an address carrying
    /// one is an address, and both survive a round trip. The rows that
    /// were *already* stored under the delimiter were settled by the
    /// rewrite migration, which asks the filesystem whether the whole
    /// string is a file — a test no parser can run, and the reason this
    /// stopped being a carried limitation for stored data.
    ///
    /// **What remains true.** `SourceLocator::from_wire` reads the SDK's
    /// spelling, where `<container>#<record>` is the documented form for
    /// a per-record source, so `/pics/a#b.png` still arrives
    /// indistinguishable from the record `b.png` inside `/pics/a`. That
    /// is a property of the *contract*, not of the encoding, and a
    /// caller holding the pieces sidesteps it entirely
    /// (`LocalPath::try_from(path_buf)?.into()` builds the file). The
    /// split is `rsplit`, which is why the container in the first
    /// fixture keeps its own `#`.
    #[test]
    fn a_hash_in_a_container_path_survives_and_a_wire_spelling_is_still_ambiguous() {
        let record = loc("/pics/a#b.png#note-1");
        let SourceLocator::Record(inner) = &record else {
            panic!("the last `#` is the delimiter: {record:?}");
        };
        assert_eq!(inner.container().as_str(), "/pics/a#b.png");
        assert_eq!(inner.record().as_str(), "note-1");
        assert_eq!(guess_mime(&record), Some(MimeType::text_plain()));

        // Still true, and now only of the wire spelling: a PNG whose own
        // name contains a `#` reads as a record there, so it answers
        // `text/plain` and takes no thumbnail.
        let ambiguous = loc("/pics/a#b.png");
        assert!(
            matches!(ambiguous, SourceLocator::Record(_)),
            "the delimited spelling cannot tell this from a container plus a record: \
             {ambiguous:?}"
        );
        assert_eq!(guess_mime(&ambiguous), Some(MimeType::text_plain()));

        // No longer true of a *stored* value. The same path built as
        // what it is round-trips through the column as a file, and
        // answers `image/png` — which is the resolution, stated as the
        // pair that disagrees rather than as the absence of the old
        // assertion.
        let as_a_file: SourceLocator =
            crate::domain::source_locator::LocalPath::try_from("/pics/a#b.png")
                .expect("absolute")
                .into();
        assert_eq!(
            SourceLocator::try_from(as_a_file.to_storage().as_str()).expect("re-read"),
            as_a_file,
            "the character is ordinary in the tagged form"
        );
        assert_eq!(
            guess_mime(&as_a_file),
            Some(MimeType::Image(ImageFormat::Png))
        );
        assert_ne!(as_a_file, ambiguous);
    }

    /// The one shape whose answer the type changed, and deliberately:
    /// a rootless container cannot be opened by the process that reads
    /// it, so `pics/a.jsonl#uuid` is a name rather than a record — and
    /// a name ending in `.jsonl` is `text/plain` by the container rule
    /// below, while one ending in nothing at all is unknown.
    #[test]
    fn a_rootless_container_is_a_name_not_a_record() {
        let rootless = loc("pics/a.jsonl#0198c1c2");
        assert!(matches!(rootless, SourceLocator::Logical(_)));
        // Sniffed as a name: the trailing segment has no extension the
        // map knows, so the honest answer is "unknown" rather than the
        // fabricated `text/plain` the `#` test used to produce.
        assert_eq!(guess_mime(&rootless), None);
    }

    #[test]
    fn guess_mime_returns_none_for_unknown_shapes() {
        assert_eq!(guess_mime(&loc("opaque-locator")), None);
        assert_eq!(guess_mime(&loc("/dir.with.dots/plain")), None);
        assert_eq!(guess_mime(&loc("archive.xyz")), None);
    }

    /// The query strip belongs to the shapes that can carry a query.
    ///
    /// A remote's target is read as text, so `?v=2` comes off before the
    /// extension is looked for. A *file* path is read as a path, and `?`
    /// is a legal character in a POSIX filename — stripping there would
    /// take the extension off a file that has one, which is what the
    /// old locator-wide strip did.
    #[test]
    fn a_query_string_is_stripped_from_a_remote_and_not_from_a_path() {
        assert_eq!(
            guess_mime(&loc("https://host/pics/a.png?v=2")),
            Some(MimeType::Image(ImageFormat::Png))
        );
        assert_eq!(
            guess_mime(&loc("/pics/a?b.png")),
            Some(MimeType::Image(ImageFormat::Png))
        );
    }

    #[test]
    fn every_image_extension_lands_inside_the_known_list() {
        // Mirror of the video tripwire below: an image arm added to
        // `guess_mime` must appear in `KNOWN_IMAGE_MIMES`, or the
        // infra-side capability test never sees it.
        for ext in [
            "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "avif", "tiff", "tif", "bmp",
        ] {
            let mime = guess_mime(&loc(&format!("pic.{ext}"))).expect("an image extension maps");
            assert!(
                KNOWN_IMAGE_MIMES.contains(&mime.as_str()),
                "{mime} (from .{ext}) is missing from KNOWN_IMAGE_MIMES"
            );
        }
        assert_eq!(
            KNOWN_IMAGE_MIMES.len(),
            9,
            "a new entry here needs a matching guess_mime arm and a thumbnail capability check"
        );
    }

    #[test]
    fn every_video_extension_lands_inside_the_known_list() {
        // The tripwire's local half: a video arm added to `guess_mime`
        // must also appear in `KNOWN_VIDEO_MIMES`, or the infra-side
        // routing test never sees it and the drift the list exists to
        // catch reopens.
        for ext in ["mp4", "m4v", "mov", "webm", "mkv", "avi"] {
            let mime = guess_mime(&loc(&format!("clip.{ext}"))).expect("a video extension maps");
            assert!(
                KNOWN_VIDEO_MIMES.contains(&mime.as_str()),
                "{mime} (from .{ext}) is missing from KNOWN_VIDEO_MIMES"
            );
        }
        assert_eq!(
            KNOWN_VIDEO_MIMES.len(),
            5,
            "a new entry here needs a matching guess_mime arm and an infra route"
        );
    }

    #[test]
    fn primary_material_captures_facts_at_ord_zero() {
        let now = Utc::now();
        let m = Material::primary(loc("/pics/a.png"), Some(42), now);
        assert_eq!(m.ord, 0);
        assert_eq!(m.mime, Some(MimeType::Image(ImageFormat::Png)));
        assert_eq!(m.file_size_bytes, Some(42));
        assert_eq!(m.created_at, now);
    }
}
