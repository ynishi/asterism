//! Reading a container's declared chapter list through an external
//! `ffmpeg`, and turning ffmetadata's `[CHAPTER]` blocks into spans on
//! the app's own timeline.
//!
//! # Why ffmpeg, and why not ffprobe
//!
//! Every container that declares chapters spells it differently — MP4's
//! `chpl` / chapter track, Matroska's `Chapters` segment, an MP3's
//! ID3 `CHAP` frames — and ffmpeg's demuxers already read all of them
//! into one shape. `-f ffmetadata` is that shape printed out, so this
//! module parses one grammar instead of three container formats.
//!
//! `ffprobe -show_chapters` prints the same facts as JSON and would be
//! the obvious tool, but **the bundled sidecar does not contain it**:
//! `scripts/build-ffmpeg-sidecar.sh` configures with `--disable-ffprobe`,
//! so on a clean machine — no Homebrew, the case the sidecar exists for
//! — the JSON route reads chapters from nothing. The binary that ships
//! is the binary this parses the output of.
//!
//! The binary is located through
//! [`thumb_ffmpeg::ffmpeg_binary`](crate::jobs::thumb_ffmpeg::ffmpeg_binary)
//! rather than through a probe of this module's own, so the sidecar
//! beside the executable outranks `PATH` here exactly as it does for
//! thumbnails. A second copy of that order is the drift the preview e2e
//! already paid for once.
//!
//! # What the parser refuses, and why refusing is not dropping quietly
//!
//! A `[CHAPTER]` block this reading cannot represent — no `TIMEBASE`, a
//! negative `START`, an `END` before its `START` — is left out of the
//! band **and named in [`ChapterReading::refused`]**, which the handler
//! puts in the job's own message. The alternative, inventing a plausible
//! value, would put a timestamp the file never declared into a band
//! whose entire claim is that it says what the file says.

use std::process::Command;

use asterism_core::application_support::ScannedChapter;
use asterism_core::domain::material_mark::TimelineSpan;

use crate::jobs::thumb_ffmpeg::ffmpeg_binary;

/// What one reading of a material's chapter list came back with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChapterReading {
    /// The sections, in the order the container declared them.
    pub chapters: Vec<ScannedChapter>,
    /// One line per section the file declared and this reading could
    /// not represent, saying which and why.
    ///
    /// Carried out rather than logged in place: the handler's return
    /// string is stored on the job row, so "this file declares six
    /// chapters and two of them are unreadable" is answerable later by
    /// somebody who did not have a log open at the time.
    pub refused: Vec<String>,
}

/// The outcome of asking a file for its chapters.
///
/// The distinction is the one `ChapterScan` is built on: [`Read`] leaves
/// a band behind — an empty one when the file declares nothing — and
/// [`Unreadable`] leaves the material in the backfill walk. Recording an
/// empty band for a file nobody opened would answer "does this declare
/// chapters?" permanently on the strength of a missing binary or an
/// unmounted volume.
///
/// [`Read`]: ChapterProbe::Read
/// [`Unreadable`]: ChapterProbe::Unreadable
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChapterProbe {
    /// ffmpeg opened the file and this is what it declared.
    Read(ChapterReading),
    /// Nothing was learned about the file. Carries the reason for the
    /// job message; the material stays in the walk.
    Unreadable(String),
}

/// Asks the file at `path_str` for its chapter list.
///
/// Runs synchronously — call inside `spawn_blocking`, like the frame
/// grab it sits beside.
///
/// A file that simply has no chapters is `Read` with an empty list, not
/// `Unreadable`: ffmpeg exits zero and prints a header with no
/// `[CHAPTER]` block, which is a complete answer.
pub fn read_chapters(path_str: &str) -> ChapterProbe {
    let Some(bin) = ffmpeg_binary() else {
        return ChapterProbe::Unreadable(format!(
            "{path_str}: reading chapters needs ffmpeg, which was not found — \
             install it (e.g. `brew install ffmpeg`) or point $ASTERISM_FFMPEG at a binary"
        ));
    };
    // `-f ffmetadata -` writes the metadata to stdout and no media at
    // all, so this costs a demux of the container's header rather than
    // a pass over its frames. No `-c copy` and no output file: the
    // muxer being asked for carries only metadata.
    let output = match Command::new(&bin)
        .args(["-v", "error", "-i", path_str])
        .args(["-f", "ffmetadata", "-"])
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            return ChapterProbe::Unreadable(format!("{path_str}: ffmpeg spawn failed: {err}"));
        }
    };
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return ChapterProbe::Unreadable(format!("{path_str}: ffmpeg could not read it: {detail}"));
    }
    // Lossy on purpose. ffmetadata is UTF-8 by spec, but a chapter title
    // copied verbatim out of a container written by something careless
    // is not guaranteed to be — and a title with a replacement character
    // in it is a better answer than refusing to read a file's whole
    // chapter list over one bad byte.
    ChapterProbe::Read(parse_chapters(&String::from_utf8_lossy(&output.stdout)))
}

/// Turns the text of an ffmetadata document into spans on the playback
/// timeline.
///
/// Pure, and the reason it is worth keeping pure is the unit tests below
/// it: every shape this has to survive — a missing title, a zero-length
/// section, a timebase in nanoseconds, a container that declares its
/// sections out of order — is a string, and none of them need a file.
///
/// # The grammar, as much of it as matters here
///
/// A document is lines. `;` or `#` opens a comment, `[NAME]` opens a
/// section, and everything else is `key=value`. A backslash escapes the
/// next character, which is how a value carries a literal `=`, `;`, `#`,
/// `\` or newline. Keys before the first section belong to the file as a
/// whole — which is why a top-level `title=` must not be mistaken for a
/// chapter's, and why this walks sections rather than grepping for
/// `title`.
///
/// # `TIMEBASE` and the conversion this function owes the domain
///
/// `START` and `END` are integers in units of `TIMEBASE` seconds, and
/// the unit is per file rather than per format. Measured against this
/// repository's own fixtures, both of which were *written* with
/// millisecond chapter times: the Matroska one reads back as
/// `TIMEBASE=1/1000000000` and the MP4 one as `TIMEBASE=1/1000`. So a
/// parser that assumed milliseconds would be right on one of the two
/// and wrong by a factor of a million on the other — which is why a
/// block that declares no `TIMEBASE` is refused rather than assumed.
///
/// [`TimelineSpan`] documents that "values on another origin must be
/// converted before they arrive here". The multiply below is this
/// reading's half of that contract.
///
/// The division truncates, so a division point never lands later than
/// the file puts it.
pub fn parse_chapters(metadata: &str) -> ChapterReading {
    let mut reading = ChapterReading::default();
    let mut open: Option<RawChapter> = None;
    let close = |open: &mut Option<RawChapter>, reading: &mut ChapterReading| {
        if let Some(raw) = open.take() {
            match raw.into_scanned() {
                Ok(chapter) => reading.chapters.push(chapter),
                Err(why) => reading.refused.push(why),
            }
        }
    };
    for line in lines(metadata) {
        match line {
            Line::Section(name) => {
                close(&mut open, &mut reading);
                // Case-insensitively, because the key names are the
                // muxer's convention rather than a guarantee: ffmpeg
                // writes `[CHAPTER]`, and a document typed by hand or
                // produced by another tool is not obliged to shout.
                if name.eq_ignore_ascii_case("chapter") {
                    open = Some(RawChapter::default());
                }
            }
            Line::Pair(key, value) => {
                // Only inside a chapter block. A `title` in the global
                // header is the file's name for itself.
                let Some(raw) = open.as_mut() else { continue };
                if key.eq_ignore_ascii_case("timebase") {
                    raw.timebase = Some(value);
                } else if key.eq_ignore_ascii_case("start") {
                    raw.start = Some(value);
                } else if key.eq_ignore_ascii_case("end") {
                    raw.end = Some(value);
                } else if key.eq_ignore_ascii_case("title") {
                    raw.title = Some(value);
                }
            }
        }
    }
    close(&mut open, &mut reading);
    reading
}

/// One `[CHAPTER]` block as read, before anything is believed about it.
///
/// Values stay as strings until the whole block is in hand because the
/// meaning of `START` depends on a `TIMEBASE` that may be declared after
/// it.
#[derive(Debug, Default)]
struct RawChapter {
    timebase: Option<String>,
    start: Option<String>,
    end: Option<String>,
    title: Option<String>,
}

impl RawChapter {
    fn into_scanned(self) -> Result<ScannedChapter, String> {
        let label = self.title.unwrap_or_default();
        let named = if label.is_empty() {
            "an untitled section".to_string()
        } else {
            format!("section {label:?}")
        };
        let Some(timebase) = self.timebase.as_deref() else {
            return Err(format!(
                "{named}: no TIMEBASE, so its timestamps have no unit"
            ));
        };
        let (num, den) = parse_timebase(timebase)
            .ok_or_else(|| format!("{named}: TIMEBASE {timebase:?} is not a usable ratio"))?;
        let Some(start) = self.start.as_deref() else {
            return Err(format!("{named}: no START, so it marks no division"));
        };
        // `u64` rather than a signed parse and a range check: a negative
        // offset is a position on some *other* origin (a container start
        // PTS), and `TimelineSpan` states that its zero is the playback
        // timeline's. Reading one in would silently file a mark against
        // the wrong clock.
        let start_ms = to_ms(start, num, den)
            .ok_or_else(|| format!("{named}: START {start:?} is not a timestamp this can carry"))?;
        let end_ms = match self.end.as_deref() {
            None => None,
            Some(end) => {
                let end_ms = to_ms(end, num, den).ok_or_else(|| {
                    format!("{named}: END {end:?} is not a timestamp this can carry")
                })?;
                // Equal ends are an ordinary declaration, not a fault: a
                // zero-length chapter is how several muxers spell "the
                // section starts here", and `TimelineSpan` has a
                // spelling for exactly that. Below the start is not the
                // same thing — the block contradicts itself, and there
                // is no way to tell which of the two numbers is the
                // wrong one, so the section goes out whole rather than
                // being half-believed.
                if end_ms == start_ms {
                    None
                } else if end_ms < start_ms {
                    return Err(format!(
                        "{named}: END {end_ms} ms is before START {start_ms} ms"
                    ));
                } else {
                    Some(end_ms)
                }
            }
        };
        let span = TimelineSpan::new(start_ms, end_ms).map_err(|err| format!("{named}: {err}"))?;
        Ok(ScannedChapter { span, label })
    }
}

/// `num/den`, both positive.
///
/// A bare integer is refused rather than read as `n/1`: that spelling
/// does not occur in ffmpeg's output, and guessing at it would turn a
/// malformed field into timestamps 90000 times too large without anybody
/// being told.
fn parse_timebase(raw: &str) -> Option<(u64, u64)> {
    let (num, den) = raw.trim().split_once('/')?;
    let num: u64 = num.trim().parse().ok()?;
    let den: u64 = den.trim().parse().ok()?;
    (num != 0 && den != 0).then_some((num, den))
}

/// `value × num ÷ den` seconds, in milliseconds.
///
/// Computed in `u128` and refused on overflow rather than wrapped: the
/// product of a large timestamp and a large numerator does leave `u64`,
/// and a wrapped value is a plausible-looking timestamp in the middle of
/// a file — the one failure mode a reader could not spot.
fn to_ms(raw: &str, num: u64, den: u64) -> Option<u64> {
    let value: u64 = raw.trim().parse().ok()?;
    let ms = u128::from(value)
        .checked_mul(u128::from(num))?
        .checked_mul(1_000)?
        / u128::from(den);
    u64::try_from(ms).ok()
}

/// One meaningful line of an ffmetadata document.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// `[NAME]`, with the brackets stripped.
    Section(String),
    /// `key=value`, both unescaped.
    Pair(String, String),
}

/// Splits a document into [`Line`]s, resolving escapes as it goes.
///
/// Escapes are resolved *during* the split rather than after it because
/// they change where the split falls: an escaped `=` is part of a title,
/// and an escaped newline does not end the line. A pass that split first
/// and unescaped afterwards would cut a title in half at the first
/// `\=` — which is precisely the character a chapter called "Act 1 = the
/// arrival" contains.
fn lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        match chars.peek() {
            None => break,
            // Blank lines and the leftovers of `\r\n`.
            Some('\n' | '\r') => {
                chars.next();
                continue;
            }
            // `;FFMETADATA1` — the magic line — is a comment like any
            // other, so the version is not checked. A document this
            // could not read would fail at the section level anyway,
            // and refusing on an unrecognised version number would mean
            // a newer ffmpeg silently stops filling chapter bands.
            Some(';' | '#') => {
                skip_to_newline(&mut chars);
                continue;
            }
            Some('[') => {
                chars.next();
                let mut name = String::new();
                for c in chars.by_ref() {
                    if c == ']' || c == '\n' {
                        break;
                    }
                    name.push(c);
                }
                skip_to_newline(&mut chars);
                out.push(Line::Section(name));
                continue;
            }
            _ => {}
        }
        let mut key = String::new();
        let mut separated = false;
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        key.push(escaped);
                    }
                }
                '=' => {
                    separated = true;
                    break;
                }
                '\n' => break,
                _ => key.push(c),
            }
        }
        // A line with no separator states nothing; it is not an error
        // either, since a document may carry trailing whitespace or a
        // convention this does not know about.
        if !separated {
            continue;
        }
        let mut value = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        value.push(escaped);
                    }
                }
                '\n' => break,
                _ => value.push(c),
            }
        }
        out.push(Line::Pair(
            key.trim_end_matches('\r').to_string(),
            value.trim_end_matches('\r').to_string(),
        ));
    }
    out
}

fn skip_to_newline(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for c in chars.by_ref() {
        if c == '\n' {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What ffmpeg actually prints for a two-chapter Matroska file,
    /// down to the magic line and the global title.
    const MATROSKA_LIKE: &str = "\
;FFMETADATA1
title=A recording
encoder=Lavf60.16.100
[CHAPTER]
TIMEBASE=1/1000
START=0
END=2000
title=Opening
[CHAPTER]
TIMEBASE=1/1000
START=2000
END=6000
title=Finale
[STREAM]
title=not a chapter
";

    #[test]
    fn reads_the_sections_a_container_declares() {
        let reading = parse_chapters(MATROSKA_LIKE);
        assert!(reading.refused.is_empty(), "{:?}", reading.refused);
        assert_eq!(reading.chapters.len(), 2, "the [STREAM] block is not one");
        assert_eq!(reading.chapters[0].label, "Opening");
        assert_eq!(reading.chapters[0].span.start_ms(), 0);
        assert_eq!(reading.chapters[0].span.end_ms(), Some(2_000));
        assert_eq!(reading.chapters[1].label, "Finale");
        assert_eq!(reading.chapters[1].span.start_ms(), 2_000);
        assert_eq!(reading.chapters[1].span.end_ms(), Some(6_000));
    }

    /// The global `title` belongs to the file, and a `[STREAM]` block's
    /// belongs to a stream. Neither is a chapter, and the only thing
    /// keeping them out is that this walks sections.
    #[test]
    fn a_title_outside_a_chapter_block_is_not_a_chapter() {
        let reading = parse_chapters(
            "
;FFMETADATA1
title=A recording
[STREAM]
title=Video
",
        );
        assert!(reading.chapters.is_empty());
        assert!(reading.refused.is_empty(), "nothing was declared to refuse");
    }

    /// A file that declares nothing reads as an empty list — which the
    /// intake writes as an empty band, not as no band.
    #[test]
    fn a_document_with_no_chapter_blocks_reads_as_an_empty_list() {
        let reading = parse_chapters(";FFMETADATA1\ntitle=A recording\n");
        assert_eq!(reading, ChapterReading::default());
    }

    /// `TIMEBASE` is a ratio of seconds, so the same file expressed in
    /// three of them has to land on the same milliseconds.
    #[test]
    fn timestamps_are_converted_out_of_the_container_timebase() {
        let at = |timebase: &str, start: &str, end: &str| {
            let doc =
                format!("[CHAPTER]\nTIMEBASE={timebase}\nSTART={start}\nEND={end}\ntitle=One\n");
            let reading = parse_chapters(&doc);
            assert!(reading.refused.is_empty(), "{:?}", reading.refused);
            let span = reading.chapters[0].span;
            (span.start_ms(), span.end_ms())
        };
        // Milliseconds, the Matroska spelling.
        assert_eq!(at("1/1000", "1500", "4500"), (1_500, Some(4_500)));
        // 90 kHz, the MPEG one.
        assert_eq!(at("1/90000", "135000", "405000"), (1_500, Some(4_500)));
        // Nanoseconds, and a numerator that is not 1.
        assert_eq!(
            at("1/1000000000", "1500000000", "4500000000"),
            (1_500, Some(4_500))
        );
        assert_eq!(at("2/1000", "750", "2250"), (1_500, Some(4_500)));
    }

    /// Truncation, stated as a test so a later change to rounding is a
    /// decision rather than an accident: a division point never moves
    /// later than the file puts it.
    #[test]
    fn a_timestamp_between_milliseconds_truncates_rather_than_rounds() {
        let reading = parse_chapters("[CHAPTER]\nTIMEBASE=1/90000\nSTART=179\ntitle=One\n");
        // 179 / 90000 s = 1.988… ms.
        assert_eq!(reading.chapters[0].span.start_ms(), 1);
    }

    /// Both shapes `ChapterMark` documents itself as accepting, arriving
    /// the way a container states them.
    #[test]
    fn an_untitled_section_and_one_with_no_end_are_both_read() {
        let reading = parse_chapters(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=1000\ntitle=Two\n",
        );
        assert!(reading.refused.is_empty(), "{:?}", reading.refused);
        assert_eq!(reading.chapters[0].label, "", "a file may declare no title");
        assert_eq!(reading.chapters[0].span.end_ms(), Some(1_000));
        assert!(
            reading.chapters[1].span.is_instant(),
            "no END means the section starts here and states no end"
        );
    }

    /// `END == START` is a zero-length declaration, which the domain
    /// spells as an instant rather than as an interval covering nothing.
    #[test]
    fn a_section_whose_end_equals_its_start_is_an_instant() {
        let reading =
            parse_chapters("[CHAPTER]\nTIMEBASE=1/1000\nSTART=5000\nEND=5000\ntitle=Mark\n");
        assert!(reading.refused.is_empty(), "{:?}", reading.refused);
        assert!(reading.chapters[0].span.is_instant());
        assert_eq!(reading.chapters[0].span.start_ms(), 5_000);
    }

    /// The reading order is the file's, including when the file states
    /// its sections out of timeline order — `ord` is assigned by
    /// position in this list, so the order here is the order stored.
    #[test]
    fn sections_keep_the_order_the_container_declared_them_in() {
        let reading = parse_chapters(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART=60000\ntitle=Later\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=10000\ntitle=Earlier\n",
        );
        assert_eq!(
            reading
                .chapters
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            ["Later", "Earlier"]
        );
    }

    /// Every block this reading cannot represent, and the requirement
    /// that each is *named* rather than dropped quietly.
    #[test]
    fn a_block_that_cannot_be_represented_is_refused_by_name() {
        let cases: [(&str, &str); 6] = [
            // No unit for the numbers.
            ("[CHAPTER]\nSTART=0\nEND=1000\ntitle=NoBase\n", "TIMEBASE"),
            // A unit that is not a ratio, and one that divides by zero.
            (
                "[CHAPTER]\nTIMEBASE=1000\nSTART=0\ntitle=Bare\n",
                "usable ratio",
            ),
            (
                "[CHAPTER]\nTIMEBASE=1/0\nSTART=0\ntitle=ZeroDen\n",
                "usable ratio",
            ),
            // A position on another clock.
            (
                "[CHAPTER]\nTIMEBASE=1/1000\nSTART=-500\ntitle=Negative\n",
                "START",
            ),
            // Self-contradictory.
            (
                "[CHAPTER]\nTIMEBASE=1/1000\nSTART=9000\nEND=1000\ntitle=Inverted\n",
                "before START",
            ),
            // Past what the column can hold: 2^63 ms is beyond
            // `TimelineSpan`'s storable range, and the multiply itself
            // stays inside `u128`.
            (
                "[CHAPTER]\nTIMEBASE=1/1\nSTART=18446744073709551615\ntitle=Huge\n",
                "carry",
            ),
        ];
        for (doc, expected) in cases {
            let reading = parse_chapters(doc);
            assert!(
                reading.chapters.is_empty(),
                "nothing should be believed from {doc:?}"
            );
            assert_eq!(reading.refused.len(), 1, "one refusal for {doc:?}");
            assert!(
                reading.refused[0].contains(expected),
                "refusal for {doc:?} should say why, got {:?}",
                reading.refused[0]
            );
        }
    }

    /// A refusal is per section: the rest of the file still lands.
    ///
    /// The alternative — one bad block failing the whole read — would
    /// leave the material in the backfill walk forever, re-opened on
    /// every pass for an answer that cannot change.
    #[test]
    fn one_unrepresentable_section_does_not_cost_the_others() {
        let reading = parse_chapters(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=Good\n\
             [CHAPTER]\nSTART=1000\ntitle=Bad\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=2000\ntitle=AlsoGood\n",
        );
        assert_eq!(
            reading
                .chapters
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            ["Good", "AlsoGood"]
        );
        assert_eq!(reading.refused.len(), 1);
        assert!(reading.refused[0].contains("Bad"));
    }

    /// The escapes ffmetadata defines, in the field most likely to carry
    /// them. A title with an `=` in it is the case that decides whether
    /// escapes are resolved during the split or after it.
    #[test]
    fn escapes_are_resolved_without_moving_the_split() {
        let reading = parse_chapters(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\ntitle=Act 1 \\= the arrival\\; part \\#2\n",
        );
        assert!(reading.refused.is_empty(), "{:?}", reading.refused);
        assert_eq!(reading.chapters[0].label, "Act 1 = the arrival; part #2");
    }

    /// An escaped newline continues the value, which is how a container
    /// carries a two-line title.
    #[test]
    fn an_escaped_newline_keeps_the_value_open() {
        let reading =
            parse_chapters("[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\ntitle=First\\\nSecond\n");
        assert!(reading.refused.is_empty(), "{:?}", reading.refused);
        assert_eq!(reading.chapters[0].label, "First\nSecond");
    }

    /// CRLF and a comment mid-document, since neither is this parser's
    /// choice to make about a file it did not write.
    #[test]
    fn carriage_returns_and_comments_do_not_reach_the_values() {
        let reading = parse_chapters(
            ";FFMETADATA1\r\n[CHAPTER]\r\nTIMEBASE=1/1000\r\nSTART=0\r\n\
             ; a note from the muxer\r\ntitle=Opening\r\n",
        );
        assert!(reading.refused.is_empty(), "{:?}", reading.refused);
        assert_eq!(reading.chapters[0].label, "Opening");
    }

    /// Key names are matched case-insensitively, because their casing is
    /// a muxer's convention rather than a guarantee.
    #[test]
    fn the_key_names_are_matched_without_regard_to_case() {
        let reading =
            parse_chapters("[chapter]\ntimebase=1/1000\nstart=1000\nend=3000\nTITLE=Mixed\n");
        assert!(reading.refused.is_empty(), "{:?}", reading.refused);
        assert_eq!(reading.chapters[0].label, "Mixed");
        assert_eq!(reading.chapters[0].span.start_ms(), 1_000);
        assert_eq!(reading.chapters[0].span.end_ms(), Some(3_000));
    }

    /// A block cut off by the end of the document is still a block: the
    /// close runs at EOF as well as at the next section header.
    #[test]
    fn the_last_block_is_closed_by_the_end_of_the_document() {
        let reading = parse_chapters("[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\ntitle=Only");
        assert_eq!(reading.chapters.len(), 1);
        assert_eq!(reading.chapters[0].label, "Only");
    }

    /// An empty document is not an error — it is a file that declares
    /// nothing, which is the answer for most of the library.
    #[test]
    fn an_empty_document_reads_as_nothing_declared() {
        assert_eq!(parse_chapters(""), ChapterReading::default());
    }
}
