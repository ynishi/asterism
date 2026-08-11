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
# support statically linked binaries (QA1118), and the LGPL system
# library exception covers frameworks the OS ships.
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

root="$(cd "$(dirname "$0")/.." && pwd)"
out_dir="$root/target/ffmpeg-sidecar"
triple="$(rustc -vV | sed -n 's/^host: //p')"
out="$out_dir/ffmpeg-$triple"
stamp="$out_dir/.ffmpeg-version"

if [[ -x "$out" && -f "$stamp" && "$(cat "$stamp")" == "$FFMPEG_VERSION" && "${FFMPEG_SIDECAR_FORCE:-0}" != "1" ]]; then
    echo "ffmpeg sidecar $FFMPEG_VERSION already built: $out"
    exit 0
fi

mkdir -p "$out_dir"
tarball="$out_dir/ffmpeg-$FFMPEG_VERSION.tar.xz"
src_dir="$out_dir/ffmpeg-$FFMPEG_VERSION"

if [[ ! -f "$tarball" ]]; then
    echo "downloading ffmpeg $FFMPEG_VERSION source..."
    curl -fSL --retry 3 -o "$tarball" "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz"
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
# third-party dylib may appear in the link table (system paths only —
# that is the LGPL §6 system library exception boundary).
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
echo "$FFMPEG_VERSION" > "$stamp"
echo "built: $out ($(du -h "$out" | cut -f1 | tr -d ' '))"
"$out" -version | head -2
