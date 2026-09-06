#!/usr/bin/env bash
# Build the LGPL-clean ffmpeg sidecar that `tauri build` bundles into
# Asterism.app (bundle.externalBin, see src-tauri/tauri.conf.json).
#
# Why build instead of download: every published macOS arm64 ffmpeg
# binary (evermeet, martin-riedl, Homebrew bottle) is a GPL build —
# they carry libx264/libx265. Shipping one inside the app would put
# the whole bundle under GPL. An LGPL build needs zero external
# libraries for what this app does:
#
#   decode  : native vp8/vp9/h264/... decoders (libvpx is only needed
#             for *encoding* VP8/VP9, which we never do)
#   encode  : h264_videotoolbox (Apple system framework) + native aac
#   package : mp4 muxer with +faststart, mjpeg for thumbnails
#
# so the whole build is `./configure && make` with no dependency
# bootstrap, and the result links only /usr/lib + system frameworks
# (verified below). Full static linking is not a goal: Apple does not
# support statically linked binaries (QA1118), and what the OS ships is
# not part of FFmpeg's corresponding source to begin with — see the
# note on the link check below for why no exception is being invoked.
#
# Output: target/ffmpeg-sidecar/ffmpeg-<host-triple>
#   - under target/ on purpose: already gitignored, wiped by
#     `cargo clean` like every other build artifact, and rebuilt by
#     this script (the Justfile recipes that need it depend on it).
#   - the -<triple> suffix is Tauri's externalBin naming contract; the
#     bundler strips it when copying to Asterism.app/Contents/MacOS/.
#
# Idempotent: exits fast when the output already exists for the pinned
# version. FFMPEG_SIDECAR_FORCE=1 to rebuild.

set -euo pipefail

FFMPEG_VERSION="${FFMPEG_VERSION:-8.0}"

# The bytes this build is allowed to compile, pinned per version.
#
# Provenance is established by hand and this digest is what carries it
# into every later build: the 8.0 tarball was checked against
# `ffmpeg-8.0.tar.xz.asc` with FFmpeg's release signing key, whose
# fingerprint FCF986EA15E6E293A5644F10B4322F04D67658D8 matches the one
# published on ffmpeg.org/download.html, and gpg reported a good
# signature. Repeating that check here would mean shipping a key or
# trusting a keyserver at build time; a digest recorded after a
# verification somebody did is the stronger of the two.
#
# What it buys after that is what a pin always buys: a download that
# came back different — corrupted, intercepted, or silently re-rolled
# upstream — stops here instead of being compiled and signed into the
# app.
#
# Bumping FFMPEG_VERSION touches two files and neither one keeps the
# other honest, so edit them together: verify the new tarball's
# signature and put its digest here, and update FFMPEG-NOTICE.md, which
# restates the version, the URL, the digest and the configure list —
# and which ships inside the app, where a wrong one cannot be corrected
# after the fact.
FFMPEG_SHA256="${FFMPEG_SHA256:-b2751fccb6cc4c77708113cd78b561059b6fa904b24162fa0be2d60273d27b8e}"

root="$(cd "$(dirname "$0")/.." && pwd)"
out_dir="$root/target/ffmpeg-sidecar"
triple="$(rustc -vV | sed -n 's/^host: //p')"
out="$out_dir/ffmpeg-$triple"
stamp="$out_dir/.ffmpeg-version"

mkdir -p "$out_dir"
tarball="$out_dir/ffmpeg-$FFMPEG_VERSION.tar.xz"
src_dir="$out_dir/ffmpeg-$FFMPEG_VERSION"

# The source comes first, and it comes before the fast path rather than
# after it, so that a run which builds nothing still leaves the tarball
# here and still checks it. Two things depend on that:
#
#   - the release workflow uploads this file beside the DMG, which is
#     how the LGPL's source offer is met. A cached `target/` that
#     carried the binary and not the archive would otherwise take the
#     run all the way through the compile and Apple's notarization
#     queue before failing at the last step, with the tag spent.
#   - FFMPEG-NOTICE.md, inside the app, states this digest as the
#     source the binary was built from. A check that a warm build skips
#     is not holding that sentence up.
#
# The cost is one 11 MB download on a machine that has the binary and
# not the archive, and a hash of it on every run.
if [[ ! -f "$tarball" ]]; then
    echo "downloading ffmpeg $FFMPEG_VERSION source..."
    curl -fSL --retry 3 -o "$tarball" "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz"
fi

# A tarball already sitting in target/ is exactly as unchecked as one
# that just arrived, so this runs for both.
actual="$(shasum -a 256 "$tarball" | cut -d' ' -f1)"
if [[ "$actual" != "$FFMPEG_SHA256" ]]; then
    echo "ffmpeg $FFMPEG_VERSION tarball is not the pinned one:" >&2
    echo "  expected $FFMPEG_SHA256" >&2
    echo "  got      $actual" >&2
    echo "  at       $tarball" >&2
    echo "Delete it to re-download, or — if the pin is what is stale —" >&2
    echo "verify the new tarball's signature and update FFMPEG_SHA256." >&2
    exit 1
fi

# The stamp records both halves of what the output was built from. It
# used to record the version alone, which left the case a pin exists
# for unhandled: correcting FFMPEG_SHA256 without moving the version —
# because the first digest was wrong, or upstream re-rolled the
# archive — matched a warm `target/`, skipped the build, and kept a
# binary compiled from the bytes that were just rejected.
built="$FFMPEG_VERSION $FFMPEG_SHA256"
if [[ -x "$out" && -f "$stamp" && "$(cat "$stamp")" == "$built" && "${FFMPEG_SIDECAR_FORCE:-0}" != "1" ]]; then
    echo "ffmpeg sidecar $FFMPEG_VERSION already built: $out"
    exit 0
fi

rm -rf "$src_dir"
tar -xf "$tarball" -C "$out_dir"

cd "$src_dir"

# LGPL discipline: no --enable-gpl, no --enable-nonfree, no
# --enable-version3 — absence is what keeps the build LGPL v2.1
# (ffmpeg.org/legal.html). Everything below is component selection.
#
# --disable-everything then explicit enables: decoders do not pull
# their parsers/demuxers in automatically, so each layer is listed.
# Decoder list = what webm/mkv/avi containers commonly carry; the
# encode side is exactly the three encoders the jobs use
# (thumb_ffmpeg.rs → mjpeg, preview_ffmpeg.rs → h264_videotoolbox +
# aac; libx264 is intentionally absent — preview_ffmpeg tries it
# first and falls through to videotoolbox).
#
# This list is restated in FFMPEG-NOTICE.md, which ships inside the app
# as the statement of what a user can rebuild from the source offered
# beside the download. A flag added or removed here goes there in the
# same commit, or the notice describes a build nobody made.
./configure \
    --prefix="$src_dir/dist" \
    --enable-static --disable-shared \
    --enable-pthreads \
    --enable-videotoolbox \
    --disable-doc --disable-debug \
    --disable-network \
    --disable-avdevice --disable-indevs --disable-outdevs \
    --disable-sdl2 --disable-xlib \
    --disable-ffplay --disable-ffprobe \
    --disable-everything \
    --enable-decoder=h264,hevc,vp8,vp9,av1,mpeg4,msmpeg4v1,msmpeg4v2,msmpeg4v3,mjpeg,theora,mpeg1video,mpeg2video,flv,wmv1,wmv2,rawvideo,png,aac,mp3,ac3,eac3,opus,vorbis,flac,pcm_s16le,pcm_s16be,pcm_u8,pcm_f32le,pcm_alaw,pcm_mulaw \
    --enable-parser=h264,hevc,vp8,vp9,av1,mpeg4video,mjpeg,png,aac,ac3,mpegaudio,opus,vorbis,flac \
    --enable-demuxer=matroska,avi,mov \
    --enable-encoder=h264_videotoolbox,aac,mjpeg \
    --enable-muxer=mp4,image2pipe \
    --enable-bsf=aac_adtstoasc,extract_extradata,h264_mp4toannexb \
    --enable-filter=scale,format,aresample,aformat,null,anull \
    --enable-protocol=file,pipe

make -j"$(sysctl -n hw.ncpu)"

# Fail here, not at first user click: assert the capabilities the jobs
# depend on actually made it into the binary.
./ffmpeg -hide_banner -decoders | grep -Eq '^ V.* vp9 ' || { echo "missing native vp9 decoder" >&2; exit 1; }
./ffmpeg -hide_banner -decoders | grep -Eq '^ V.* vp8 ' || { echo "missing native vp8 decoder" >&2; exit 1; }
./ffmpeg -hide_banner -encoders | grep -q  'h264_videotoolbox' || { echo "missing h264_videotoolbox encoder" >&2; exit 1; }
./ffmpeg -hide_banner -encoders | grep -Eq '^ A.* aac ' || { echo "missing native aac encoder" >&2; exit 1; }
./ffmpeg -hide_banner -encoders | grep -Eq '^ V.* mjpeg ' || { echo "missing mjpeg encoder (thumbnails)" >&2; exit 1; }
./ffmpeg -hide_banner -muxers   | grep -Eq ' mp4 ' || { echo "missing mp4 muxer" >&2; exit 1; }
# GPL tripwire: the license line of -version must say LGPL, and no
# third-party dylib may appear in the link table.
#
# System paths only, and the reason is not the exception this comment
# used to name. §6's system-library exception belongs to the combined
# work case — a program of your own linked against the library and
# distributed under terms of your choice. What ships here is FFmpeg's
# own tool linked against FFmpeg's own libraries, wholly LGPL, which
# §4 governs; under §4 the frameworks the OS ships are not part of
# FFmpeg's corresponding source at all, so no exception has to be
# invoked to leave them out. A *third-party* dylib would be a different
# matter, which is what this check is for: it would be a library the
# binary needs, that this build did not build and does not offer, and
# it would also break on a machine that does not happen to have it.
./ffmpeg -version | grep -q 'the FFmpeg developers' || { echo "unexpected -version output" >&2; exit 1; }
if ./ffmpeg -version | head -3 | grep -qi 'gpl'; then
    if ! ./ffmpeg -version | head -3 | grep -qi 'lgpl'; then
        echo "build reports GPL, not LGPL — refusing to install" >&2
        exit 1
    fi
fi
bad_links="$(otool -L ffmpeg | tail -n +2 | awk '{print $1}' | grep -Ev '^(/usr/lib/|/System/Library/)' || true)"
if [[ -n "$bad_links" ]]; then
    echo "non-system link dependencies found (would break on a clean machine):" >&2
    echo "$bad_links" >&2
    exit 1
fi

cp ffmpeg "$out"
echo "$built" > "$stamp"
echo "built: $out ($(du -h "$out" | cut -f1 | tr -d ' '))"
"$out" -version | head -2
