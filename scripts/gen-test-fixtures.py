#!/usr/bin/env python3
"""Regenerate every binary test fixture in this workspace.

Why this script exists
----------------------

The media fixtures used to be downloaded from third-party repositories
(SillyTavern's default character card, libheif's ``example.heic``, a Big
Buck Bunny clip, freesound recordings mirrored in ``ArtskydJ/test-audio``
…) and were then committed. Two things were wrong with that:

* the tests' own doc comments claimed the files were *not* committed and
  had to be fetched on demand, which had stopped being true;
* several of those upstreams ship under licences (AGPL-3.0, LGPL-3.0,
  CC-BY) that this MIT-OR-Apache-2.0 repository cannot simply
  redistribute, and none of them carried an attribution notice here.

Everything under ``crates/*/tests/fixtures/`` is now produced by this
script from synthetic sources — ffmpeg's ``testsrc`` / ``sine`` filters
and a character card written byte by byte below — so the committed
fixtures have no upstream and no licence to honour beyond this repo's.

Usage
-----

    python3 scripts/gen-test-fixtures.py            # regenerate all
    python3 scripts/gen-test-fixtures.py --check    # verify presence only

Requirements: ``ffmpeg`` on PATH (Homebrew's build is fine) and, for the
HEIC fixture, macOS ``sips`` — ffmpeg has no HEIF muxer. The generated
bytes are not expected to be bit-identical across encoder versions; what
the tests assert is the *shape* (dimensions, duration, codec slug), and
those are pinned by the arguments below.

Any change to a dimension, duration or codec here has a matching
assertion in the corresponding ``parser.rs`` test module.
"""

from __future__ import annotations

import argparse
import base64
import json
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CRATES = REPO / "crates"

AUDIO_DIR = CRATES / "asterism-importer-audio" / "tests" / "fixtures"
IMAGE_DIR = CRATES / "asterism-importer-image" / "tests" / "fixtures"
VIDEO_DIR = CRATES / "asterism-importer-video" / "tests" / "fixtures"
CARD_DIR = CRATES / "asterism-importer-sdk" / "tests" / "fixtures"

# Every fixture this script owns, so --check can report a missing one
# without regenerating (and so a reader can see the full set at once).
EXPECTED = [
    AUDIO_DIR / "tone.mp3",
    AUDIO_DIR / "tone.m4a",
    AUDIO_DIR / "tone.wav",
    AUDIO_DIR / "tone.flac",
    AUDIO_DIR / "tone.ogg",
    IMAGE_DIR / "testcard.gif",
    IMAGE_DIR / "testcard.tiff",
    IMAGE_DIR / "testcard.bmp",
    IMAGE_DIR / "testcard.avif",
    IMAGE_DIR / "testcard.heic",
    VIDEO_DIR / "testsrc.mp4",
    VIDEO_DIR / "testsrc.mov",
    VIDEO_DIR / "testsrc.webm",
    VIDEO_DIR / "chaptered.mkv",
    AUDIO_DIR / "chaptered.m4a",
    CARD_DIR / "character-card-lyra.png",
]


def run(*args: str) -> None:
    """Run a command, surfacing stderr verbatim when it fails."""
    proc = subprocess.run(args, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        raise SystemExit(f"command failed ({proc.returncode}): {' '.join(args)}")


def ffmpeg(*args: str) -> None:
    run("ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *args)


# --------------------------------------------------------------------
# Audio — one second of a 440 Hz sine, one fixture per container.
#
# The audio parser asserts a codec slug per container and a positive
# duration / sample rate; the sample rate is pinned at 44100 so the
# `sample_rate > 0` assertion is not resting on a default.
# --------------------------------------------------------------------

SINE = "sine=frequency=440:sample_rate=44100:duration=1"


def gen_audio() -> None:
    AUDIO_DIR.mkdir(parents=True, exist_ok=True)
    ffmpeg("-f", "lavfi", "-i", SINE, "-c:a", "libmp3lame", "-b:a", "64k",
           str(AUDIO_DIR / "tone.mp3"))
    # `-c:a aac` in an MP4 container; lofty reports the codec as `aac`.
    ffmpeg("-f", "lavfi", "-i", SINE, "-c:a", "aac", "-b:a", "64k",
           str(AUDIO_DIR / "tone.m4a"))
    ffmpeg("-f", "lavfi", "-i", SINE, "-c:a", "pcm_s16le",
           str(AUDIO_DIR / "tone.wav"))
    ffmpeg("-f", "lavfi", "-i", SINE, "-c:a", "flac",
           str(AUDIO_DIR / "tone.flac"))
    # Ogg carries Vorbis here (ffmpeg's native encoder, hence -strict -2);
    # the test accepts any known codec slug for the container. `-ac 2` is
    # not decoration: that encoder refuses anything but stereo, so the
    # otherwise-mono sine has to be upmixed before it reaches it.
    ffmpeg("-f", "lavfi", "-i", SINE, "-ac", "2", "-c:a", "vorbis", "-strict", "-2",
           str(AUDIO_DIR / "tone.ogg"))


# --------------------------------------------------------------------
# Images — one still frame of ffmpeg's `testsrc` pattern.
#
# The TIFF is 157×151 because the image parser pins those exact
# dimensions (it is the one format whose header read is asserted
# precisely rather than "positive"). AVIF is 64×64: SVT-AV1 will not
# encode the 1×1 frame the previous upstream fixture used.
# --------------------------------------------------------------------


def testsrc_still(size: str, out: Path, *extra: str) -> None:
    ffmpeg("-f", "lavfi", "-i", f"testsrc=size={size}:rate=1",
           "-frames:v", "1", *extra, str(out))


def gen_images() -> None:
    IMAGE_DIR.mkdir(parents=True, exist_ok=True)
    # Two frames so the GIF is animated, matching what the importer sees
    # from real animated GIFs.
    ffmpeg("-f", "lavfi", "-i", "testsrc=size=120x90:rate=2:duration=1",
           "-loop", "0", str(IMAGE_DIR / "testcard.gif"))
    testsrc_still("157x151", IMAGE_DIR / "testcard.tiff")
    testsrc_still("160x120", IMAGE_DIR / "testcard.bmp", "-pix_fmt", "bgr24")
    testsrc_still("64x64", IMAGE_DIR / "testcard.avif", "-c:v", "libsvtav1")

    # HEIC: ffmpeg has no HEIF muxer, so hand the still to macOS `sips`.
    if shutil.which("sips") is None:
        print("! skipping testcard.heic — `sips` not found (macOS only)")
        return
    staging = IMAGE_DIR / "_heic-src.png"
    testsrc_still("160x120", staging)
    run("sips", "-s", "format", "heic", str(staging),
        "--out", str(IMAGE_DIR / "testcard.heic"))
    staging.unlink()


# --------------------------------------------------------------------
# Video — `testsrc` again, one file per container probe path.
#
# mp4/mov go through the ISOBMFF probe, webm through the Matroska one,
# so the WebM fixture is the only VP9 encode and the only one whose
# duration is asserted in seconds (≈10 s). `-cpu-used 8` keeps that
# encode to a couple of seconds.
# --------------------------------------------------------------------


def gen_video() -> None:
    VIDEO_DIR.mkdir(parents=True, exist_ok=True)
    for name, extra in (
        ("testsrc.mp4", ("-c:v", "libx264", "-pix_fmt", "yuv420p")),
        ("testsrc.mov", ("-c:v", "libx264", "-pix_fmt", "yuv420p")),
    ):
        ffmpeg("-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=1",
               *extra, str(VIDEO_DIR / name))
    ffmpeg("-f", "lavfi", "-i", "testsrc=size=640x360:rate=25:duration=10",
           "-c:v", "libvpx-vp9", "-b:v", "200k", "-deadline", "realtime",
           "-cpu-used", "8", str(VIDEO_DIR / "testsrc.webm"))


# --------------------------------------------------------------------
# Chaptered media — the only fixtures that carry a chapter list.
#
# `chapter_scan` reads a container's declared chapters through
# `ffmpeg -f ffmetadata`, and every other fixture in this file is a
# negative case for it: none of them declare any. These two are the
# positive one, in the two container families that spell chapters
# differently — Matroska stores a `Chapters` segment with explicit start
# and end times, MP4 a chapter track whose entries carry starts and take
# their ends from the next entry.
#
# Chapters are baked by handing ffmpeg an ffmetadata document as a
# second input and mapping its chapters onto the output. `-map_chapters`
# is given explicitly even though ffmpeg would pick input 1 by default
# (it copies from the first input that has any): the default depends on
# neither input growing chapters later, which is not a property a
# fixture should rest on.
#
# **The timings below are managed twice** — here, and in the Rust tests
# that assert them (`asterism-infra/tests/chapter_scan_job.rs`). That is
# the same deliberate duplication the header notes for dimensions and
# durations: a fixture whose numbers are read out of the fixture proves
# nothing.
# --------------------------------------------------------------------

# Matroska: three sections over six seconds, the middle one untitled.
# The untitled one is not filler — an empty label is a shape
# `ChapterMark` documents itself as accepting, and without a fixture
# that has one the "a file may declare a section without naming it"
# path is only ever exercised by a hand-written string.
MKV_CHAPTERS = [(0, 2000, "Opening"), (2000, 4000, ""), (4000, 6000, "Finale")]

# MP4: two sections over the same six seconds. Fewer, because what this
# file is for is the other container's spelling rather than a second
# copy of the same assertions.
M4A_CHAPTERS = [(0, 3000, "Side A"), (3000, 6000, "Side B")]

CHAPTERED_SECONDS = 6


def ffmetadata(chapters: list[tuple[int, int, str]]) -> str:
    """An ffmetadata document declaring `chapters`, in milliseconds.

    `TIMEBASE=1/1000` makes the numbers here milliseconds outright, so
    the constants above read as what the tests assert rather than as
    ticks of some container's clock.
    """
    out = [";FFMETADATA1", "title=Chaptered fixture"]
    for start, end, title in chapters:
        out += ["[CHAPTER]", "TIMEBASE=1/1000", f"START={start}", f"END={end}"]
        # An empty `title=` line and no line at all are both ways to say
        # "untitled"; omitting it is what a muxer does, so that is what
        # this writes.
        if title:
            out.append(f"title={title}")
    return "\n".join(out) + "\n"


def gen_chaptered() -> None:
    VIDEO_DIR.mkdir(parents=True, exist_ok=True)
    AUDIO_DIR.mkdir(parents=True, exist_ok=True)

    # The ffmetadata document is input to ffmpeg, not a fixture, and it
    # goes to a temporary directory rather than beside the outputs. The
    # unlink at the end of this function used to be the only thing
    # removing it, so any ffmpeg failure in between left a stray
    # `_chapters.txt` inside a *tracked* fixtures directory — where the
    # next `git add -A` would sweep it into a commit as though it were
    # one of the generated files. `TemporaryDirectory` removes it on the
    # way out of the `with` block whether or not the encode succeeded.
    with tempfile.TemporaryDirectory(prefix="asterism-fixtures-") as scratch:
        meta = Path(scratch) / "chapters.txt"

        meta.write_text(ffmetadata(MKV_CHAPTERS), encoding="utf-8")
        ffmpeg("-f", "lavfi", "-i", f"testsrc=size=320x240:rate=25:duration={CHAPTERED_SECONDS}",
               "-i", str(meta), "-map", "0", "-map_metadata", "1", "-map_chapters", "1",
               "-c:v", "libx264", "-pix_fmt", "yuv420p", str(VIDEO_DIR / "chaptered.mkv"))

        meta.write_text(ffmetadata(M4A_CHAPTERS), encoding="utf-8")
        ffmpeg("-f", "lavfi",
               "-i", f"sine=frequency=440:sample_rate=44100:duration={CHAPTERED_SECONDS}",
               "-i", str(meta), "-map", "0", "-map_metadata", "1", "-map_chapters", "1",
               "-c:a", "aac", "-b:a", "64k", str(AUDIO_DIR / "chaptered.m4a"))


# --------------------------------------------------------------------
# Character card — a V2 + V3 dual-chunk PNG, written here rather than
# taken from a card site.
#
# The card subsystem reads two `tEXt` chunks (`chara` = V2, `ccv3` = V3),
# each holding base64 of the card JSON. The integration test asserts a
# composition floor: six filled text slots, at least one greeting, at
# least one doc, plus a four-entry character book. Filling all six V2
# slots is deliberate — a card that left one empty would make the
# "one Note per present slot" rule pass for the wrong reason.
#
# **Size is part of the fixture, not an accident of it.** Three tests in
# `asterism-infra` read this file as "the largest thing the corpus
# carries" rather than as a card: the metadata ceiling
# (`MAX_META_RAW_BYTES`) is justified against its tEXt payload, and the
# IDAT re-split test needs a pixel stream long enough to cut into dozens
# of chunks. A real card is roughly half a megabyte of artwork plus tens
# of kilobytes of base64 persona document, so this one is built to the
# same order of magnitude — a 4 KB card would have quietly turned all
# three of those measurements into tautologies.
# --------------------------------------------------------------------

CARD_NAME = "Lyra"

# Artwork dimensions, chosen for the *compressed* size rather than for
# the picture: 384×448 of the pattern below deflates to roughly the
# half-megabyte of IDAT a real card's portrait occupies, which is the
# number three tests in `asterism-infra` are written against.
CARD_W, CARD_H = 384, 448

# How much deterministic noise rides on top of the gradient, as a bit
# width per channel. This is the compression dial, and it is blunter
# than it looks: deflate's Huffman table is built over the whole image,
# so 4 noise bits spread across a gradient still put all 256 byte values
# in play and the stream barely shrinks (~94% of raw). The gradient is
# there to make it an image; the noise is there to stop it collapsing to
# a few KB. Changing either this or the dimensions moves the chunk
# counts asserted in
# `probes::png::tests::splitting_the_pixel_stream_does_not_move_the_digest`.
CARD_NOISE_BITS = 4


def lcg(seed: int):
    """A deterministic byte source — same fixture on every machine.

    `random` would do, but its stream is a documented-stable *sequence*
    rather than a documented-stable *algorithm*, and this file is
    committed: the point is that regenerating it on another machine, or
    in five years, produces the same bytes.
    """
    x = seed
    while True:
        x = (1103515245 * x + 12345) & 0x7FFFFFFF
        yield (x >> 16) & 0xFF


def persona_document() -> str:
    """Filler with the bulk of a real persona document.

    Cards carry a lore book whose entries are prose; the ceiling test
    cares about how many kilobytes of it end up base64'd into a tEXt
    chunk, so the text has to be that long. Generated rather than
    written out so the length is a number here instead of a wall of
    invented lore.
    """
    # 72 entries puts the two tEXt payloads together just under 40 KB,
    # which is what the real card this replaced measured (40,339). The
    # number is load-bearing in a way a filler length usually is not:
    # `probes::png` freezes it as `CARD_PNG_META_RAW_BYTES` and asserts
    # *at compile time* that the 1 MiB ceiling stays 25× clear of it, so
    # a fixture with a chattier lore book does not fail a test — it
    # stops the crate building. The real card sat 4% under that line, so
    # there was never much room to spend.
    lines = []
    for i in range(72):
        lines.append(
            f"Entry {i:03d}. Declination band {i % 90:02d}, drawer {i % 12}. "
            f"Observed on a clear night; the plate was re-exposed twice and "
            f"the second exposure is the one filed. Cross-reference the "
            f"ledger before moving it."
        )
    return " ".join(lines)

CARD_DATA = {
    "name": CARD_NAME,
    "description": (
        "A star-chart archivist who keeps every observation on local "
        "parchment and refuses to file anything she cannot re-read."
    ),
    "personality": "Precise, unhurried, allergic to unsourced claims.",
    "scenario": (
        "The observatory has lost its index. Lyra is rebuilding it one "
        "constellation at a time."
    ),
    "system_prompt": "Answer as Lyra. Cite the chart you read it from.",
    "post_history_instructions": "Never invent a catalogue number.",
    "first_mes": "You found the archive. Careful — the drawers are ordered by declination.",
    "alternate_greetings": [
        "Back again? Bring the ledger this time.",
        "Mind the lantern. Ink runs.",
    ],
    "mes_example": "<START>\n{{user}}: Which chart?\n{{char}}: Third drawer, the one that smells of dust.",
    "creator_notes": "Synthetic fixture card. Generated by scripts/gen-test-fixtures.py.",
    "tags": ["fixture", "synthetic"],
    "creator": "asterism test fixtures",
    "character_version": "1.0",
    "extensions": {},
    "character_book": {
        "name": "Observatory notes",
        "entries": [
            {"id": "e1", "keys": ["chart"], "content": "Charts are filed by declination, not by name."},
            {"id": "e2", "keys": ["lantern"], "content": "The lantern is oil; it must not sit on parchment."},
            {"id": "e3", "keys": ["ledger"], "content": "The ledger records who read which drawer, and when."},
            # The bulk entry — this is what makes the tEXt payload the
            # size a real card's is. See `persona_document`.
            {"id": "e4", "keys": ["asterism"], "content": None},
        ],
    },
}


def card_envelope(spec: str, version: str) -> str:
    """base64 of one card envelope, the way a tEXt chunk carries it."""
    data = json.loads(json.dumps(CARD_DATA))  # deep copy
    data["character_book"]["entries"][3]["content"] = persona_document()
    payload = {"spec": spec, "spec_version": version, "data": data}
    raw = json.dumps(payload, ensure_ascii=False, separators=(",", ":"))
    return base64.b64encode(raw.encode("utf-8")).decode("ascii")


def png_chunk(kind: bytes, body: bytes) -> bytes:
    return (
        struct.pack(">I", len(body))
        + kind
        + body
        + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
    )


def gen_card() -> None:
    """Write the card PNG: pixels first, then `chara` / `ccv3` tEXt.

    Written by hand instead of through an image library so the script
    keeps to the standard library. The card subsystem only reads the
    text chunks, but `asterism-infra` reads the whole file — chunk
    framing, IDAT length, metadata size — so the pixels are not filler:
    see the note above about size being part of the fixture.

    **Chunk order is IHDR / IDAT / tEXt / tEXt / IEND, and the order is
    the point.** A card is artwork that had its persona document
    written into it afterwards, so the metadata lands behind the pixel
    stream — and `probes::png::tests::a_real_file_carrying_its_text_after_the_pixels_walks`
    asserts exactly that before using the file, because a fixture with
    its tEXt in front would leave the "the walk does not stop at the
    pixels" claim untested while still passing.
    """
    CARD_DIR.mkdir(parents=True, exist_ok=True)
    noise = lcg(0xA57E_2026)
    mask = (1 << CARD_NOISE_BITS) - 1
    rows = bytearray()
    for y in range(CARD_H):
        rows.append(0)  # filter type 0 (None) for this scanline
        for x in range(CARD_W):
            # A diagonal gradient carrying `CARD_NOISE_BITS` of noise per
            # channel: the gradient keeps it looking like an image, the
            # noise keeps deflate from crushing it to nothing.
            for base in (x * 255 // CARD_W, y * 255 // CARD_H,
                         (x + y) * 255 // (CARD_W + CARD_H)):
                rows.append((base & ~mask) | (next(noise) & mask))

    png = b"\x89PNG\r\n\x1a\n"
    png += png_chunk(b"IHDR", struct.pack(">IIBBBBB", CARD_W, CARD_H, 8, 2, 0, 0, 0))
    png += png_chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += png_chunk(b"tEXt", b"chara\x00" + card_envelope("chara_card_v2", "2.0").encode("ascii"))
    png += png_chunk(b"tEXt", b"ccv3\x00" + card_envelope("chara_card_v3", "3.0").encode("ascii"))
    png += png_chunk(b"IEND", b"")
    (CARD_DIR / "character-card-lyra.png").write_bytes(png)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true",
                    help="report missing fixtures instead of regenerating")
    args = ap.parse_args()

    if args.check:
        missing = [p for p in EXPECTED if not p.exists()]
        for p in missing:
            print(f"missing: {p.relative_to(REPO)}")
        print(f"{len(EXPECTED) - len(missing)}/{len(EXPECTED)} fixtures present")
        return 1 if missing else 0

    if shutil.which("ffmpeg") is None:
        raise SystemExit("ffmpeg not found on PATH")

    gen_audio()
    gen_images()
    gen_video()
    gen_chaptered()
    gen_card()

    for p in EXPECTED:
        mark = "ok " if p.exists() else "MISSING"
        size = f"{p.stat().st_size:>9,}" if p.exists() else " " * 9
        print(f"{mark} {size}  {p.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
