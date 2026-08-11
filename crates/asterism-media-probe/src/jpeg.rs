//! JPEG segment framing: where one marker segment ends, where the next
//! begins, and where the entropy-coded bytes run.
//!
//! The sibling of [`png`](crate::png) and the same contract. Every
//! function here is a pure function of a byte slice over input an
//! importer collected from outside, with the failure modes a parser has
//! (truncation, a length field that lies, an unbounded segment count).
//! **Nothing here decides what a segment means.** Which of them belong
//! to a picture's identity, which are notes written about it, what gets
//! hashed and in what order — those are judgements about a corpus, and
//! they are made by the caller that has one
//! (`asterism-infra`'s `probes::jpeg`).
//!
//! # Two shapes in one container
//!
//! A PNG is one shape all the way down: length, type, payload, CRC,
//! repeated. A JPEG is two. Up to `SOS` it is a sequence of marker
//! segments, each a two-byte marker and — for most markers — a
//! two-byte length that includes itself. After an `SOS` header comes
//! **entropy-coded data, which is not framed at all**: it runs until the
//! next real marker, and the only way to find that marker is to read
//! every byte looking for one.
//!
//! So the walk yields more than one thing ([`Element`]), and the scan is
//! why this module cannot be a copy of the PNG one with the names
//! changed.
//!
//! # And bytes after the end marker, which are not nothing
//!
//! `EOI` ends the image. It does not end the file: a Google or Samsung
//! *Motion Photo* is a complete JPEG with a complete MP4 appended behind
//! it, which is how a phone ships a still and its two seconds of video as
//! one artefact. Those bytes are reported — [`Element::Trailing`], last,
//! once — rather than silently dropped, because dropping them here would
//! be this module deciding that a Motion Photo and its still-only export
//! are one file. That is exactly the identity judgement the layer above
//! owns, and it is the direction that cannot be undone: the caller folds
//! a duplicate group and the video is gone.
//!
//! The walk still stops *reading* at `EOI` — the trailing bytes are
//! handed back as one span and nothing in them is parsed, because what
//! follows an image is not a JPEG structure and this module has no
//! grammar for it.
//!
//! # Finding the end of a scan is the dangerous part
//!
//! Inside entropy-coded data a literal `0xFF` byte is written as the two
//! bytes `0xFF 0x00` — *byte stuffing* — precisely so that a `0xFF`
//! followed by anything else can be recognised as a marker. Two more
//! things also appear mid-scan and are not the end of it: `RSTn`
//! (`0xFFD0`–`0xFFD7`), which an encoder emits every restart interval,
//! and fill bytes (runs of `0xFF` before a marker, which the format
//! permits anywhere a marker may appear).
//!
//! Reading any of the three as "the scan ends here" truncates the scan,
//! and what that costs depends on what the walk does next. Resume by
//! *resynchronising* — skipping ahead to the next plausible marker —
//! and the caller is handed a short span, so **two different pictures
//! get the same digest** as soon as they agree up to the first stuffed
//! byte: the unrecoverable direction the content probe's denylist
//! exists to avoid, arriving through the parser instead. Resume by
//! refusing, which is what this module's marker step does, and the file
//! loses its region entirely — an improvement lost, on nearly every
//! JPEG in existence, which is loud rather than dangerous.
//!
//! The second is what happens here, and it is a property of the two
//! halves together rather than of either: `0xFF 0x00` is not a marker
//! and the bytes after an `RSTn` are not one either, so a misread scan
//! runs straight into a refusal.
//! `tests::a_scan_runs_through_stuffing_restarts_and_fill_bytes` keeps
//! the walk itself measured, and the probe's
//! `a_scan_is_read_past_its_stuffing_and_its_restart_markers` keeps both
//! outcomes measured from the caller's side.
//!
//! All three therefore stay *inside* the span this module reports. They
//! are bytes of the scan, they are handed back as bytes of the scan, and
//! nothing here removes them — a parser that dropped the `0x00` of a
//! stuffed pair would be deciding that two files differing in their
//! stuffing are one file, which is not a framing question.
//!
//! # Fill bytes outside a scan end the walk
//!
//! Between two marker segments the format also permits fill bytes, and
//! there this module refuses rather than skipping them. Skipping would
//! be an exclusion — two files differing only in that padding would come
//! out identical — decided here, in the layer that is supposed to have
//! no opinion about identity, and invisible to the probe that owns the
//! denylist. Refusing lands the file on the caller's "no region" marker,
//! which costs an undetected duplicate and no more. If such a file ever
//! turns up, the fix is to give the padding a place in [`Element`] so
//! the probe can decide, not to make it disappear here.
//!
//! # One fact here is not framing
//!
//! [`orientation`] reads the EXIF Orientation tag, which is neither a
//! boundary nor a length, and it is here anyway because it is still a
//! *fact about the bytes*: this file, the format says, renders
//! transposed. What it is worth — whether two files that disagree about
//! it are two pictures — is the caller's, and the caller is the one that
//! decides to feed it to a digest.
//!
//! It reads through `kamadak-exif` rather than through [`segments`],
//! even though the tag arrives in an `APP1` segment this module can
//! already find. Locating the segment is the easy tenth of the job; the
//! rest is a TIFF header, an endianness flag, an IFD and its entry
//! table, and writing a second reader for it here would put a TIFF
//! parser in a module whose whole claim is that it does not have one.
//! The crate already depends on that reader for
//! [`ExifFields`](crate::ExifFields).
//!
//! # Reading files from outside
//!
//! Every length in a JPEG is two bytes inside the file and includes
//! itself, so the largest a segment can declare is 65,533 bytes of
//! payload — there is no 4 GiB bomb to defuse here, unlike PNG's
//! 32-bit lengths. What a length can still do is lie: declare more than
//! the file actually holds, which is [`ScanError::Truncated`] and is
//! checked before the payload is sliced. Nothing here is sized from a
//! declared length; a payload is a subslice of the input.

/// Ceiling on segments walked in one file.
///
/// A JPEG's segments each cost at least two bytes, so the count is
/// already bounded by the input, and — unlike [`png::MAX_CHUNKS`](crate::png::MAX_CHUNKS)
/// — nothing on either side of this walk accumulates one item per
/// segment: the content probe feeds each element to a hash in file order
/// and keeps none of them, because a JPEG's scan data is hashed where it
/// sits rather than concatenated at the end. So this bounds work, not
/// memory, and saying which it bounds is the point of writing it down:
/// a future caller that starts collecting elements has to raise its own
/// ceiling deliberately rather than inherit one that was never about it.
///
/// Real files are nowhere near it. A photograph out of a camera carries
/// on the order of ten segments; a progressive re-encode with a full set
/// of scans, a few dozen. 65,536 needs 128 KiB of nothing but markers,
/// and matches the PNG walk's ceiling so the two answer "too many" at
/// the same order of magnitude.
pub const MAX_SEGMENTS: usize = 65_536;

/// `SOI` — the two bytes every JPEG starts with, after the `0xFF`.
const SOI: u8 = 0xD8;

/// `EOI` — end of image, and the end of the walk.
const EOI: u8 = 0xD9;

/// `SOS` — start of scan. Its payload is a header; the entropy-coded
/// bytes follow it unframed.
const SOS: u8 = 0xDA;

/// `TEM` — a standalone marker with no payload, the one below the
/// restart range.
const TEM: u8 = 0x01;

/// The prefix byte of every marker, the stuffing pair, and the fill
/// byte, all of which are this value.
const MARK: u8 = 0xFF;

/// One element of a JPEG, in the order the file carries it.
///
/// Four variants because a JPEG is not one shape (see the module doc).
/// The marker is the byte *after* the `0xFF`, so `Framed { marker:
/// 0xE1, .. }` is `APP1`, matching how the format's tables name them and
/// how a caller's denylist will be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element<'a> {
    /// A marker with no length field and no payload: `SOI`, `EOI`,
    /// `TEM`, and any `RSTn` that turns up outside a scan.
    Bare(u8),
    /// `0xFF <marker> <length: u16 be> <payload>`.
    ///
    /// The payload is borrowed out of the input **without** the two
    /// length bytes, and the length is recoverable exactly as
    /// `payload.len() + 2` — which is what the file declared, since a
    /// declaration that did not match the bytes present is refused
    /// before this is built.
    Framed {
        /// The byte after the `0xFF`.
        marker: u8,
        /// The segment body, borrowed out of the input.
        payload: &'a [u8],
    },
    /// The entropy-coded bytes an `SOS` header introduces, exactly as
    /// the file carries them.
    ///
    /// Stuffing pairs, `RSTn` markers and fill bytes are all inside this
    /// slice and none of them are removed — see the module doc for why
    /// taking any of them out would be an identity decision made in the
    /// wrong layer. Always yielded directly after the `Framed` `SOS`
    /// that introduced it, including when it is empty.
    Scan(&'a [u8]),
    /// Everything the file carries after its `EOI`, as one span.
    ///
    /// Yielded last, at most once, and **only when there is something
    /// there** — a file that ends at its end marker yields no `Trailing`
    /// rather than an empty one, so the ordinary JPEG is described
    /// exactly as it was before this variant existed.
    ///
    /// Nothing in the span is parsed. It is usually an MP4 (see the
    /// module doc on Motion Photos), sometimes a thumbnail, sometimes
    /// padding, and this module has no grammar for any of them; what it
    /// can say truthfully is where the bytes are.
    Trailing(&'a [u8]),
}

/// Why a walk stopped short of the end of the image.
///
/// The variants are worth having even for a caller that treats all of
/// them alike: a reader of a stack trace or a future diagnostic can tell
/// which happened, where a boolean could only say "stopped".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanError {
    /// The input does not start with `SOI` followed by a marker.
    NotJpeg,
    /// A segment declares more bytes than the input holds, or the input
    /// ended without an `EOI`.
    Truncated,
    /// A segment declares a length below the two bytes the length field
    /// itself occupies — a claim no payload can satisfy.
    InvalidLength,
    /// A marker was expected and the bytes there are not one: a byte
    /// other than `0xFF`, a fill byte between segments (see the module
    /// doc), or a `0xFF 0x00` stuffing pair where nothing is being
    /// stuffed.
    NotAMarker,
    /// The sequence ran past [`MAX_SEGMENTS`].
    TooManySegments,
}

/// True when `bytes` starts with `SOI` and the first byte of a marker.
///
/// Three bytes rather than two. `SOI` is followed by a marker in every
/// JPEG, so the third byte is `0xFF` by construction, and requiring it
/// costs nothing while turning away the far larger set of files that
/// merely begin `0xFF 0xD8`.
pub fn is_jpeg(bytes: &[u8]) -> bool {
    matches!(bytes, [MARK, SOI, MARK, ..])
}

/// Walks the image, yielding every element in the order the file
/// carries it.
///
/// `Err(ScanError::NotJpeg)` up front when the signature is wrong;
/// otherwise the iterator yields elements until one of them fails, and a
/// failure is the last item. **A caller must not treat a partial walk as
/// a whole one** — the sequence stops on the first defect, so what came
/// before it is part of an image rather than an image.
///
/// The walk stops reading at `EOI`, which is yielded, and whatever
/// stands behind it follows as one unparsed [`Element::Trailing`] span
/// when the file carries any. `SOI` is *not* yielded: it is the
/// signature, it was checked to get here, and it is the same two bytes
/// in every JPEG, so a caller hashing the elements is neither given nor
/// denied anything by it.
///
/// The iterator keeps no per-element state beyond a count and a cursor,
/// so the memory a walk costs is whatever the caller chooses to hold on
/// to.
pub fn segments(bytes: &[u8]) -> Result<Segments<'_>, ScanError> {
    if !is_jpeg(bytes) {
        return Err(ScanError::NotJpeg);
    }
    Ok(Segments {
        rest: &bytes[2..],
        walked: 0,
        pending_scan: false,
        ended: false,
        done: false,
    })
}

/// Iterator returned by [`segments`].
#[derive(Debug)]
pub struct Segments<'a> {
    /// The unread remainder, starting at the `0xFF` of the next marker
    /// or — while [`pending_scan`](Self::pending_scan) is set — at the
    /// first entropy-coded byte.
    rest: &'a [u8],
    walked: usize,
    /// Set when the last element was an `SOS` header, so the next one is
    /// the scan it introduced rather than another marker.
    pending_scan: bool,
    /// Set when `EOI` was yielded. The image is over and nothing more is
    /// parsed; what remains is handed back whole as
    /// [`Element::Trailing`].
    ended: bool,
    done: bool,
}

impl<'a> Iterator for Segments<'a> {
    type Item = Result<Element<'a>, ScanError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.ended {
            // The image is over. What is left is one span, it is not
            // walked, and it does not count against the ceiling — the
            // ceiling bounds parsing work and nothing here parses.
            self.done = true;
            let trailing = std::mem::take(&mut self.rest);
            return (!trailing.is_empty()).then_some(Ok(Element::Trailing(trailing)));
        }
        self.walked += 1;
        if self.walked > MAX_SEGMENTS {
            self.done = true;
            return Some(Err(ScanError::TooManySegments));
        }

        let stepped = if self.pending_scan {
            self.pending_scan = false;
            self.take_scan()
        } else {
            self.take_marker()
        };
        if stepped.is_err() {
            self.done = true;
        }
        Some(stepped)
    }
}

impl<'a> Segments<'a> {
    /// The entropy-coded run from the cursor to the next real marker.
    fn take_scan(&mut self) -> Result<Element<'a>, ScanError> {
        let at = scan_length(self.rest)?;
        let (entropy, rest) = self.rest.split_at(at);
        self.rest = rest;
        Ok(Element::Scan(entropy))
    }

    /// The marker segment at the cursor.
    fn take_marker(&mut self) -> Result<Element<'a>, ScanError> {
        let rest = self.rest;
        let (Some(&MARK), Some(&marker)) = (rest.first(), rest.get(1)) else {
            // Either the input ran out — a JPEG that never reached its
            // `EOI` — or the byte where a marker belongs is not one.
            return Err(if rest.len() < 2 {
                ScanError::Truncated
            } else {
                ScanError::NotAMarker
            });
        };
        // A fill byte between two segments, or a stuffing pair outside
        // the scan that is the only place stuffing means anything. Both
        // are refused rather than skipped — see the module doc.
        if marker == MARK || marker == 0x00 {
            return Err(ScanError::NotAMarker);
        }

        if is_standalone(marker) {
            self.rest = &rest[2..];
            if marker == EOI {
                self.ended = true;
            }
            return Ok(Element::Bare(marker));
        }

        let body = &rest[2..];
        // The length field is two bytes and counts itself, so anything
        // below 2 describes a segment shorter than its own length, and
        // the payload it implies has no representation.
        let declared = match body.get(..2) {
            Some(&[high, low]) => usize::from(u16::from_be_bytes([high, low])),
            _ => return Err(ScanError::Truncated),
        };
        if declared < 2 {
            return Err(ScanError::InvalidLength);
        }
        // The declared length is a claim about the file, so it is used
        // only to ask whether that many bytes are actually there.
        if body.len() < declared {
            return Err(ScanError::Truncated);
        }
        let payload = &body[2..declared];
        self.rest = &body[declared..];
        if marker == SOS {
            self.pending_scan = true;
        }
        Ok(Element::Framed { marker, payload })
    }
}

/// The EXIF Orientation this file declares, if it declares one the
/// format defines.
///
/// `Some(1..=8)` or `None`, and the three ways of arriving at `None` are
/// deliberately one answer: no EXIF block, an EXIF block nothing can
/// read, and a code outside the eight the specification assigns. None of
/// them says how the picture is rotated, so a caller that needs a
/// rendering has the obvious value to fall back on and the content probe
/// in `asterism-infra` falls back to it. Reporting *which* kind of
/// silence it was would be reporting on the file's health, which is a
/// different question from the one this answers.
///
/// What the codes mean, since a caller reasoning about identity needs to
/// know the range is not decorative: `1` is upright, `2`–`4` are
/// mirrored and half-turned, and **`5`–`8` transpose the frame** — a
/// portrait photograph out of a phone is stored landscape with a `6` on
/// it. So two files agreeing on every pixel and disagreeing here are
/// shown to a person as two different pictures.
///
/// Reads the primary IFD only, which is the one a camera writes it in.
/// A thumbnail's own orientation (`In::THUMBNAIL`) is about the
/// thumbnail.
pub fn orientation(bytes: &[u8]) -> Option<u8> {
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(bytes))
        .ok()?;
    let code = exif
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
        .value
        .get_uint(0)?;
    u8::try_from(code)
        .ok()
        .filter(|code| (1..=8).contains(code))
}

/// Markers that carry no length field and no payload.
///
/// `TEM` and the restart range, plus `SOI` and `EOI` at the top of it —
/// `0xD8` and `0xD9` sit directly above `RST7`, so the four are one
/// range in the table and are one arm here.
fn is_standalone(marker: u8) -> bool {
    matches!(marker, TEM | 0xD0..=EOI)
}

/// How many bytes of entropy-coded data sit before the next real
/// marker.
///
/// The three things that look like the end and are not — stuffing
/// (`0xFF 0x00`), restarts (`0xFF 0xD0`–`0xFF 0xD7`) and fill bytes
/// (`0xFF 0xFF`) — are stepped over and stay inside the returned length.
/// A fill byte advances by one rather than two, so a run of them ends at
/// the real marker behind it and every `0xFF` of the run but the last
/// stays in the scan.
///
/// Running out of input is [`ScanError::Truncated`]: entropy-coded data
/// is terminated by a marker, so a scan that reaches the end of the file
/// is a file with no `EOI`.
fn scan_length(data: &[u8]) -> Result<usize, ScanError> {
    let mut at = 0;
    while at < data.len() {
        if data[at] != MARK {
            at += 1;
            continue;
        }
        match data.get(at + 1).copied() {
            None => return Err(ScanError::Truncated),
            // A stuffed `0xFF`, written as two bytes so that it cannot
            // be read as a marker. Both bytes are scan data.
            Some(0x00) => at += 2,
            // A fill byte: step one, so the marker behind the run is
            // found and the padding stays where the file put it.
            Some(MARK) => at += 1,
            // A restart marker, which an encoder emits inside the scan
            // every restart interval.
            Some(0xD0..=0xD7) => at += 2,
            // Anything else is the marker that ends the scan.
            Some(_) => return Ok(at),
        }
    }
    Err(ScanError::Truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `0xFF` + marker, with no payload.
    fn bare(marker: u8) -> Vec<u8> {
        vec![MARK, marker]
    }

    /// `0xFF` + marker + the length the payload implies + the payload.
    ///
    /// The length is computed here rather than passed in, so a fixture
    /// cannot accidentally declare one the bytes do not match — the
    /// cases that want that declare it by hand.
    fn framed(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![MARK, marker];
        let declared =
            u16::try_from(payload.len() + 2).expect("fixture payload fits a JPEG length");
        out.extend_from_slice(&declared.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn build(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut out = bare(SOI);
        for part in parts {
            out.extend_from_slice(part);
        }
        out
    }

    /// Walks to the end and returns what was yielded, so a test can
    /// assert on both the elements and the stop.
    fn walk(bytes: &[u8]) -> Result<(Vec<Element<'_>>, Option<ScanError>), ScanError> {
        let mut out = Vec::new();
        let mut stopped = None;
        for item in segments(bytes)? {
            match item {
                Ok(element) => out.push(element),
                Err(err) => {
                    stopped = Some(err);
                    break;
                }
            }
        }
        Ok((out, stopped))
    }

    #[test]
    fn every_element_comes_back_in_file_order_with_its_payload_borrowed() {
        let bytes = build(&[
            framed(0xE0, b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0"),
            framed(0xDB, &[0x00, 0x10, 0x20]),
            framed(0xDA, &[0x01, 0x01, 0x00]),
            vec![0x9a, 0xbc, 0xde],
            bare(EOI),
        ]);
        let (walked, stopped) = walk(&bytes).expect("a JPEG");
        assert_eq!(stopped, None);
        assert_eq!(
            walked,
            vec![
                Element::Framed {
                    marker: 0xE0,
                    payload: b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0",
                },
                Element::Framed {
                    marker: 0xDB,
                    payload: &[0x00, 0x10, 0x20],
                },
                Element::Framed {
                    marker: 0xDA,
                    payload: &[0x01, 0x01, 0x00],
                },
                Element::Scan(&[0x9a, 0xbc, 0xde]),
                Element::Bare(EOI),
            ]
        );
        // The length field is never part of a payload — the whole reason
        // a caller can hash a payload directly and re-derive the length
        // the file declared.
        let Element::Framed { payload, .. } = walked[1] else {
            panic!("the second element is a framed segment");
        };
        assert_eq!(payload.len() + 2, 5, "the length the fixture wrote");
    }

    /// **The scan is not ended by anything that merely looks like a
    /// marker.**
    ///
    /// Stuffing, restarts and fill bytes all appear in ordinary encoder
    /// output, and reading any of them as the end of the scan hands the
    /// caller a truncated span — which is how two different pictures
    /// come to share a digest. Each of the three is placed before a byte
    /// that would otherwise be a plausible stopping point.
    #[test]
    fn a_scan_runs_through_stuffing_restarts_and_fill_bytes() {
        let entropy = vec![
            0x12, // ordinary data
            MARK, 0x00, // a stuffed 0xFF
            0x34, MARK, 0xD0, // RST0
            0x56, MARK, 0xD7, // RST7
            0x78, MARK, MARK, MARK, 0x00, // fill bytes, then a stuffed 0xFF
            0x9a,
        ];
        let bytes = build(&[
            framed(0xDA, &[0x01]),
            entropy.clone(),
            vec![MARK, MARK], // fill before the end marker
            bare(EOI),
        ]);

        let (walked, stopped) = walk(&bytes).expect("a JPEG");
        assert_eq!(stopped, None);
        // The trailing fill run is scan data up to the last 0xFF, which
        // is the one the EOI is read from — so two of the three go in
        // and the third begins the marker.
        let mut expected = entropy.clone();
        expected.extend_from_slice(&[MARK, MARK]);
        assert_eq!(
            walked,
            vec![
                Element::Framed {
                    marker: 0xDA,
                    payload: &[0x01],
                },
                Element::Scan(&expected),
                Element::Bare(EOI),
            ]
        );

        // The measurement, not the assertion: a walk that stopped at the
        // first 0xFF pair would report one byte of scan, and every image
        // whose first entropy byte is 0x12 would look alike from there.
        assert!(
            expected.len() > 1,
            "the fixture has to carry more than the prefix a naive stop would report"
        );
    }

    #[test]
    fn a_progressive_image_walks_every_scan_it_carries() {
        let bytes = build(&[
            framed(0xC2, &[0x08, 0, 1, 0, 1, 1]),
            framed(0xDA, &[0x01, 0x00]),
            vec![0x11, 0x22],
            framed(0xDA, &[0x01, 0x01]),
            vec![0x33, MARK, 0x00],
            bare(EOI),
        ]);
        let (walked, stopped) = walk(&bytes).expect("a JPEG");
        assert_eq!(stopped, None);
        assert_eq!(
            walked
                .iter()
                .filter(|e| matches!(e, Element::Scan(_)))
                .count(),
            2,
            "each SOS introduces its own scan"
        );
        assert_eq!(walked[4], Element::Scan(&[0x33, MARK, 0x00]));
    }

    /// **What follows the end marker comes back, unparsed and whole.**
    ///
    /// A Motion Photo is this shape: a complete JPEG, then a complete
    /// MP4. So the tail is handed to the caller rather than dropped —
    /// dropping it here would make a phone's still-and-video artefact
    /// indistinguishable from its still-only export, in a module with no
    /// business deciding that.
    ///
    /// Unparsed is the other half. The tail below *is* a well-formed
    /// `COM` segment, and it is reported as bytes rather than as a
    /// segment: the walk is over at `EOI`, and reading structure into
    /// what follows would be guessing at a grammar this module does not
    /// have.
    #[test]
    fn the_bytes_after_the_end_marker_come_back_as_one_unparsed_span() {
        let tail = framed(0xFE, b"structure the walk must not read as structure");
        let mut bytes = build(&[framed(0xDA, &[0x01]), vec![0x42], bare(EOI)]);
        bytes.extend_from_slice(&tail);

        let (walked, stopped) = walk(&bytes).expect("a JPEG");
        assert_eq!(stopped, None);
        assert_eq!(
            walked,
            vec![
                Element::Framed {
                    marker: 0xDA,
                    payload: &[0x01],
                },
                Element::Scan(&[0x42]),
                Element::Bare(EOI),
                Element::Trailing(&tail),
            ]
        );

        // An ordinary JPEG ends at its end marker and says nothing more
        // — an empty tail is no element, not an empty one.
        let plain = build(&[framed(0xDA, &[0x01]), vec![0x42], bare(EOI)]);
        let (walked, stopped) = walk(&plain).expect("a JPEG");
        assert_eq!(stopped, None);
        assert_eq!(walked.last(), Some(&Element::Bare(EOI)));
        assert_eq!(walked.len(), 3, "SOS, its scan, and the end");
    }

    /// **The orientation is read, normalised, and silent about which
    /// silence it met.**
    ///
    /// The fixture is written by `image`'s encoder and then given an
    /// EXIF block by hand, because the encoder writes none: what is
    /// being read is a TIFF structure, and building it here is the only
    /// way to state a code and then assert it came back.
    #[test]
    fn the_orientation_comes_back_normalised_or_not_at_all() {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(8, 4))
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .expect("encode");
        let plain = buf.into_inner();
        assert_eq!(
            orientation(&plain),
            None,
            "an encoder that writes no EXIF declares no orientation"
        );

        // `Exif\0\0`, a little-endian TIFF header, and an IFD holding
        // one SHORT: the smallest thing that states an orientation.
        let tagged = |code: u16| {
            let mut payload = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0".to_vec();
            payload.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
            payload.extend_from_slice(&3u16.to_le_bytes()); // SHORT
            payload.extend_from_slice(&1u32.to_le_bytes()); // count
            payload.extend_from_slice(&code.to_le_bytes());
            payload.extend_from_slice(&[0, 0]); // the value field is four wide
            payload.extend_from_slice(&0u32.to_le_bytes()); // no next IFD

            let mut out = plain[..2].to_vec();
            out.extend_from_slice(&framed(0xE1, &payload));
            out.extend_from_slice(&plain[2..]);
            out
        };

        for code in 1..=8u16 {
            assert_eq!(
                orientation(&tagged(code)),
                Some(code as u8),
                "the file states {code}"
            );
        }
        // Outside the eight the specification assigns, so it names no
        // rendering and is the same answer as no tag at all.
        for code in [0u16, 9, 255, 4096] {
            assert_eq!(
                orientation(&tagged(code)),
                None,
                "{code} names no rendering"
            );
        }
        // And so is a block nothing can read.
        assert_eq!(orientation(b"not an image at all"), None);
    }

    #[test]
    fn a_structural_defect_ends_the_walk_and_names_itself() {
        let intact = build(&[framed(0xDB, &[1, 2, 3]), bare(EOI)]);
        assert_eq!(walk(&intact).expect("a JPEG").1, None);

        // A length that runs past the end of the file.
        let mut overrun = intact.clone();
        overrun[4..6].copy_from_slice(&0x0100u16.to_be_bytes());
        assert_eq!(
            walk(&overrun).expect("a JPEG").1,
            Some(ScanError::Truncated)
        );

        // A length below the two bytes it occupies itself.
        for declared in [0u16, 1] {
            let mut short = intact.clone();
            short[4..6].copy_from_slice(&declared.to_be_bytes());
            assert_eq!(
                walk(&short).expect("a JPEG").1,
                Some(ScanError::InvalidLength),
                "declared length {declared}"
            );
        }

        // Segments that never reach EOI — the walk that yielded real
        // elements and still must not be read as a whole image.
        let unterminated = build(&[framed(0xDB, &[1, 2, 3])]);
        let (walked, stopped) = walk(&unterminated).expect("a JPEG");
        assert_eq!(walked.len(), 1, "the segment before the end was real");
        assert_eq!(stopped, Some(ScanError::Truncated));

        // A scan that runs off the end of the file, which is the same
        // fact reached through the other reader.
        let open_scan = build(&[framed(0xDA, &[1]), vec![0x11, 0x22]]);
        assert_eq!(
            walk(&open_scan).expect("a JPEG").1,
            Some(ScanError::Truncated)
        );
        let dangling = build(&[framed(0xDA, &[1]), vec![0x11, MARK]]);
        assert_eq!(
            walk(&dangling).expect("a JPEG").1,
            Some(ScanError::Truncated),
            "a 0xFF with nothing behind it is not a marker yet"
        );

        // A byte where a marker belongs, a fill byte between segments,
        // and stuffing outside the one place stuffing means anything.
        for (label, trailing) in [
            ("a plain byte", vec![0x42, 0x42]),
            ("a fill byte", vec![MARK, MARK, MARK, 0xDB]),
            ("stuffing", vec![MARK, 0x00]),
        ] {
            let bytes = build(&[framed(0xDB, &[1]), trailing]);
            assert_eq!(
                walk(&bytes).expect("a JPEG").1,
                Some(ScanError::NotAMarker),
                "{label}"
            );
        }

        // Not a JPEG at all: refused up front rather than per segment.
        assert_eq!(
            segments(b"\x89PNG\r\n\x1a\n").err(),
            Some(ScanError::NotJpeg)
        );
        assert_eq!(segments(&[MARK, SOI]).err(), Some(ScanError::NotJpeg));
        assert_eq!(segments(b"").err(), Some(ScanError::NotJpeg));
        assert!(!is_jpeg(b"\x89PNG\r\n\x1a\n"));
        assert!(is_jpeg(&intact));
    }

    #[test]
    fn the_walk_stops_at_the_segment_ceiling() {
        let mut parts: Vec<Vec<u8>> = (0..=MAX_SEGMENTS).map(|_| bare(TEM)).collect();
        parts.push(bare(EOI));
        let bytes = build(&parts);
        let (walked, stopped) = walk(&bytes).expect("a JPEG");
        assert_eq!(walked.len(), MAX_SEGMENTS);
        assert_eq!(stopped, Some(ScanError::TooManySegments));
    }

    /// A real encoder's output walks to its end marker, stuffing and
    /// all.
    ///
    /// Every other fixture here is hand-built, which proves the reader
    /// against bytes written by the same understanding that wrote the
    /// reader. This one is written by `image`'s JPEG encoder — a segment
    /// set nobody here chose — over **noise**, deliberately: a blank
    /// frame compresses to a scan smaller than the Huffman tables in
    /// front of it and contains no stuffed byte at all, so it would
    /// exercise none of the part that is hard. Noise puts `0xFF` bytes
    /// in the entropy stream, which the encoder has to stuff, which is
    /// the thing a naive walk stops at.
    #[test]
    fn an_encoders_own_output_walks_to_its_end_marker() {
        let mut state = 0x1234_5678u32;
        let noise = image::RgbImage::from_fn(64, 64, |_, _| {
            let mut channel = || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            };
            image::Rgb([channel(), channel(), channel()])
        });
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(noise)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .expect("encode");
        let bytes = buf.into_inner();

        let (walked, stopped) = walk(&bytes).expect("a JPEG");
        assert_eq!(stopped, None, "an encoder's own output has no defect");
        assert_eq!(walked.last(), Some(&Element::Bare(EOI)));

        let scan: &[u8] = walked
            .iter()
            .find_map(|e| match e {
                Element::Scan(data) => Some(*data),
                _ => None,
            })
            .expect("the picture's own bytes are in a scan");
        // What makes this fixture worth encoding rather than writing:
        // the encoder put stuffed bytes in its own output, and they are
        // inside the span this walk reported.
        assert!(
            scan.windows(2).any(|pair| pair == [MARK, 0x00]),
            "the fixture has to carry a stuffed byte for walking past one to prove anything"
        );
        // And the scan is the bulk of the file, which is what says the
        // walk did not stop at the first of them.
        assert!(
            scan.len() * 2 > bytes.len(),
            "{} of {}",
            scan.len(),
            bytes.len()
        );
    }
}
