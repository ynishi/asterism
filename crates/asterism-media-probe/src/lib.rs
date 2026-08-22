//! Header-only media measurement: what the bytes say about themselves.
//!
//! Every function here is a pure function of a byte slice, and none of
//! them decodes pixels — they read far enough into a container to expose
//! what it declares and stop. That is the whole contract, and it is what
//! lets the same code run in an importer walking a directory and in a
//! server job walking a table.
//!
//! # Why this is a crate and not a helper in one of its callers
//!
//! Two paths write the same columns and must not disagree:
//!
//! - **Ingest.** `asterism-importer-image` / `-video` measure a file as
//!   it arrives and put the result on a `Footprint`.
//! - **Backfill.** The server measures rows that predate the columns
//!   (`asset.width_px` / `height_px` landed in schema V69; every row
//!   older than it carries `NULL`).
//!
//! The importers run as separate processes and talk to the server over
//! HTTP, so neither can call the other. Before this crate the only ways
//! to give the server a measurement were to write a second
//! implementation of it, or to have the server link an importer binary's
//! crate. The first puts two definitions of "the dimensions of these
//! bytes" in the tree, told apart by nothing once both have written to
//! the same column; the second inverts the out-of-process design that
//! makes importers replaceable.
//!
//! # Coded, not displayed
//!
//! **[`coded_dims`] returns the dimensions of the stored byte stream,
//! before any orientation is applied.** A photo shot upright with EXIF
//! Orientation 6 is *stored* as a landscape frame plus a rotation flag,
//! and that is the pair returned here — the flag is reported separately
//! ([`ExifFields::orientation`]) and applying it is the caller's
//! decision. Video is weaker still: neither container probe reads the
//! MP4 display matrix or Matroska's `DisplayWidth` / `DisplayHeight`,
//! so an upright phone clip measures 1920×1080 and nothing in the
//! returned value says otherwise.
//!
//! The consequence is worth stating where the measurement is defined
//! rather than only where it is consumed: **the product of the pair is
//! invariant under that rotation and the sides are not**, which is why
//! Asterism's resolution facet is a pixel count rather than a width
//! band. A caller that compares widths is comparing storage layout.
//!
//! # Container structure, beside container headers
//!
//! [`png`] is the same contract one level down: a pure function of a
//! byte slice that reads a container's framing — where each chunk
//! begins, what the text chunks say — and stops. It is here for the
//! reason the dimension probes are: a second caller appeared. The
//! server's fingerprint pass walks a PNG's chunks to decide which bytes
//! are the picture and which are notes written about it, and that walk
//! used to live in `asterism-core::domain`, where every new format made
//! the domain layer wider. **The judgement stayed there and the parsing
//! came here** — this module reports what the bytes are and never what
//! they are worth, which is why it can be read by a consumer that has no
//! opinion about identity at all.
//!
//! [`jpeg`] is the second one, on the same terms, and the pair is what
//! says the terms were terms rather than one module's habits. It is not
//! shaped like [`png`]: a JPEG is marker segments up to its scan,
//! unframed entropy-coded bytes after it, and whatever a phone appended
//! behind its end marker, so the walk yields four kinds of element where
//! the PNG walk yields one. What the two share is the
//! contract — boundaries out, judgement left to the caller — and that is
//! the part worth having twice.

// Ungated: a grammar over text that is already out of its container
// needs no image dependency — its caller holds stored rows, not files.
pub mod a1111;

#[cfg(feature = "image")]
pub mod jpeg;

#[cfg(feature = "image")]
pub mod png;

#[cfg(feature = "image")]
mod still {
    use std::collections::BTreeMap;
    use std::fmt::Display;
    use std::fs::File;
    use std::io::{BufRead, BufReader, Cursor, Seek};
    use std::path::Path;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use chrono::{DateTime, NaiveDateTime, Utc};
    use exif::{Context, Field, In, Reader as ExifReader, Tag, Value};

    /// The EXIF fields Asterism surfaces on an asset.
    ///
    /// A small owned struct rather than a live `exif::Exif`, so callers
    /// read fields instead of learning the tag vocabulary, and so the
    /// container is parsed exactly once per file.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct ExifFields {
        /// `DateTimeOriginal`, when present and parseable.
        pub datetime_original: Option<DateTime<Utc>>,
        /// Coded pixel dimensions as EXIF states them. See the crate
        /// docs for what "coded" excludes.
        pub dims: Option<(u32, u32)>,
        /// `Make`.
        pub camera_make: Option<String>,
        /// `Model`.
        pub camera_model: Option<String>,
        /// `Orientation`, the 1–8 code. **Not applied to
        /// [`dims`](Self::dims)** — it is reported so a consumer that
        /// wants a displayed shape can compute one, and so a consumer
        /// that does not stays unaffected.
        pub orientation: Option<u32>,
    }

    /// Reads EXIF out of an image payload.
    ///
    /// `None` when the container carries none, which is the ordinary
    /// state for PNG screenshots, GIF, BMP and most generated AVIF —
    /// not an error. Callers fall back to other evidence (file mtime
    /// for the timestamp, [`dims_from_header`] for the dimensions).
    pub fn exif_fields(payload: &[u8]) -> Option<ExifFields> {
        exif_from(&mut Cursor::new(payload))
    }

    /// [`exif_fields`] over a file, without loading it.
    ///
    /// `kamadak-exif` starts by taking 4 KiB and seeks from there, so a
    /// buffered `File` reads the header region and stops — the whole
    /// point of having this beside the slice form rather than only the
    /// slice form. A caller holding the bytes already (an importer, whose
    /// `RawItem` carries the payload) should use [`exif_fields`]; a
    /// caller holding a path (the server's backfill) should use this,
    /// because `std::fs::read` on a 4 GB clip is a 4 GB allocation to
    /// answer a question the first kilobyte settles.
    pub fn exif_fields_at(path: &Path) -> Option<ExifFields> {
        exif_from(&mut BufReader::new(File::open(path).ok()?))
    }

    fn exif_from<R: BufRead + Seek>(reader: &mut R) -> Option<ExifFields> {
        let exif = ExifReader::new().read_from_container(reader).ok()?;

        let datetime_original = exif
            .get_field(Tag::DateTimeOriginal, In::PRIMARY)
            .and_then(|f| ascii_str(&f.value))
            .and_then(parse_exif_datetime);

        // `PixelXDimension` is the compressed frame's width and is what
        // a camera writes; `ImageWidth` is the TIFF-side tag and is what
        // survives some editing pipelines. Preferring the first and
        // falling back to the second covers both without deciding which
        // producer wrote the file.
        let width = exif
            .get_field(Tag::PixelXDimension, In::PRIMARY)
            .and_then(|f| f.value.get_uint(0))
            .or_else(|| {
                exif.get_field(Tag::ImageWidth, In::PRIMARY)
                    .and_then(|f| f.value.get_uint(0))
            });
        let height = exif
            .get_field(Tag::PixelYDimension, In::PRIMARY)
            .and_then(|f| f.value.get_uint(0))
            .or_else(|| {
                exif.get_field(Tag::ImageLength, In::PRIMARY)
                    .and_then(|f| f.value.get_uint(0))
            });
        // `zip`, so half a pair is no pair. A width with no height
        // measures nothing, and the asset write path refuses the shape
        // anyway.
        let dims = width.zip(height);

        let camera_make = exif
            .get_field(Tag::Make, In::PRIMARY)
            .and_then(|f| ascii_str(&f.value));
        let camera_model = exif
            .get_field(Tag::Model, In::PRIMARY)
            .and_then(|f| ascii_str(&f.value));
        let orientation = exif
            .get_field(Tag::Orientation, In::PRIMARY)
            .and_then(|f| f.value.get_uint(0));

        Some(ExifFields {
            datetime_original,
            dims,
            camera_make,
            camera_model,
            orientation,
        })
    }

    /// EXIF `Ascii` values ship as `Vec<Vec<u8>>`; join the first string
    /// and trim trailing NULs / whitespace.
    fn ascii_str(value: &Value) -> Option<String> {
        if let Value::Ascii(chunks) = value {
            let bytes = chunks.iter().flatten().copied().collect::<Vec<u8>>();
            let s = String::from_utf8(bytes).ok()?;
            let trimmed = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        } else {
            None
        }
    }

    /// EXIF `DateTimeOriginal` format: `"YYYY:MM:DD HH:MM:SS"` (local
    /// time; no timezone). We treat it as UTC — good enough for MVP; a
    /// future improvement is to consult `OffsetTimeOriginal` when
    /// present.
    fn parse_exif_datetime(raw: String) -> Option<DateTime<Utc>> {
        let naive = NaiveDateTime::parse_from_str(raw.trim(), "%Y:%m:%d %H:%M:%S").ok()?;
        Some(naive.and_utc())
    }

    /// **Every field the EXIF block states**, keyed by where it sits and
    /// numbered as the file numbers it, each value carrying the type the
    /// file gave it.
    ///
    /// The wide read, beside [`exif_fields`]'s narrow one. That struct
    /// answers five questions a caller already had — when, how big, what
    /// camera, which way up — and this answers none of them: what comes
    /// back is the block's whole vocabulary, addressed rather than named,
    /// for a consumer that decides afterwards which of it it cares about.
    ///
    /// # Why a second function and not five more fields
    ///
    /// [`ExifFields`] is read at ingest by every importer, on every
    /// image, to fill four columns. Widening it to carry the whole block
    /// would build this map for all of them and hand it to none — and it
    /// could not stay a struct anyway, since the set of fields a file
    /// carries is open (a vendor writes what it likes, and Exif 3.x adds
    /// tags this crate's version has no constant for). The two readings
    /// also differ in kind: `ExifFields` **interprets** — it parses a
    /// timestamp, prefers `PixelXDimension` over `ImageWidth`, drops a
    /// dimension with no partner — where this one interprets nothing
    /// beyond what the bytes say about themselves. Keeping them apart
    /// keeps the interpretation out of the map.
    ///
    /// # The key is a destination
    ///
    /// `exif:0x829a`, `gps:0x0001`, `ifd1:0x0100` — a namespace, then the
    /// tag number in four lowercase hex digits, and **nothing about the
    /// value**. A consumer addresses a field by writing the key down, so
    /// what a key must not contain is anything that moves when the
    /// reading of the value changes.
    ///
    /// The namespace is the IFD the field sits in, because **the same tag
    /// number means different things in different IFDs**: `0x0100` is the
    /// image width in the TIFF IFDs and a tag number GPS never assigns,
    /// and IFD1 describes the embedded thumbnail rather than the picture.
    /// A key without it would merge a photograph's dimensions with its
    /// thumbnail's. The TIFF context takes `ifd0` / `ifd1` because it has
    /// no name of its own — the number *is* its name — and the other
    /// three take theirs, suffixed with the IFD number only where that is
    /// not the primary one, so the ordinary spelling stays short.
    ///
    /// # The value describes itself
    ///
    /// `rational:1/125`, `ascii:ACME`, `undefined:SGVsbG8=`. The marker
    /// is the type the file itself carries — an IFD entry is twelve bytes
    /// of which two are the type — so this is transcription rather than a
    /// vocabulary invented here, and the thirteen names below are that
    /// field's values plus `unknown` for a code this crate's reader does
    /// not have a parser for.
    ///
    /// **It is on the value and not on the key.** Without it `1/125` is
    /// ambiguous between a rational and an ASCII tag whose text is
    /// literally `1/125`, and those are two different fields; with it in
    /// the *key* a consumer could not address a field without knowing its
    /// type, and every stored address would break the day a rendering
    /// changed.
    ///
    /// The renderings, and what each is chosen against:
    ///
    /// - **`ascii`** — UTF-8 lossy, NULs removed, which is the rule
    ///   [`png::text_fields`](crate::png::text_fields) already fixes for
    ///   the other container: a total function with one answer per input,
    ///   which is what a digest taken over the result needs. The crate
    ///   hands back a NUL-separated value as several slices and they are
    ///   concatenated, so how the value was split is not part of the
    ///   answer.
    /// - **integers** — `to_string`, comma-separated where the count is
    ///   more than one.
    /// - **`rational` / `srational`** — `num/den`, **never reduced**.
    ///   Reducing loses information the file states, and it is not a safe
    ///   loss: `0/0` and `1/0` both occur in real files (a lens whose
    ///   minimum focal length is unrecorded), and no reduction of them is
    ///   a number. A consumer that wants a quantity can divide; nothing
    ///   here can undo a division.
    /// - **`float` / `double`** — base64 of the IEEE-754 bytes,
    ///   big-endian. The specification assigns neither type to any tag,
    ///   so a file carrying one is already outside what anybody agreed;
    ///   choosing a decimal spelling for it would be choosing how many
    ///   digits a number nobody defined is worth.
    /// - **`undefined`** — base64, which is how `MakerNote` arrives.
    /// - **`unknown`** — the type code, the count and the offset the
    ///   entry pointed at, which is all the reader keeps for a type it
    ///   cannot parse. Narrower than the others on purpose: it says a
    ///   field of an unrecognised type was there and where to find it,
    ///   and the bytes themselves are recoverable from whatever kept the
    ///   block (`material.meta_raw`, on the consumer's side). Dropping
    ///   the field instead would lose the distinction between a file that
    ///   carried it and one that did not.
    ///
    /// # What it does not do
    ///
    /// No selection. Every field the block states comes back, including
    /// the ones a consumer will certainly ignore, because *which* fields
    /// are worth having is a judgement about a corpus and this crate does
    /// not make those — narrowing here would decide it for every consumer
    /// at once, and quietly.
    ///
    /// Three entries are nonetheless missing, and they are the reader's
    /// doing rather than a choice made here: `ExifIFDPointer` (`0x8769`),
    /// `GPSInfoIFDPointer` (`0x8825`) and `InteropIFDPointer` (`0xa005`)
    /// are followed into the sub-IFDs they address and never emitted as
    /// fields. Nothing is lost that a consumer could use — the value of
    /// each is a file offset, and what it pointed at arrives under its
    /// own namespace — but a consumer counting fields should know the
    /// count is of the ones with values in them.
    ///
    /// No decompression, no maker-note parsing, no tag names. `MakerNote`
    /// is one `undefined` value whatever vendor wrote it; the reading
    /// that opens it is somebody's later decision and the base64 is what
    /// leaves that decision open.
    ///
    /// `None` when the container states no EXIF at all — the ordinary
    /// state for a screenshot — and an **empty map** when it states a
    /// block with no fields in it, which are different facts. A repeated
    /// tag in one IFD collapses, last occurrence winning, the same way
    /// [`png::text_fields`](crate::png::text_fields) collapses a repeated
    /// keyword.
    pub fn exif_tags(payload: &[u8]) -> Option<BTreeMap<String, String>> {
        let exif = ExifReader::new()
            .read_from_container(&mut Cursor::new(payload))
            .ok()?;
        Some(
            exif.fields()
                .map(|field| (tag_key(field), typed_text(&field.value)))
                .collect(),
        )
    }

    /// Where a field sits, as the address a consumer writes down — see
    /// [`exif_tags`] for why the IFD is in it.
    fn tag_key(field: &Field) -> String {
        let ifd = field.ifd_num.index();
        let space = match field.tag.context() {
            Context::Tiff => format!("ifd{ifd}"),
            Context::Exif => in_ifd("exif", ifd),
            Context::Gps => in_ifd("gps", ifd),
            Context::Interop => in_ifd("interop", ifd),
            // `Context` is `#[non_exhaustive]`, so this arm is what an
            // upstream bump lands in. The variant's own name keeps such a
            // field addressable and distinct from the four above, where a
            // shared bucket would file every future context together —
            // and a field nobody can address is one nobody can exclude
            // either.
            other => in_ifd(&format!("{other:?}").to_ascii_lowercase(), ifd),
        };
        format!("{space}:0x{:04x}", field.tag.number())
    }

    /// A named context's namespace, numbered only where the IFD is not
    /// the primary one.
    fn in_ifd(space: &str, ifd: u16) -> String {
        if ifd == 0 {
            space.to_string()
        } else {
            format!("{space}{ifd}")
        }
    }

    /// One field's value, behind the name of the type the file gave it.
    ///
    /// Exhaustive over [`Value`] deliberately — the crate does not mark
    /// it `#[non_exhaustive]`, so a version that added a type would fail
    /// to compile here rather than fall into a catch-all that spelled a
    /// new type as an old one. Every renderable arm is argued in
    /// [`exif_tags`].
    fn typed_text(value: &Value) -> String {
        match value {
            Value::Ascii(chunks) => {
                let bytes: Vec<u8> = chunks
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|b| *b != 0)
                    .collect();
                marked("ascii", String::from_utf8_lossy(&bytes).into_owned())
            }
            Value::Byte(values) => marked("byte", listed(values)),
            Value::Short(values) => marked("short", listed(values)),
            Value::Long(values) => marked("long", listed(values)),
            Value::SByte(values) => marked("sbyte", listed(values)),
            Value::SShort(values) => marked("sshort", listed(values)),
            Value::SLong(values) => marked("slong", listed(values)),
            Value::Rational(values) => marked(
                "rational",
                joined(values.iter().map(|r| format!("{}/{}", r.num, r.denom))),
            ),
            Value::SRational(values) => marked(
                "srational",
                joined(values.iter().map(|r| format!("{}/{}", r.num, r.denom))),
            ),
            Value::Float(values) => marked(
                "float",
                BASE64.encode(
                    values
                        .iter()
                        .flat_map(|v| v.to_be_bytes())
                        .collect::<Vec<u8>>(),
                ),
            ),
            Value::Double(values) => marked(
                "double",
                BASE64.encode(
                    values
                        .iter()
                        .flat_map(|v| v.to_be_bytes())
                        .collect::<Vec<u8>>(),
                ),
            ),
            Value::Undefined(bytes, _) => marked("undefined", BASE64.encode(bytes)),
            Value::Unknown(kind, count, offset) => {
                marked("unknown", format!("{kind}/{count}@{offset}"))
            }
        }
    }

    /// `type:rendering` — the one place the marker is attached.
    fn marked(kind: &str, rendered: String) -> String {
        format!("{kind}:{rendered}")
    }

    /// A count of more than one, comma-separated.
    ///
    /// Safe for every type it is reached for: an integer and a rational
    /// render to digits, a slash and a minus sign, so the separator
    /// cannot occur inside an element. `ascii` does not come through here
    /// — a comma is ordinary text there, and a separator would be
    /// ambiguous with it.
    fn listed<T: Display>(values: &[T]) -> String {
        joined(values.iter().map(T::to_string))
    }

    fn joined(rendered: impl Iterator<Item = String>) -> String {
        rendered.collect::<Vec<_>>().join(",")
    }

    /// Header-only dimension read, for containers with no EXIF. Two
    /// probes in cascade so we cover the union of format support:
    ///
    /// 1. `imagesize::blob_size` — pure-Rust header sniffer that reads
    ///    AVIF / HEIF / JXL headers without a codec dep. Fastest and
    ///    covers modern smartphone / web formats the `image` crate's
    ///    default features skip.
    /// 2. `image::ImageReader` — battle-tested header decode for
    ///    everything `imagesize` misses (EXR, QOI, exotic long-tail).
    ///
    /// Both probes are cheap — neither decodes pixels, they only read
    /// enough header bytes to expose the dimensions.
    pub fn dims_from_header(payload: &[u8]) -> Option<(u32, u32)> {
        imagesize::blob_size(payload)
            .ok()
            .map(|sz| (sz.width as u32, sz.height as u32))
            .or_else(|| {
                let reader = image::ImageReader::new(Cursor::new(payload))
                    .with_guessed_format()
                    .ok()?;
                reader.into_dimensions().ok()
            })
    }

    /// [`dims_from_header`] over a file, without loading it.
    ///
    /// Both probes have their own file-side entry points and both read
    /// incrementally, so the cascade costs the header region rather than
    /// the artefact.
    pub fn dims_from_header_at(path: &Path) -> Option<(u32, u32)> {
        imagesize::size(path)
            .ok()
            .map(|sz| (sz.width as u32, sz.height as u32))
            .or_else(|| {
                image::ImageReader::open(path)
                    .ok()?
                    .with_guessed_format()
                    .ok()?
                    .into_dimensions()
                    .ok()
            })
    }

    /// The measurement Asterism stores for a still image: EXIF when the
    /// container states it, the header otherwise.
    ///
    /// This composition is the thing that has to be shared rather than
    /// either half of it. Ingest and backfill reading the same bytes
    /// have to reach the same pair, and "EXIF first, header second" is
    /// the part a second implementation would most plausibly get
    /// subtly different — a header-only reader answers a *different*
    /// question for a JPEG that was cropped by a tool which updated the
    /// EXIF tags and not the frame.
    ///
    /// Callers that already hold [`ExifFields`] should not call this
    /// (it would parse the container twice); use
    /// [`coded_dims_with_exif`].
    pub fn coded_dims(payload: &[u8]) -> Option<(u32, u32)> {
        coded_dims_with_exif(exif_fields(payload).as_ref(), payload)
    }

    /// [`coded_dims`] for a caller that has already read the EXIF.
    pub fn coded_dims_with_exif(exif: Option<&ExifFields>, payload: &[u8]) -> Option<(u32, u32)> {
        exif.and_then(|f| f.dims)
            .or_else(|| dims_from_header(payload))
    }

    /// [`coded_dims`] over a file, without loading it.
    ///
    /// **Same composition, same order, same answer** — that is the
    /// contract, and it is why this is here rather than at the callsite.
    /// A backfill that reached for `dims_from_header_at` alone would
    /// disagree with ingest on exactly the files where EXIF and header
    /// differ, which is the disagreement this crate exists to prevent.
    pub fn coded_dims_at(path: &Path) -> Option<(u32, u32)> {
        exif_fields_at(path)
            .and_then(|f| f.dims)
            .or_else(|| dims_from_header_at(path))
    }
}

#[cfg(feature = "video")]
mod motion {
    use std::fs::File;
    use std::io::{BufReader, Cursor, Read, Seek};
    use std::path::Path;

    /// Which container reader answered.
    ///
    /// Reported rather than left implicit because it is provenance, not
    /// a detail: a caller recording *that* a file was probed has to be
    /// able to say by what, and "dims are absent" reads differently
    /// depending on whether nothing read the container or something read
    /// it and found no video track.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ProbeSource {
        /// `mp4parse` — MP4 / MOV and the rest of ISOBMFF.
        Isobmff,
        /// `matroska` — WebM / MKV.
        Ebml,
    }

    /// What a container probe surfaces, whichever layer answered.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct VideoProbe {
        /// Which reader produced this.
        pub source: ProbeSource,
        /// Coded pixel dimensions. See the crate docs: no probe here
        /// reads the MP4 display matrix or Matroska's `DisplayWidth`,
        /// so a clip stored rotated measures as it is stored.
        pub dims: Option<(u32, u32)>,
        /// Playback length in milliseconds.
        pub duration_ms: Option<u64>,
        /// Canonical codec slug (`"h264"`, `"vp9"`, …).
        pub codec: Option<String>,
    }

    /// Probes a video payload: `mp4parse` for ISOBMFF (MP4 / MOV),
    /// then `matroska` for EBML (WebM / MKV).
    ///
    /// Each layer rejects the other's container at the first magic
    /// bytes, so the order is cost only, not correctness. `None` means
    /// neither read it — AVI today — which is a footprint without dims
    /// / duration / codec rather than a failure.
    pub fn probe(payload: &[u8]) -> Option<VideoProbe> {
        try_parse_mp4(&mut Cursor::new(payload))
            .or_else(|| try_parse_matroska(Cursor::new(payload)))
    }

    /// [`probe`] over a file, without loading it.
    ///
    /// **How much this actually reads differs by container, and one of
    /// them is unavoidable.** `matroska` seeks, so an MKV costs its
    /// header. `mp4parse::read_mp4` takes `Read` and *not* `Seek`, so it
    /// walks the box tree forward: an MP4 written with `moov` at the
    /// front (the "faststart" layout) costs its header, and one with
    /// `moov` at the end — what most encoders emit by default — has its
    /// `mdat` consumed on the way there. That is a property of the
    /// parser's interface, not something a caller can arrange around.
    ///
    /// What the caller *can* arrange is that those bytes stream rather
    /// than accumulate. Reading the file into a `Vec` first makes the
    /// worst case a resident copy of the artefact; a buffered `File`
    /// makes it a buffer.
    /// Each layer gets **its own handle**, rather than a rewound one.
    ///
    /// `read_mp4` consumes as it goes and leaves the cursor wherever it
    /// gave up, so the second attempt has to start from zero. Seeking
    /// back would do that, and would also make the two attempts share a
    /// piece of mutable state whose only correct value is the one they
    /// both assume — a missing rewind reads as "this MKV is not an MKV",
    /// silently, on exactly the files the second layer exists for.
    /// Opening again costs one syscall on the branch that already
    /// failed, and there is no state left to get wrong.
    pub fn probe_at(path: &Path) -> Option<VideoProbe> {
        if let Some(probed) = try_parse_mp4(&mut BufReader::new(File::open(path).ok()?)) {
            return Some(probed);
        }
        try_parse_matroska(BufReader::new(File::open(path).ok()?))
    }

    /// EBML (WebM / MKV) probe.
    fn try_parse_matroska<R: Read + Seek>(reader: R) -> Option<VideoProbe> {
        let mkv = matroska::Matroska::open(reader).ok()?;

        let duration_ms = mkv
            .info
            .duration
            .map(|d| d.as_millis().min(u64::MAX as u128) as u64);

        let video_track = mkv
            .tracks
            .iter()
            .find(|t| t.tracktype == matroska::Tracktype::Video);
        let dims = video_track
            .and_then(|t| match &t.settings {
                matroska::Settings::Video(v) => Some((v.pixel_width as u32, v.pixel_height as u32)),
                _ => None,
            })
            .filter(|(w, h)| *w > 0 && *h > 0);
        let codec = video_track
            .or_else(|| {
                mkv.tracks
                    .iter()
                    .find(|t| t.tracktype == matroska::Tracktype::Audio)
            })
            .and_then(|t| matroska_codec_slug(&t.codec_id));

        Some(VideoProbe {
            source: ProbeSource::Ebml,
            dims,
            duration_ms,
            codec,
        })
    }

    /// Matroska codec ids are strings ("V_VP9"), not an enum — map the
    /// ones we recognise onto the same slugs [`mp4_codec_slug`] emits so
    /// the label vocabulary does not fork by container.
    fn matroska_codec_slug(codec_id: &str) -> Option<String> {
        Some(
            match codec_id {
                "V_VP8" => "vp8",
                "V_VP9" => "vp9",
                "V_AV1" => "av1",
                "V_MPEG4/ISO/AVC" => "h264",
                "V_MPEGH/ISO/HEVC" => "hevc",
                "A_AAC" => "aac",
                "A_OPUS" => "opus",
                "A_VORBIS" => "vorbis",
                "A_FLAC" => "flac",
                "A_MPEG/L3" => "mp3",
                _ => return None,
            }
            .to_string(),
        )
    }

    /// ISOBMFF (MP4 / MOV) probe.
    ///
    /// `Read` and not `Read + Seek` because that is `read_mp4`'s own
    /// bound — see [`probe_at`] for what it costs on a container whose
    /// `moov` sits behind its `mdat`.
    fn try_parse_mp4<R: Read>(reader: &mut R) -> Option<VideoProbe> {
        let ctx = mp4parse::read_mp4(reader).ok()?;

        // Prefer the first video track for dims / codec. Fall back to
        // any track for duration when no video is present (audio-only
        // MP4).
        let video_track = ctx
            .tracks
            .iter()
            .find(|t| t.track_type == mp4parse::TrackType::Video);
        let any_track = video_track.or_else(|| ctx.tracks.first());

        // Duration lives on Track (per-track scaled time). Use the
        // representative track's timescale to convert to ms.
        let duration_ms = any_track.and_then(|t| {
            let dur = t.duration?.0;
            let ts = t.timescale?.0;
            if ts == 0 {
                return None;
            }
            Some((dur as u128).saturating_mul(1000).checked_div(ts as u128)? as u64)
        });

        let dims = video_track
            .and_then(|t| t.tkhd.as_ref())
            .map(|tkhd| {
                // 16.16 fixed-point → integer pixel count.
                let w = tkhd.width >> 16;
                let h = tkhd.height >> 16;
                (w, h)
            })
            .filter(|(w, h)| *w > 0 && *h > 0);

        // Codec lives inside stsd.descriptions[0] as a SampleEntry
        // (Video / Audio / Unknown). VideoSampleEntry carries the codec
        // enum.
        let codec = video_track.and_then(|t| {
            t.stsd.as_ref().and_then(|stsd| {
                stsd.descriptions.first().and_then(|d| match d {
                    mp4parse::SampleEntry::Video(v) => mp4_codec_slug(v.codec_type),
                    mp4parse::SampleEntry::Audio(a) => mp4_codec_slug(a.codec_type),
                    _ => None,
                })
            })
        });

        Some(VideoProbe {
            source: ProbeSource::Isobmff,
            dims,
            duration_ms,
            codec,
        })
    }

    fn mp4_codec_slug(kind: mp4parse::CodecType) -> Option<String> {
        use mp4parse::CodecType;
        Some(match kind {
            CodecType::H264 => "h264".into(),
            CodecType::H263 => "h263".into(),
            CodecType::AV1 => "av1".into(),
            CodecType::VP8 => "vp8".into(),
            CodecType::VP9 => "vp9".into(),
            CodecType::MP4V => "mp4v".into(),
            CodecType::AAC => "aac".into(),
            CodecType::FLAC => "flac".into(),
            CodecType::Opus => "opus".into(),
            CodecType::MP3 => "mp3".into(),
            CodecType::LPCM => "lpcm".into(),
            CodecType::ALAC => "alac".into(),
            _ => return None,
        })
    }
}

#[cfg(feature = "image")]
pub use still::{
    ExifFields, coded_dims, coded_dims_at, coded_dims_with_exif, dims_from_header,
    dims_from_header_at, exif_fields, exif_fields_at, exif_tags,
};

#[cfg(feature = "video")]
pub use motion::{ProbeSource, VideoProbe, probe, probe_at};

#[cfg(all(test, feature = "image"))]
mod still_tests {
    use std::io::Cursor;

    use super::*;
    use image::{ImageFormat, RgbImage};

    /// A blank image of a known size, encoded into `format`.
    ///
    /// Every fixture is **non-square**, which is the one property that
    /// makes these cases able to fail: nothing in the type system stops
    /// a transposed read, and a square fixture reports the same pair
    /// either way round.
    fn encode(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let img = RgbImage::new(width, height);
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, format)
            .expect("encode");
        buf.into_inner()
    }

    #[test]
    fn dims_from_header_reads_png() {
        assert_eq!(
            dims_from_header(&encode(37, 21, ImageFormat::Png)),
            Some((37, 21))
        );
    }

    #[test]
    fn dims_from_header_reads_gif() {
        assert_eq!(
            dims_from_header(&encode(64, 48, ImageFormat::Gif)),
            Some((64, 48))
        );
    }

    #[test]
    fn dims_from_header_reads_bmp() {
        assert_eq!(
            dims_from_header(&encode(16, 15, ImageFormat::Bmp)),
            Some((16, 15))
        );
    }

    #[test]
    fn dims_from_header_reads_tiff() {
        assert_eq!(
            dims_from_header(&encode(24, 12, ImageFormat::Tiff)),
            Some((24, 12))
        );
    }

    #[test]
    fn dims_from_header_returns_none_on_garbage() {
        assert!(dims_from_header(b"not an image at all").is_none());
    }

    /// **The slice form and the file form answer identically.**
    ///
    /// They exist as a pair so a caller holding bytes pays nothing and a
    /// caller holding a path does not load the artefact — but two entry
    /// points to one measurement is exactly the shape this crate was
    /// created to remove, so the equivalence has to be asserted rather
    /// than assumed. The cascade inside them is written twice (`_at`
    /// reaches for `imagesize::size` where the other reaches for
    /// `blob_size`), which is where a divergence would come from.
    #[test]
    fn the_file_form_agrees_with_the_slice_form() {
        let dir = std::env::temp_dir().join("asterism-media-probe-parity");
        std::fs::create_dir_all(&dir).expect("scratch dir");
        for (name, format) in [
            ("parity.png", ImageFormat::Png),
            ("parity.gif", ImageFormat::Gif),
            ("parity.bmp", ImageFormat::Bmp),
            ("parity.tiff", ImageFormat::Tiff),
        ] {
            let bytes = encode(37, 21, format);
            let path = dir.join(name);
            std::fs::write(&path, &bytes).expect("write fixture");
            assert_eq!(
                coded_dims_at(&path),
                coded_dims(&bytes),
                "{name}: the two forms disagree"
            );
            assert_eq!(coded_dims_at(&path), Some((37, 21)), "{name}");
            let _ = std::fs::remove_file(&path);
        }

        // And on something neither reads, so "they agree" is not just
        // "they both answer".
        let junk = dir.join("parity.txt");
        std::fs::write(&junk, b"not an image at all").expect("write fixture");
        assert_eq!(coded_dims_at(&junk), None);
        assert_eq!(coded_dims(b"not an image at all"), None);
        let _ = std::fs::remove_file(&junk);

        // A path that is not there is absence, not a panic — the
        // backfill meets moved and deleted files routinely.
        assert_eq!(coded_dims_at(&dir.join("nothing-here.png")), None);
    }

    /// A container with no EXIF reads its dimensions from the header.
    ///
    /// This is the composition, not either probe: `coded_dims` has to
    /// fall through when `exif_fields` answers `None`, which is the
    /// ordinary state for a screenshot.
    #[test]
    fn coded_dims_falls_through_to_the_header_without_exif() {
        let bytes = encode(37, 21, ImageFormat::Png);
        assert_eq!(exif_fields(&bytes), None, "the fixture carries no EXIF");
        assert_eq!(coded_dims(&bytes), Some((37, 21)));
    }

    /// EXIF wins over the header when both are present.
    ///
    /// The order matters and is the reason the composition is shared
    /// rather than reimplemented: a file whose EXIF states a different
    /// frame from its header — a crop written by a tool that updated one
    /// and not the other — gets one answer at ingest and has to get the
    /// same one at backfill. The fixture forces the two apart by handing
    /// `coded_dims_with_exif` an EXIF reading the payload does not have.
    #[test]
    fn coded_dims_prefers_exif_over_the_header() {
        let bytes = encode(37, 21, ImageFormat::Png);
        let stated = ExifFields {
            dims: Some((1000, 2000)),
            ..ExifFields::default()
        };
        assert_eq!(
            coded_dims_with_exif(Some(&stated), &bytes),
            Some((1000, 2000)),
            "EXIF is the preferred evidence"
        );
        // …and an EXIF block that states everything *except* dimensions
        // still falls through, rather than reading "EXIF present" as
        // "EXIF answered".
        let silent = ExifFields {
            camera_make: Some("ACME".into()),
            ..ExifFields::default()
        };
        assert_eq!(
            coded_dims_with_exif(Some(&silent), &bytes),
            Some((37, 21)),
            "EXIF without dimensions is not an answer"
        );
    }
}

#[cfg(all(test, feature = "video"))]
mod motion_tests {
    use super::*;

    /// A container neither probe reads answers `None` rather than a
    /// zeroed [`VideoProbe`].
    ///
    /// The distinction is the same one the still side draws: "nobody
    /// measured this" and "measured as nothing" are different facts, and
    /// only the first is true of an AVI here.
    #[test]
    fn an_unreadable_container_is_not_a_zeroed_measurement() {
        assert_eq!(probe(b"RIFF\x00\x00\x00\x00AVI LIST"), None);
        assert_eq!(probe(b""), None);
    }

    /// The file form agrees with the slice form.
    ///
    /// **Narrower than it looks, and worth saying so.** The fixture is a
    /// container neither layer reads, so this pins the absent case and
    /// the missing-file case and nothing about the hand-off between the
    /// two layers — a stub that answers `None` twice cannot tell a
    /// working second attempt from a broken one. Covering that needs a
    /// real MKV, which lives in `asterism-importer-video`'s fixtures and
    /// is not this crate's to reach for. The hand-off is instead made
    /// unbreakable by construction: [`probe_at`] opens the file again
    /// rather than rewinding one handle, so there is no shared cursor
    /// for a missing seek to leave in the wrong place.
    #[test]
    fn the_file_form_agrees_with_the_slice_form() {
        let dir = std::env::temp_dir().join("asterism-media-probe-video-parity");
        std::fs::create_dir_all(&dir).expect("scratch dir");

        let junk = dir.join("opaque.avi");
        std::fs::write(&junk, b"RIFF\x00\x00\x00\x00AVI LIST").expect("write fixture");
        assert_eq!(probe_at(&junk), probe(b"RIFF\x00\x00\x00\x00AVI LIST"));
        assert_eq!(probe_at(&junk), None);
        let _ = std::fs::remove_file(&junk);

        assert_eq!(probe_at(&dir.join("nothing-here.mp4")), None);
    }
}
