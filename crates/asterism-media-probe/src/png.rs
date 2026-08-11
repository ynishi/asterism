//! PNG chunk framing: where one chunk ends, where the next begins, and
//! what the text chunks say.
//!
//! Every function here is a pure function of a byte slice over input an
//! importer collected from outside, with the failure modes a parser has
//! (truncation, a length field that lies, an unbounded chunk count).
//! **Nothing here decides what a chunk means.** Which chunks belong to a
//! picture's identity, which are metadata, what gets hashed and in what
//! order — those are judgements about a corpus, and they are made by the
//! caller that has one.
//!
//! # Why the two readers are two readers
//!
//! [`chunks`] and [`text_fields`] walk the same chunk sequence and are
//! written separately anyway. Their callers select opposite halves of
//! it, and a shared walk parameterised by which is which would be one
//! function whose two callers each hope the flag was passed the right
//! way round. They also disagree about what a defect is: a `tEXt`
//! payload with no NUL separator is skipped by [`text_fields`] and is
//! nothing at all to [`chunks`], while a chunk sequence that ends
//! without `IEND` stops both.
//!
//! # Reading files from outside
//!
//! Every length in a PNG is four bytes inside the file, so it is
//! whatever the file says it is, and reading one correctly is a solved
//! problem this module does not re-solve twice over: [`chunks`] takes
//! its boundaries from `pngmeta`, which borrows each payload out of the
//! input and reports a length that does not fit as a typed error rather
//! than a stop. A chunk claiming 4 GiB inside a 30-byte file is refused
//! on the declared length, before anything is sized from it.
//!
//! Truncation is not hypothetical: 2 files out of a 4,601-image corpus
//! declare a chunk longer than the file that carries it, with no
//! attacker anywhere near them.
//!
//! CRCs are read past rather than verified. A wrong CRC is a fact about
//! the file that a whole-file digest already distinguishes, and refusing
//! to read a file every decoder still displays would cost a real answer
//! for nothing.

use std::collections::BTreeMap;

/// Ceiling on chunks walked in one file.
///
/// Every chunk costs at least 12 real bytes, so the count is already
/// bounded by the input; the ceiling bounds what a walk *keeps* —
/// whatever list of borrowed payloads the caller is accumulating, at
/// most this many fat pointers — independently of how the input was
/// produced. libpng writes `IDAT` in 8 KiB pieces, so this admits a PNG
/// of about half a gigabyte before the ceiling is the thing that stops
/// the walk, which is far past the point where holding the file in
/// memory is the larger problem.
///
/// A parser's number rather than a caller's, which is why it moved here
/// with the parser: the other ceiling — a chunk's declared length
/// against PNG's own 2^31-1 limit — is `pngmeta`'s (`MAX_CHUNK_LENGTH`),
/// and this one sits directly above it in the same walk.
///
/// # The allocation it bounds is in another crate
///
/// Nothing here holds a chunk, so nothing here gets larger when this
/// number does — which makes it look free to raise. It is not. The
/// caller in this workspace is `asterism-infra`'s PNG probe, which
/// borrows one `&[u8]` per `IDAT` payload into a `Vec` and feeds the
/// whole list to a hash at the end of the walk, so this is the length of
/// that `Vec`. It is pinned from that side too, by a test that spells
/// the number out instead of importing it
/// (`probes::png::tests::the_pixel_list_this_probe_accumulates_cannot_outgrow_its_ceiling`),
/// so raising the ceiling here fails there rather than quietly widening
/// an allocation nobody on this side can see.
pub const MAX_CHUNKS: usize = 65_536;

/// The 8 bytes every PNG starts with (`\x89PNG\r\n\x1a\n`).
const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Length field + chunk type, in bytes.
const CHUNK_HEADER: usize = 8;

/// Trailing CRC of every chunk, in bytes.
const CHUNK_CRC: usize = 4;

/// The uncompressed text chunk: `keyword \0 text`, both Latin-1 by the
/// spec.
const TEXT: &[u8; 4] = b"tEXt";

/// End of the chunk sequence.
const IEND: &[u8; 4] = b"IEND";

/// Ceiling on one chunk's declared length: the PNG format's own limit
/// (the high bit of the length field must be zero).
const MAX_CHUNK_LEN: usize = 0x7fff_ffff;

/// One chunk as the file carries it.
///
/// The type is the four raw bytes rather than a parsed enum, because a
/// caller that feeds a chunk to a hash feeds exactly these four — a
/// vocabulary that mapped unknown types onto one variant would erase the
/// difference between two chunks nobody has heard of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk<'a> {
    /// The chunk's four-byte type code, e.g. `b"IDAT"`.
    pub kind: [u8; 4],
    /// The payload, borrowed out of the input. Never the length field
    /// and never the CRC.
    pub payload: &'a [u8],
    /// The whole chunk as the file carries it — `length (4) || type (4)
    /// || payload || CRC (4)` — borrowed out of the input.
    ///
    /// Beside [`payload`](Self::payload) rather than instead of it,
    /// because the two answer different questions and both callers here
    /// are real. A caller hashing a chunk wants the payload and the type
    /// and deliberately not the length, since encoders split one stream
    /// at different boundaries. A caller **keeping** a chunk wants every
    /// byte of it: what makes kept bytes worth keeping is that they can
    /// be walked again later by a reader that disagrees with today's
    /// about what they mean, and a sequence with the lengths taken out
    /// cannot be walked at all.
    ///
    /// Reassembling this from the other two fields is not the same
    /// thing: the length would have to be re-derived and the CRC
    /// recomputed, so a chunk whose stored CRC is wrong — a real file,
    /// which this walk reads past rather than refusing — would come back
    /// different from the one that went in.
    pub frame: &'a [u8],
}

/// Why a walk stopped short of the end of the chunk sequence.
///
/// The variants are worth having even for a caller that treats all of
/// them alike: a reader of a stack trace or a future diagnostic can tell
/// which happened, where a boolean could only say "stopped".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanError {
    /// The first eight bytes are not the PNG signature, or the input is
    /// shorter than eight bytes.
    NotPng,
    /// A chunk declares more bytes than the input actually holds, or the
    /// input ended without an `IEND` chunk.
    Truncated,
    /// A chunk declares a length past PNG's own 2^31-1 maximum —
    /// reported before anything is sized from it.
    InvalidLength,
    /// The sequence ran past [`MAX_CHUNKS`].
    TooManyChunks,
}

/// True when `bytes` starts with the PNG signature.
pub fn is_png(bytes: &[u8]) -> bool {
    pngmeta::is_png(bytes)
}

/// Walks the chunk sequence, yielding every chunk in the order the file
/// carries it.
///
/// `Err(ScanError::NotPng)` up front when the signature is wrong;
/// otherwise the iterator yields chunks until one of them fails, and a
/// failure is the last item. **A caller must not treat a partial walk as
/// a whole one** — the sequence stops on the first defect, so what came
/// before it is part of a file rather than a file.
///
/// The iterator keeps no per-chunk state beyond a count, so the memory a
/// walk costs is whatever the caller chooses to hold on to. That is also
/// where [`MAX_CHUNKS`] earns its place: it bounds the accumulation the
/// caller is doing, at the one point in the walk that can see how far
/// the sequence has run.
pub fn chunks(bytes: &[u8]) -> Result<Chunks<'_>, ScanError> {
    let spans = pngmeta::chunk_spans(bytes).map_err(|_| ScanError::NotPng)?;
    Ok(Chunks {
        bytes,
        spans,
        walked: 0,
    })
}

/// Iterator returned by [`chunks`].
#[derive(Debug)]
pub struct Chunks<'a> {
    /// The whole input, kept so a chunk can be handed back as the file
    /// carries it ([`Chunk::frame`]) and not only as the parts a hash
    /// wants. The spans below are offsets into exactly this slice.
    bytes: &'a [u8],
    spans: pngmeta::ChunkSpans<'a>,
    walked: usize,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>, ScanError>;

    fn next(&mut self) -> Option<Self::Item> {
        let span = self.spans.next()?;
        self.walked += 1;
        if self.walked > MAX_CHUNKS {
            return Some(Err(ScanError::TooManyChunks));
        }
        Some(match span {
            Ok((span, payload)) => match frame_of(self.bytes, &span) {
                Some(frame) => Ok(Chunk {
                    kind: span.kind.0,
                    payload,
                    frame,
                }),
                // A span handed to this iterator maps into the input it
                // was walked from — but that is **`pngmeta`'s property
                // and not this module's**, and `chunk_spans` does not
                // state it as a contract; it is read off the
                // implementation, which rejects a chunk longer than
                // what remains before yielding it. So this arm is not
                // called unreachable. What holds it is on this side:
                // `tests::a_chunks_frame_is_the_bytes_the_file_carries`
                // (every frame of a whole file is a slice of the input,
                // and the frames in order are the input) and
                // `tests::a_tail_the_input_does_not_hold_never_comes_back_as_a_chunk`
                // (a short tail ends the walk rather than arriving).
                // An upstream bump that loosened either turns those red.
                //
                // `Truncated` rather than a panic because this runs in a
                // background pass over somebody's library, where
                // refusing one file is survivable and a panic is not.
                // The cost is real and is stated where it lands: a
                // stopped walk reads as `ContentRegion::EmptySpan` in
                // `asterism-infra`'s PNG probe, so a file that holds a
                // digest today would take a marker — which is the
                // reason the property above is pinned rather than
                // trusted.
                None => Err(ScanError::Truncated),
            },
            Err(pngmeta::Error::NotPng) => Err(ScanError::NotPng),
            Err(pngmeta::Error::InvalidLength { .. }) => Err(ScanError::InvalidLength),
            Err(_) => Err(ScanError::Truncated),
        })
    }
}

/// The bytes one span covers — `length || type || payload || CRC` — or
/// `None` when the input does not hold all of them.
fn frame_of<'a>(bytes: &'a [u8], span: &pngmeta::ChunkSpan) -> Option<&'a [u8]> {
    let start = usize::try_from(span.offset).ok()?;
    let len = usize::try_from(span.total_len).ok()?;
    bytes.get(start..start.checked_add(len)?)
}

/// Collects the `tEXt` chunks out of a PNG as `keyword → text`.
///
/// `None` means the input is not a PNG, or its chunk sequence never
/// reached its end marker — so what was collected is part of a file
/// rather than a file, and a caller must not read it as "this file
/// carries no more than these".
///
/// Nothing here is sized from a declared length: a payload is a subslice
/// of the input, so a chunk claiming 4 GiB inside a 30-byte file simply
/// fails to fit and ends the walk. What the walk keeps is bounded by
/// [`MAX_CHUNKS`].
///
/// # The two decodings this fixes in place
///
/// A `tEXt` payload is Latin-1 by the spec and arbitrary bytes in
/// practice, so it is read with [`String::from_utf8_lossy`] — a total
/// function with one answer per input, which is what a digest taken over
/// the result needs. A repeated keyword collapses, last occurrence
/// winning, because the return is a map and the reader that fixed this
/// shape (`pngmeta::read_text_chunks`) returns one too. Neither is the
/// only defensible reading; both are fixed here so that a consumer's
/// question "what does this mean" has one place to be answered.
///
/// `zTXt` and `iTXt` are not read. Both may be compressed, so reading
/// them means deciding how each decodes to a string, and a decision made
/// carelessly there redefines what the result is rather than widening
/// it.
///
/// # Why this walk is written out rather than delegated
///
/// `pngmeta::read_text_chunks` answers the same question from a `Path`.
/// This caller already holds the bytes — they were read once to answer
/// several questions at a time — and handing a path back would be a
/// second read of the same file, chosen for tidiness.
pub fn text_fields(bytes: &[u8]) -> Option<BTreeMap<String, String>> {
    if !bytes.starts_with(SIGNATURE) {
        return None;
    }
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    let mut rest = &bytes[SIGNATURE.len()..];
    let mut walked = 0usize;

    loop {
        if rest.len() < CHUNK_HEADER {
            return None;
        }
        let (header, body) = rest.split_at(CHUNK_HEADER);
        let declared = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind = &header[4..CHUNK_HEADER];

        if declared > MAX_CHUNK_LEN {
            return None;
        }
        // The declared length is a claim about the file, so it is used
        // only to ask whether that many bytes are actually there.
        let consumed = declared.checked_add(CHUNK_CRC)?;
        if body.len() < consumed {
            return None;
        }
        let payload = &body[..declared];

        walked += 1;
        if walked > MAX_CHUNKS {
            return None;
        }

        if kind == TEXT
            && let Some(separator) = payload.iter().position(|byte| *byte == 0)
        {
            // A keyword is 1–79 bytes by the spec; an empty one names
            // nothing and is dropped rather than stored under `""`.
            let keyword = String::from_utf8_lossy(&payload[..separator]).into_owned();
            if !keyword.is_empty() {
                let text = String::from_utf8_lossy(&payload[separator + 1..]).into_owned();
                fields.insert(keyword, text);
            }
        }

        rest = &body[consumed..];
        if kind == IEND {
            break;
        }
    }

    Some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Built = (&'static [u8; 4], Vec<u8>);

    /// PNG's CRC-32 (the standard reflected polynomial), so the fixtures
    /// stay valid files.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn build(chunks: &[Built]) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        for (kind, payload) in chunks {
            let length = u32::try_from(payload.len()).expect("fixture chunk fits in a PNG length");
            out.extend_from_slice(&length.to_be_bytes());
            out.extend_from_slice(*kind);
            out.extend_from_slice(payload);
            let mut crc_input = kind.to_vec();
            crc_input.extend_from_slice(payload);
            out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        }
        out
    }

    fn text(keyword: &str, value: &str) -> Built {
        let mut payload = keyword.as_bytes().to_vec();
        payload.push(0);
        payload.extend_from_slice(value.as_bytes());
        (TEXT, payload)
    }

    fn ihdr() -> Built {
        (b"IHDR", vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0])
    }

    /// Walks to the end and returns what was yielded, so a test can
    /// assert on both the chunks and the stop.
    fn walk(bytes: &[u8]) -> Result<(Vec<Chunk<'_>>, Option<ScanError>), ScanError> {
        let mut out = Vec::new();
        let mut stopped = None;
        for item in chunks(bytes)? {
            match item {
                Ok(chunk) => out.push(chunk),
                Err(err) => {
                    stopped = Some(err);
                    break;
                }
            }
        }
        Ok((out, stopped))
    }

    #[test]
    fn every_chunk_comes_back_in_file_order_with_its_payload_borrowed() {
        let bytes = build(&[
            ihdr(),
            text("prompt", "a cat"),
            (b"IDAT", vec![1, 2, 3]),
            (b"IDAT", vec![4, 5]),
            (b"IEND", Vec::new()),
        ]);
        let (walked, stopped) = walk(&bytes).expect("a PNG");
        assert_eq!(stopped, None);
        assert_eq!(
            walked.iter().map(|c| c.kind).collect::<Vec<_>>(),
            [*b"IHDR", *b"tEXt", *b"IDAT", *b"IDAT", *b"IEND"]
        );
        // The order of the two pixel chunks is the file's, not a set's.
        assert_eq!(walked[2].payload, &[1, 2, 3]);
        assert_eq!(walked[3].payload, &[4, 5]);
        // Neither the length field nor the CRC is part of a payload —
        // the whole reason a caller can hash a payload directly.
        assert_eq!(walked[4].payload, b"");
    }

    /// **A chunk's frame is the bytes of the file, and the concatenated
    /// frames are the file without its signature.**
    ///
    /// Two halves. The first is that a frame is a subslice rather than a
    /// reassembly: it starts with the declared length, carries the type
    /// and payload, and ends with the CRC exactly as written — including
    /// a CRC this walk never verified, which is the case a rebuild would
    /// silently correct.
    ///
    /// The second is what a caller keeping frames is entitled to assume:
    /// the frames of every chunk, in order, are the input minus its
    /// eight signature bytes. Without that a kept subset is bytes with
    /// no framing, and nothing could walk them again.
    #[test]
    fn a_chunks_frame_is_the_bytes_the_file_carries() {
        let mut bytes = build(&[
            ihdr(),
            text("prompt", "a cat"),
            (b"IDAT", vec![1, 2, 3]),
            (b"IEND", Vec::new()),
        ]);
        // A CRC nobody wrote correctly, so "the frame is the file's
        // bytes" and "the frame is a recomputed chunk" stop agreeing.
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;

        let (walked, stopped) = walk(&bytes).expect("a PNG");
        assert_eq!(stopped, None);

        let text_chunk = walked
            .iter()
            .find(|chunk| chunk.kind == *TEXT)
            .expect("the fixture carries one");
        let payload: &[u8] = b"prompt\0a cat";
        assert_eq!(text_chunk.payload, payload);
        assert_eq!(
            &text_chunk.frame[..8],
            [&12u32.to_be_bytes()[..], TEXT].concat(),
            "the frame opens with the declared length and the type"
        );
        assert_eq!(&text_chunk.frame[8..8 + payload.len()], payload);
        assert_eq!(
            text_chunk.frame.len(),
            12 + payload.len(),
            "and closes with the CRC — 12 bytes of framing, no more"
        );

        let rebuilt: Vec<u8> = walked
            .iter()
            .flat_map(|chunk| chunk.frame)
            .copied()
            .collect();
        assert_eq!(
            rebuilt,
            bytes[SIGNATURE.len()..],
            "every frame in order is the input without its signature — corrupt CRC and all"
        );
    }

    /// **A chunk the input does not wholly hold never arrives as one.**
    ///
    /// The property [`Chunk::frame`] rests on, pinned on this side
    /// because it belongs to `pngmeta`: its walk rejects a chunk longer
    /// than what remains, and nothing in its documented contract
    /// promises to go on doing so. A bump that yielded a short tail as
    /// a chunk would reach `frame_of`, which cannot produce a frame for
    /// it, and a caller would read the walk as stopped — see that arm
    /// for what a stop costs downstream.
    ///
    /// Cut at three depths because they end the sequence in different
    /// places: inside the final CRC, inside a payload, and before a
    /// header is complete.
    #[test]
    fn a_tail_the_input_does_not_hold_never_comes_back_as_a_chunk() {
        let whole = build(&[
            ihdr(),
            text("prompt", "a cat"),
            (b"IDAT", vec![7; 32]),
            (b"IEND", Vec::new()),
        ]);

        for missing in [1usize, 20, 46] {
            let cut = &whole[..whole.len() - missing];
            let (walked, stopped) = walk(cut).expect("a PNG");
            assert_eq!(
                stopped,
                Some(ScanError::Truncated),
                "{missing} bytes short: the walk has to stop rather than hand back a \
                 chunk the input does not hold"
            );
            // And everything it did hand back is whole: a frame is 12
            // bytes of framing around its own payload, and the frames
            // in order are a prefix of the input after the signature —
            // so nothing was borrowed past the defect.
            assert!(
                !walked.is_empty(),
                "{missing} bytes short: the cut is past the tail, so the assertions \
                 below would hold over nothing"
            );
            let rebuilt: Vec<u8> = walked
                .iter()
                .flat_map(|chunk| chunk.frame)
                .copied()
                .collect();
            for chunk in &walked {
                assert_eq!(chunk.frame.len(), 12 + chunk.payload.len(), "{missing}");
            }
            assert!(
                cut[SIGNATURE.len()..].starts_with(&rebuilt),
                "{missing} bytes short: {} frame bytes are not a prefix of the input",
                rebuilt.len()
            );
        }
    }

    #[test]
    fn a_structural_defect_ends_the_walk_and_names_itself() {
        let intact = build(&[ihdr(), (b"IDAT", vec![7; 32]), (b"IEND", Vec::new())]);

        // A length that runs past the end of the file.
        let mut overrun = intact.clone();
        let len_at = SIGNATURE.len();
        overrun[len_at..len_at + 4].copy_from_slice(&0x0010_0000u32.to_be_bytes());
        assert_eq!(walk(&overrun).expect("a PNG").1, Some(ScanError::Truncated));

        // Cut in half.
        assert_eq!(
            walk(&intact[..intact.len() / 2]).expect("a PNG").1,
            Some(ScanError::Truncated)
        );

        // Chunks that never reach IEND — the walk that yielded real
        // chunks and still must not be read as a whole file.
        let unterminated = build(&[ihdr(), (b"IDAT", vec![7; 32])]);
        let (walked, stopped) = walk(&unterminated).expect("a PNG");
        assert_eq!(walked.len(), 2, "the chunks before the end were real");
        assert_eq!(stopped, Some(ScanError::Truncated));

        // A 4 GiB chunk declared inside a 30-byte file. Nothing is sized
        // from that number — if it were, this would be an allocation of
        // four gigabytes rather than an assertion.
        for declared in [0xffff_fff0u32, u32::MAX, 0x8000_0000] {
            let mut bomb = SIGNATURE.to_vec();
            bomb.extend_from_slice(&declared.to_be_bytes());
            bomb.extend_from_slice(b"IDAT");
            bomb.extend_from_slice(&[0u8; 14]);
            assert_eq!(bomb.len(), 30);
            assert_eq!(
                walk(&bomb).expect("a PNG").1,
                Some(ScanError::InvalidLength),
                "declared length {declared:#x}"
            );
        }

        // Not a PNG at all: refused up front rather than per chunk.
        assert_eq!(
            chunks(b"\xff\xd8\xff\xe0 a jpeg").err(),
            Some(ScanError::NotPng)
        );
        assert_eq!(chunks(&SIGNATURE[..7]).err(), Some(ScanError::NotPng));
        assert!(!is_png(b"\xff\xd8\xff\xe0 a jpeg"));
        assert!(is_png(&intact));
    }

    #[test]
    fn the_walk_stops_at_the_chunk_ceiling() {
        let mut swarm: Vec<Built> = vec![(b"IDAT", vec![0u8; 1])];
        swarm.extend((0..=MAX_CHUNKS).map(|_| (b"prIV", Vec::new())));
        swarm.push((b"IEND", Vec::new()));
        let bytes = build(&swarm);
        let (walked, stopped) = walk(&bytes).expect("a PNG");
        assert_eq!(walked.len(), MAX_CHUNKS);
        assert_eq!(stopped, Some(ScanError::TooManyChunks));
    }

    #[test]
    fn text_fields_reads_the_uncompressed_chunk_only_and_fixes_its_two_decodings() {
        let bytes = build(&[
            ihdr(),
            text("Comment", "first"),
            text("Comment", "second"),
            text("Empty", ""),
            // Neither of these is read: both may be compressed.
            (
                b"zTXt",
                b"workflow\0\0\x78\x9c\x03\x00\x00\x00\x00\x01".to_vec(),
            ),
            (b"iTXt", b"Description\0\0\0\0\0a caption".to_vec()),
            // A payload with no separator names nothing.
            (TEXT, b"no separator here".to_vec()),
            (b"IDAT", vec![1]),
            (b"IEND", Vec::new()),
        ]);
        let fields = text_fields(&bytes).expect("a complete chunk sequence");
        assert_eq!(fields.len(), 2, "{fields:?}");
        assert_eq!(
            fields.get("Comment").map(String::as_str),
            Some("second"),
            "the last occurrence wins"
        );
        assert_eq!(
            fields.get("Empty").map(String::as_str),
            Some(""),
            "an empty text is a value, not a dropped entry"
        );

        // Bytes that are not UTF-8 are read losslessly-in-length rather
        // than refused: one answer per input is what a digest needs.
        let mut latin1 = "caption".as_bytes().to_vec();
        latin1.push(0);
        latin1.extend_from_slice(&[0xe9, 0xff]);
        let odd = build(&[ihdr(), (TEXT, latin1), (b"IEND", Vec::new())]);
        assert_eq!(
            text_fields(&odd).expect("a PNG").get("caption"),
            Some(&"\u{fffd}\u{fffd}".to_string())
        );
    }

    #[test]
    fn text_fields_refuses_a_file_it_did_not_reach_the_end_of() {
        let whole = build(&[ihdr(), text("prompt", "a cat"), (b"IEND", Vec::new())]);
        assert!(text_fields(&whole).is_some());

        // The text was read before the end was not — and the answer is
        // still `None`, because "these are the fields" would be a claim
        // about a file this is only part of.
        let unterminated = build(&[ihdr(), text("prompt", "a cat")]);
        assert_eq!(text_fields(&unterminated), None);
        assert_eq!(text_fields(&whole[..whole.len() / 2]), None);
        assert_eq!(text_fields(b"\xff\xd8\xff\xe0 a jpeg"), None);

        let mut bomb = SIGNATURE.to_vec();
        bomb.extend_from_slice(&u32::MAX.to_be_bytes());
        bomb.extend_from_slice(TEXT);
        bomb.extend_from_slice(&[0u8; 14]);
        assert_eq!(text_fields(&bomb), None);

        let mut swarm: Vec<Built> = vec![text("workflow", "{}")];
        swarm.extend((0..=MAX_CHUNKS).map(|_| (b"prIV", Vec::new())));
        swarm.push((b"IEND", Vec::new()));
        assert_eq!(text_fields(&build(&swarm)), None);
    }
}
