# asterism-infra::jobs::chapter_ffmetadata

Reading a container's declared chapter list through an external
`ffmpeg`, and turning ffmetadata's `[CHAPTER]` blocks into spans on
the app's own timeline.

# Why ffmpeg, and why not ffprobe

Every container that declares chapters spells it differently — MP4's
`chpl` / chapter track, Matroska's `Chapters` segment, an MP3's
ID3 `CHAP` frames — and ffmpeg's demuxers already read all of them
into one shape. `-f ffmetadata` is that shape printed out, so this
module parses one grammar instead of three container formats.

`ffprobe -show_chapters` prints the same facts as JSON and would be
the obvious tool, but **the bundled sidecar does not contain it**:
`scripts/build-ffmpeg-sidecar.sh` configures with `--disable-ffprobe`,
so on a clean machine — no Homebrew, the case the sidecar exists for
— the JSON route reads chapters from nothing. The binary that ships
is the binary this parses the output of.

The binary is located through
[`thumb_ffmpeg::ffmpeg_binary`](crate::jobs::thumb_ffmpeg::ffmpeg_binary)
rather than through a probe of this module's own, so the sidecar
beside the executable outranks `PATH` here exactly as it does for
thumbnails. A second copy of that order is the drift the preview e2e
already paid for once.

# What the parser refuses, and why refusing is not dropping quietly

A `[CHAPTER]` block this reading cannot represent — no `TIMEBASE`, a
negative `START`, an `END` before its `START` — is left out of the
band **and named in [`ChapterReading::refused`]**, which the handler
puts in the job's own message. The alternative, inventing a plausible
value, would put a timestamp the file never declared into a band
whose entire claim is that it says what the file says.

## Functions

- `parse_chapters` — Turns the text of an ffmetadata document into spans on the playback
- `read_chapters` — Asks the file at `path_str` for its chapter list.

## Types

- `ChapterProbe` — The outcome of asking a file for its chapters.
- `ChapterReading` — What one reading of a material's chapter list came back with.

