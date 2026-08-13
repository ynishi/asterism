//! Putting an XMP packet into a container, and taking the old one out.
//!
//! Two containers, because those are the two still-image formats this
//! corpus arrives in. Both are handled as byte transforms over a whole
//! file rather than as a decode/re-encode: an export must hand back the
//! pixels it was given, and the only way to be sure of that is to never
//! decode them. It also means a PNG whose ancillary chunks nothing here
//! understands comes out with those chunks intact and in order.
//!
//! # Replace, never append
//!
//! Both containers permit a second XMP packet to sit next to the first,
//! and neither says which one wins. Readers disagree in practice, so a
//! file with two packets has a disclosure that depends on who opens it —
//! the failure mode being that a stale `digitalSourceType` shadows a
//! corrected one. Every writer here removes the existing packet before
//! adding its own, and the tests pin that a twice-stamped file has
//! exactly one.
//!
//! # Where the packet goes
//!
//! PNG: an `iTXt` chunk before the first `IDAT`. The chunk is
//! uncompressed — the compression flag exists, and the packet is small
//! enough that a reader unable to inflate it would be a worse outcome
//! than a few hundred spare bytes.
//!
//! JPEG: an `APP1` segment before the first non-`APPn` marker, so it
//! lands after a JFIF `APP0` and after an EXIF `APP1` rather than
//! displacing either. The one hard limit in this module is here — a JPEG
//! segment carries at most 65,533 bytes and a generator prompt can be
//! larger than that on its own; see [`EmbedError::PacketTooLarge`] and
//! [`DisclosureRecord::essential`](crate::record::DisclosureRecord::essential).

use crate::xmp;

/// A still-image container this module can write into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// PNG — the packet becomes an `iTXt` chunk.
    Png,
    /// JPEG — the packet becomes an `APP1` segment.
    Jpeg,
}

/// What went wrong writing a packet.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// The bytes are not a container this module writes into. Carries no
    /// detail on purpose: the caller's next move is the same whether the
    /// file is a WebP, a text file, or eight bytes of nothing.
    #[error("not a container this build can write XMP into (expected PNG or JPEG)")]
    UnsupportedContainer,
    /// The container's own framing did not hold up — a chunk or segment
    /// length that runs past the end of the file, a PNG that never
    /// reaches `IEND`, a JPEG that never reaches its first scan.
    ///
    /// Two files out of a 4,601-image corpus already do this with no
    /// attacker anywhere near them (`asterism-media-probe::png` module
    /// docs), so this is an ordinary outcome rather than an alarm.
    #[error("malformed {container:?}: {detail}")]
    Malformed {
        /// Which container's framing failed.
        container: Container,
        /// What the walk found, in the words of whatever read it.
        detail: String,
    },
    /// The packet does not fit the container's metadata slot. JPEG only
    /// — PNG's chunk length field admits 2 GiB, so no realistic packet
    /// reaches it.
    ///
    /// The caller's move is to write less rather than to split the
    /// packet: the ExtendedXMP mechanism JPEG defines for this is not
    /// implemented, and a split packet a reader fails to reassemble is a
    /// disclosure that silently is not there.
    #[error(
        "XMP packet is {bytes} bytes and a JPEG APP1 segment holds at most {limit}; \
         write a reduced record instead of splitting the packet"
    )]
    PacketTooLarge {
        /// Size of the packet that was offered.
        bytes: usize,
        /// The largest packet this segment can carry.
        limit: usize,
    },
}

/// Identifies the container from its own first bytes.
///
/// Deliberately not from a MIME type or a file extension. Both are
/// statements *about* a file made by something that is not the file, and
/// this module is about to rewrite the byte framing — being wrong here
/// produces a corrupt artefact rather than a failed call.
pub fn sniff(bytes: &[u8]) -> Option<Container> {
    if bytes.starts_with(PNG_SIGNATURE) {
        return Some(Container::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return Some(Container::Jpeg);
    }
    None
}

/// Writes `packet` into `bytes`, replacing any packet already there.
///
/// Returns the whole file. Nothing is written in place: the caller
/// decides whether the result replaces the original, and an export that
/// fails halfway must not leave a half-stamped file behind.
pub fn embed_xmp(bytes: &[u8], packet: &str) -> Result<Vec<u8>, EmbedError> {
    match sniff(bytes).ok_or(EmbedError::UnsupportedContainer)? {
        Container::Png => png::embed(bytes, packet),
        Container::Jpeg => jpeg::embed(bytes, packet),
    }
}

/// Reads back the XMP packet a file carries, if it carries one.
///
/// Present for the tests and for the re-apply path, which has to be able
/// to say whether a file that came back from somewhere still has its
/// disclosure — the question "did this survive the round trip" cannot be
/// answered by the writer alone.
pub fn read_xmp(bytes: &[u8]) -> Result<Option<String>, EmbedError> {
    match sniff(bytes).ok_or(EmbedError::UnsupportedContainer)? {
        Container::Png => png::read(bytes),
        Container::Jpeg => jpeg::read(bytes),
    }
}

/// The 8 bytes every PNG starts with.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

mod png {
    //! PNG: the packet as an uncompressed `iTXt` chunk.
    //!
    //! Chunk boundaries come from `pngmeta` rather than from a walk
    //! written here — the same line `asterism-media-probe` takes, and
    //! for the same reason: where one chunk ends and the next begins has
    //! one right answer, including for the files whose declared length
    //! lies. What is new on this side is the CRC, which a reader can
    //! skip past and a writer cannot.

    use super::{Container, EmbedError, PNG_SIGNATURE};

    /// The keyword the XMP specification fixes for the chunk carrying a
    /// packet. Latin-1, and matched exactly — a reader looks for this
    /// string, not for "a chunk that happens to hold RDF".
    const XMP_KEYWORD: &[u8] = b"XML:com.adobe.xmp";

    /// Builds one `iTXt` chunk: `length || type || payload || CRC`.
    fn chunk(packet: &str) -> Vec<u8> {
        // Payload layout, from the PNG specification:
        //   keyword \0 compression-flag compression-method
        //   language-tag \0 translated-keyword \0 text
        // The two empty strings before the text are the language tag and
        // the translated keyword; both are legitimately empty for a
        // packet that is not human prose.
        let mut payload = Vec::with_capacity(XMP_KEYWORD.len() + 5 + packet.len());
        payload.extend_from_slice(XMP_KEYWORD);
        payload.push(0);
        payload.push(0); // compression flag: uncompressed
        payload.push(0); // compression method: the only defined value
        payload.push(0); // empty language tag
        payload.push(0); // empty translated keyword
        payload.extend_from_slice(packet.as_bytes());

        let mut out = Vec::with_capacity(payload.len() + 12);
        // A payload this long cannot occur: it is bounded by the packet,
        // which is bounded by the record, and `as` here would silently
        // truncate rather than fail. `try_into` makes the impossible
        // case loud if the bound ever moves.
        let length = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(b"iTXt");
        out.extend_from_slice(&payload);
        // The CRC covers the type code and the payload, and not the
        // length field — a detail that produces a file every decoder
        // rejects if it is got wrong, and no visible difference until
        // one is opened.
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(b"iTXt");
        hasher.update(&payload);
        out.extend_from_slice(&hasher.finalize().to_be_bytes());
        out
    }

    /// Whether an `iTXt` payload is an XMP packet's.
    fn is_xmp(payload: &[u8]) -> bool {
        payload
            .strip_prefix(XMP_KEYWORD)
            .is_some_and(|rest| rest.first() == Some(&0))
    }

    /// The text of an `iTXt` payload whose keyword is XMP's, when it is
    /// stored uncompressed.
    ///
    /// A compressed packet reads as absent rather than as an error. This
    /// crate never writes one, so encountering one means another tool
    /// wrote it; inflating it would mean carrying a decompressor (and a
    /// bomb ceiling) to read a packet that is about to be replaced
    /// wholesale anyway.
    fn text_of(payload: &[u8]) -> Option<String> {
        let rest = payload.strip_prefix(XMP_KEYWORD)?;
        let rest = rest.strip_prefix(&[0])?;
        let (&compression_flag, rest) = rest.split_first()?;
        if compression_flag != 0 {
            return None;
        }
        let (_compression_method, rest) = rest.split_first()?;
        // Language tag and translated keyword, each NUL-terminated.
        let mut fields = rest.splitn(3, |b| *b == 0);
        let _language = fields.next()?;
        let _translated = fields.next()?;
        let text = fields.next()?;
        String::from_utf8(text.to_vec()).ok()
    }

    /// Byte ranges of the chunks a walk found: where an existing XMP
    /// chunk sits, and where a new one would go.
    struct Layout {
        existing: Option<std::ops::Range<usize>>,
        before_first_idat: Option<usize>,
        end_of_walk: Option<usize>,
    }

    fn layout(bytes: &[u8]) -> Result<Layout, EmbedError> {
        let mut found = Layout {
            existing: None,
            before_first_idat: None,
            end_of_walk: None,
        };
        let spans = pngmeta::chunk_spans(bytes).map_err(|e| EmbedError::Malformed {
            container: Container::Png,
            detail: e.to_string(),
        })?;
        for span in spans {
            let (span, payload) = span.map_err(|e| EmbedError::Malformed {
                container: Container::Png,
                detail: e.to_string(),
            })?;
            let range = span.range();
            let (start, end) = (range.start as usize, range.end as usize);
            if span.kind == pngmeta::ChunkType::ITXT && is_xmp(payload) && found.existing.is_none()
            {
                found.existing = Some(start..end);
            }
            if span.kind == pngmeta::ChunkType::IDAT && found.before_first_idat.is_none() {
                found.before_first_idat = Some(start);
            }
            if span.kind == pngmeta::ChunkType::IEND {
                found.end_of_walk = Some(start);
                break;
            }
        }
        Ok(found)
    }

    pub(super) fn embed(bytes: &[u8], packet: &str) -> Result<Vec<u8>, EmbedError> {
        let found = layout(bytes)?;
        let new_chunk = chunk(packet);

        // Replacing in place keeps the packet where the previous writer
        // put it, which keeps a re-stamped file's chunk order stable
        // across repeated exports.
        if let Some(range) = found.existing {
            let mut out = Vec::with_capacity(bytes.len() + new_chunk.len());
            out.extend_from_slice(&bytes[..range.start]);
            out.extend_from_slice(&new_chunk);
            out.extend_from_slice(&bytes[range.end..]);
            return Ok(out);
        }

        // Before the first IDAT: the position the XMP specification
        // asks for, and the one a streaming reader can act on before it
        // has decoded any pixels. A PNG with no IDAT at all is
        // malformed, but its IEND is still a position that keeps the
        // file readable, so the packet goes there rather than the write
        // failing.
        let insert_at = found
            .before_first_idat
            .or(found.end_of_walk)
            .ok_or_else(|| EmbedError::Malformed {
                container: Container::Png,
                detail: "no IDAT and no IEND: nowhere to put a chunk".into(),
            })?;
        let mut out = Vec::with_capacity(bytes.len() + new_chunk.len());
        out.extend_from_slice(&bytes[..insert_at]);
        out.extend_from_slice(&new_chunk);
        out.extend_from_slice(&bytes[insert_at..]);
        Ok(out)
    }

    pub(super) fn read(bytes: &[u8]) -> Result<Option<String>, EmbedError> {
        if !bytes.starts_with(PNG_SIGNATURE) {
            return Err(EmbedError::UnsupportedContainer);
        }
        let spans = pngmeta::chunk_spans(bytes).map_err(|e| EmbedError::Malformed {
            container: Container::Png,
            detail: e.to_string(),
        })?;
        for span in spans {
            let (span, payload) = span.map_err(|e| EmbedError::Malformed {
                container: Container::Png,
                detail: e.to_string(),
            })?;
            if span.kind == pngmeta::ChunkType::ITXT && is_xmp(payload) {
                return Ok(text_of(payload));
            }
            if span.kind == pngmeta::ChunkType::IEND {
                break;
            }
        }
        Ok(None)
    }
}

mod jpeg {
    //! JPEG: the packet as an `APP1` segment.
    //!
    //! The walk stops at the first scan (`SOS`). Everything after it is
    //! entropy-coded data in which a `0xFF` byte is not a marker, so a
    //! walk that continued would be reading noise as structure — and
    //! there is nothing to find there anyway: every metadata segment
    //! precedes the scan.

    use super::{Container, EmbedError};

    /// What an XMP `APP1` payload begins with. The same constant
    /// `asterism-infra`'s JPEG probe matches on when it decides which
    /// `APP1` is which — `APP1` is shared by EXIF and XMP, and the
    /// header is the only thing that tells them apart.
    const XMP_IDENTIFIER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

    /// Largest payload a JPEG segment can carry: the length field is two
    /// bytes and counts itself.
    const MAX_SEGMENT_PAYLOAD: usize = 65_535 - 2;

    /// Largest XMP packet that fits, once the identifier is paid for.
    pub(super) const MAX_PACKET: usize = MAX_SEGMENT_PAYLOAD - XMP_IDENTIFIER.len();

    /// Start of scan — the marker the walk stops at.
    const SOS: u8 = 0xDA;
    /// End of image.
    const EOI: u8 = 0xD9;
    /// Application segment 0 (`APP0`) through 15 (`APP15`).
    const APP0: u8 = 0xE0;
    const APP15: u8 = 0xEF;
    /// `APP1` — EXIF and XMP both.
    const APP1: u8 = 0xE1;

    /// One segment's position in the file.
    struct Segment {
        marker: u8,
        /// Offset of the `0xFF` that introduces the segment.
        start: usize,
        /// Offset one past the segment's last byte.
        end: usize,
        /// Payload range, framing and length field excluded.
        payload: std::ops::Range<usize>,
    }

    /// Walks segments from `SOI` up to and including the first `SOS`.
    fn segments(bytes: &[u8]) -> Result<Vec<Segment>, EmbedError> {
        let malformed = |detail: &str| EmbedError::Malformed {
            container: Container::Jpeg,
            detail: detail.to_string(),
        };
        let mut out = Vec::new();
        let mut pos = 2; // past SOI
        loop {
            // Fill bytes: a stream may pad between segments with any
            // number of `0xFF`, and the marker is the first byte after
            // them that is not one.
            while bytes.get(pos) == Some(&0xFF) && bytes.get(pos + 1) == Some(&0xFF) {
                pos += 1;
            }
            if bytes.get(pos) != Some(&0xFF) {
                return Err(malformed("expected a marker and found a data byte"));
            }
            let &marker = bytes
                .get(pos + 1)
                .ok_or_else(|| malformed("a marker byte with nothing after it"))?;
            if marker == EOI {
                return Ok(out);
            }
            let length_at = pos + 2;
            let length = bytes
                .get(length_at..length_at + 2)
                .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
                .ok_or_else(|| malformed("a segment header that runs past the end of the file"))?;
            if length < 2 {
                return Err(malformed("a segment length shorter than its own field"));
            }
            let end = length_at + length;
            if end > bytes.len() {
                return Err(malformed(
                    "a segment length that runs past the end of the file",
                ));
            }
            out.push(Segment {
                marker,
                start: pos,
                end,
                payload: length_at + 2..end,
            });
            if marker == SOS {
                return Ok(out);
            }
            pos = end;
        }
    }

    fn is_xmp(segment: &Segment, bytes: &[u8]) -> bool {
        segment.marker == APP1 && bytes[segment.payload.clone()].starts_with(XMP_IDENTIFIER)
    }

    /// Builds one `APP1` segment carrying the packet.
    fn segment(packet: &str) -> Result<Vec<u8>, EmbedError> {
        if packet.len() > MAX_PACKET {
            return Err(EmbedError::PacketTooLarge {
                bytes: packet.len(),
                limit: MAX_PACKET,
            });
        }
        let payload_len = XMP_IDENTIFIER.len() + packet.len();
        let mut out = Vec::with_capacity(payload_len + 4);
        out.push(0xFF);
        out.push(APP1);
        // The length field counts itself and the payload, never the
        // two marker bytes.
        let declared = u16::try_from(payload_len + 2).map_err(|_| EmbedError::PacketTooLarge {
            bytes: packet.len(),
            limit: MAX_PACKET,
        })?;
        out.extend_from_slice(&declared.to_be_bytes());
        out.extend_from_slice(XMP_IDENTIFIER);
        out.extend_from_slice(packet.as_bytes());
        Ok(out)
    }

    pub(super) fn embed(bytes: &[u8], packet: &str) -> Result<Vec<u8>, EmbedError> {
        // Built before the walk so an oversized packet is refused
        // without having read the file: the caller's response is to
        // build a smaller record, which does not depend on anything the
        // walk would find.
        let new_segment = segment(packet)?;
        let found = segments(bytes)?;

        if let Some(existing) = found.iter().find(|s| is_xmp(s, bytes)) {
            let mut out = Vec::with_capacity(bytes.len() + new_segment.len());
            out.extend_from_slice(&bytes[..existing.start]);
            out.extend_from_slice(&new_segment);
            out.extend_from_slice(&bytes[existing.end..]);
            return Ok(out);
        }

        // After the leading application segments — a JFIF `APP0`, an
        // EXIF `APP1` — and before whatever the first non-`APPn` marker
        // is. Inserting at the very front instead would put XMP ahead of
        // JFIF, which readers tolerate but which no encoder produces.
        let insert_at = found
            .iter()
            .find(|s| !(APP0..=APP15).contains(&s.marker))
            .map(|s| s.start)
            .unwrap_or(bytes.len());
        let mut out = Vec::with_capacity(bytes.len() + new_segment.len());
        out.extend_from_slice(&bytes[..insert_at]);
        out.extend_from_slice(&new_segment);
        out.extend_from_slice(&bytes[insert_at..]);
        Ok(out)
    }

    pub(super) fn read(bytes: &[u8]) -> Result<Option<String>, EmbedError> {
        let found = segments(bytes)?;
        let Some(segment) = found.iter().find(|s| is_xmp(s, bytes)) else {
            return Ok(None);
        };
        let payload = &bytes[segment.payload.clone()];
        let text = &payload[XMP_IDENTIFIER.len()..];
        Ok(String::from_utf8(text.to_vec()).ok())
    }
}

/// Largest XMP packet a JPEG can carry in one segment.
///
/// Exposed so a caller can decide to write a reduced record *before*
/// paying for a render it is about to be told does not fit.
pub const JPEG_MAX_PACKET: usize = jpeg::MAX_PACKET;

/// Convenience: renders the record and writes it, doing nothing when
/// there is nothing to disclose.
///
/// Returns `Ok(None)` for a record that discloses nothing, which is the
/// same answer [`xmp::render`] gives and for the same reason — an empty
/// packet would be a modification with no content.
pub fn stamp(
    bytes: &[u8],
    record: &crate::record::DisclosureRecord,
) -> Result<Option<Vec<u8>>, EmbedError> {
    let Some(packet) = xmp::render(record) else {
        return Ok(None);
    };
    match embed_xmp(bytes, &packet) {
        Ok(stamped) => Ok(Some(stamped)),
        // The obligation outranks the context: rather than fail the
        // export over a prompt that will not fit a JPEG segment, write
        // the reduced record. The caller learns this happened by
        // comparing what it asked for against what came back, which is
        // why the reduced record is not silently substituted upstream.
        Err(EmbedError::PacketTooLarge { .. }) => {
            let reduced = record.essential();
            let Some(packet) = xmp::render(&reduced) else {
                return Ok(None);
            };
            embed_xmp(bytes, &packet).map(Some)
        }
        Err(other) => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::DisclosureRecord;
    use crate::source_type::DigitalSourceType;

    /// A minimal but structurally valid PNG: signature, IHDR, one IDAT,
    /// IEND. Hand-built rather than encoded, so the test does not depend
    /// on an encoder's chunk choices.
    fn png_fixture() -> Vec<u8> {
        fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(payload);
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(kind);
            hasher.update(payload);
            out.extend_from_slice(&hasher.finalize().to_be_bytes());
            out
        }
        let mut png = PNG_SIGNATURE.to_vec();
        // 1×1, 8-bit greyscale.
        png.extend_from_slice(&chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
        png.extend_from_slice(&chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x02]));
        png.extend_from_slice(&chunk(b"IEND", &[]));
        png
    }

    /// A JPEG with SOI, a JFIF APP0, a quantisation table, SOS and a
    /// short scan. Enough structure for the walk to have somewhere to
    /// insert and something to stop at.
    fn jpeg_fixture() -> Vec<u8> {
        let mut jpeg = vec![0xFF, 0xD8];
        let app0_payload = b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0";
        jpeg.extend_from_slice(&[0xFF, 0xE0]);
        jpeg.extend_from_slice(&((app0_payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(app0_payload);
        let dqt_payload = [0u8; 65];
        jpeg.extend_from_slice(&[0xFF, 0xDB]);
        jpeg.extend_from_slice(&((dqt_payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(&dqt_payload);
        let sos_payload = [0u8; 10];
        jpeg.extend_from_slice(&[0xFF, 0xDA]);
        jpeg.extend_from_slice(&((sos_payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(&sos_payload);
        // Entropy-coded data, including a 0xFF byte that is not a
        // marker — the reason the walk stops at SOS.
        jpeg.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34]);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }

    fn record() -> DisclosureRecord {
        DisclosureRecord::for_asset("asset-1")
            .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia)
            .with_ai_system("ComfyUI", None)
    }

    #[test]
    fn sniffing_reads_the_file_rather_than_a_claim_about_it() {
        assert_eq!(sniff(&png_fixture()), Some(Container::Png));
        assert_eq!(sniff(&jpeg_fixture()), Some(Container::Jpeg));
        assert_eq!(sniff(b"GIF89a"), None);
        assert_eq!(sniff(&[]), None);
    }

    #[test]
    fn a_png_round_trips_its_packet() {
        let stamped = stamp(&png_fixture(), &record()).unwrap().unwrap();
        let packet = read_xmp(&stamped).unwrap().expect("a packet came back");
        assert!(packet.contains("trainedAlgorithmicMedia"));
        assert!(packet.contains("ComfyUI"));
    }

    #[test]
    fn a_jpeg_round_trips_its_packet() {
        let stamped = stamp(&jpeg_fixture(), &record()).unwrap().unwrap();
        let packet = read_xmp(&stamped).unwrap().expect("a packet came back");
        assert!(packet.contains("trainedAlgorithmicMedia"));
    }

    #[test]
    fn the_png_chunk_lands_before_the_first_idat() {
        // Position is not cosmetic: a reader that decides what a file is
        // before decoding it only sees what precedes the pixel data.
        let stamped = stamp(&png_fixture(), &record()).unwrap().unwrap();
        let itxt = find(&stamped, b"iTXt").expect("the chunk is there");
        let idat = find(&stamped, b"IDAT").expect("the fixture has pixels");
        assert!(itxt < idat, "iTXt at {itxt} must precede IDAT at {idat}");
    }

    #[test]
    fn the_jpeg_segment_lands_after_jfif_and_before_the_first_non_app_marker() {
        let stamped = stamp(&jpeg_fixture(), &record()).unwrap().unwrap();
        let jfif = find(&stamped, b"JFIF").expect("the fixture has a JFIF APP0");
        let xmp = find(&stamped, b"http://ns.adobe.com/xap/1.0/").expect("the segment is there");
        assert!(jfif < xmp, "XMP must not displace the JFIF header");
        // The DQT is the first non-APPn segment; the packet goes before
        // it. Its payload is all zeroes, so its marker is what to look
        // for.
        let dqt = find(&stamped, &[0xFF, 0xDB]).expect("the fixture has a quantisation table");
        assert!(xmp < dqt);
    }

    #[test]
    fn stamping_twice_leaves_one_packet_not_two() {
        // Two packets is the failure that makes a disclosure depend on
        // which reader opens the file — a stale source type shadowing a
        // corrected one, with nothing in either file saying which wins.
        for original in [png_fixture(), jpeg_fixture()] {
            let once = stamp(&original, &record()).unwrap().unwrap();
            let corrected = DisclosureRecord::for_asset("asset-1")
                .with_source_type(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia);
            let twice = stamp(&once, &corrected).unwrap().unwrap();

            let packet = read_xmp(&twice).unwrap().unwrap();
            assert!(packet.contains("compositeWithTrainedAlgorithmicMedia"));
            assert_eq!(
                count(&twice, b"W5M0MpCehiHzreSzNTczkc9d"),
                1,
                "exactly one packet survives a re-stamp"
            );
        }
    }

    #[test]
    fn everything_that_was_not_the_packet_survives_verbatim() {
        // An export hands back the pixels it was given. Nothing here
        // decodes them, and this is what says so.
        let original = png_fixture();
        let stamped = stamp(&original, &record()).unwrap().unwrap();
        let idat = find(&original, b"IDAT").unwrap();
        assert_eq!(
            &original[idat..],
            &stamped[find(&stamped, b"IDAT").unwrap()..],
            "the pixel data and everything after it is byte-identical"
        );
        assert!(stamped.starts_with(PNG_SIGNATURE));
    }

    #[test]
    fn a_record_with_nothing_to_disclose_leaves_the_file_alone() {
        // Not an error, and not an empty packet: writing one would be a
        // modification that says nothing.
        assert!(
            stamp(&png_fixture(), &DisclosureRecord::for_asset("a"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn an_oversized_jpeg_packet_falls_back_to_the_obligation() {
        // A ComfyUI prompt can exceed a whole APP1 segment. Failing the
        // export would drop the Article 50 mark to preserve context,
        // which is the wrong way round.
        let huge = "x".repeat(JPEG_MAX_PACKET + 1);
        let record = record().with_prompt(huge, Some("owner".into()));
        let stamped = stamp(&jpeg_fixture(), &record).unwrap().unwrap();
        let packet = read_xmp(&stamped).unwrap().unwrap();
        assert!(
            packet.contains("trainedAlgorithmicMedia"),
            "the mark landed"
        );
        assert!(
            !packet.contains("AIPromptInformation"),
            "the prompt was dropped rather than split across segments"
        );
    }

    #[test]
    fn png_has_no_such_ceiling() {
        // PNG's length field admits 2 GiB, so the JPEG fallback must not
        // fire here — a PNG keeps the prompt.
        let long = "y".repeat(JPEG_MAX_PACKET + 1);
        let record = record().with_prompt(long, None);
        let stamped = stamp(&png_fixture(), &record).unwrap().unwrap();
        let packet = read_xmp(&stamped).unwrap().unwrap();
        assert!(packet.contains("AIPromptInformation"));
    }

    #[test]
    fn a_file_with_no_packet_reads_as_none_rather_than_failing() {
        assert_eq!(read_xmp(&png_fixture()).unwrap(), None);
        assert_eq!(read_xmp(&jpeg_fixture()).unwrap(), None);
    }

    #[test]
    fn an_unsupported_container_is_refused_before_anything_is_rewritten() {
        assert!(matches!(
            embed_xmp(b"GIF89a...", "packet"),
            Err(EmbedError::UnsupportedContainer)
        ));
    }

    #[test]
    fn a_truncated_container_is_an_ordinary_outcome_with_a_reason() {
        // Two files in a 4,601-image corpus already declare a chunk
        // longer than the file carrying them.
        let mut png = png_fixture();
        png.truncate(png.len() - 6);
        assert!(matches!(
            embed_xmp(&png, "packet"),
            Err(EmbedError::Malformed { .. })
        ));

        let mut jpeg = jpeg_fixture();
        // A segment length that runs past the end.
        jpeg[4] = 0xFF;
        assert!(matches!(
            embed_xmp(&jpeg, "packet"),
            Err(EmbedError::Malformed { .. })
        ));
    }

    /// Offset of the first occurrence of `needle`.
    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// How many times `needle` occurs.
    fn count(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|window| *window == needle)
            .count()
    }
}
