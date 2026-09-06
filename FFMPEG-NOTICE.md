# FFmpeg

Asterism ships an `ffmpeg` executable beside the application and runs it as a
separate process. This notice is what the LGPL asks to travel with that copy,
and it also answers a requirement that comes from libjpeg rather than from the
LGPL. Nothing here is legal advice.

## What is shipped, and under what terms

The binary is **FFmpeg 8.0**, built from unmodified upstream source, and it is
licensed under the **GNU Lesser General Public License, version 2.1 or later**.
The full text is beside this file as [LICENSE-LGPL-2.1](LICENSE-LGPL-2.1).

```
Copyright (c) 2000-2025 the FFmpeg developers

ffmpeg is free software; you can redistribute it and/or modify it under the
terms of the GNU Lesser General Public License as published by the Free
Software Foundation; either version 2.1 of the License, or (at your option)
any later version.

ffmpeg is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR
A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
details.
```

FFmpeg is not owned by, and not a product of, this project. Its home is
<https://ffmpeg.org>, and its authors are listed in the `MAINTAINERS` and
`CREDITS` files of the source below.

The build enables no GPL component and no non-free component, which is what
keeps it LGPL: FFmpeg's own `LICENSE.md` states that its GPL-covered parts are
never used unless `--enable-gpl` is passed, and this build passes neither that
nor `--enable-nonfree` nor `--enable-version3`. The binary reports the same for
itself under `ffmpeg -L`.

## The source

**No modifications were made.** The tree that was compiled is the upstream
release tarball as published, unpacked and configured, with no patch applied and
no file added, changed or removed.

- Upstream: <https://ffmpeg.org/releases/ffmpeg-8.0.tar.xz>
- SHA-256: `b2751fccb6cc4c77708113cd78b561059b6fa904b24162fa0be2d60273d27b8e`

That same tarball, and the script that configures and builds it, are published
alongside every release of this application, so the source can be obtained from
the same place the application was. The script is
`scripts/build-ffmpeg-sidecar.sh` in this project's repository,
<https://github.com/ynishi/asterism>.

The configuration it applies, which is what makes the build reproducible:

```
--enable-static --disable-shared
--enable-pthreads
--enable-videotoolbox
--disable-doc --disable-debug
--disable-network
--disable-avdevice --disable-indevs --disable-outdevs
--disable-sdl2 --disable-xlib
--disable-ffplay --disable-ffprobe
--disable-everything
--enable-decoder=h264,hevc,vp8,vp9,av1,mpeg4,msmpeg4v1,msmpeg4v2,msmpeg4v3,mjpeg,theora,mpeg1video,mpeg2video,flv,wmv1,wmv2,rawvideo,png,aac,mp3,ac3,eac3,opus,vorbis,flac,pcm_s16le,pcm_s16be,pcm_u8,pcm_f32le,pcm_alaw,pcm_mulaw
--enable-parser=h264,hevc,vp8,vp9,av1,mpeg4video,mjpeg,png,aac,ac3,mpegaudio,opus,vorbis,flac
--enable-demuxer=matroska,avi,mov
--enable-encoder=h264_videotoolbox,aac,mjpeg
--enable-muxer=mp4,image2pipe
--enable-bsf=aac_adtstoasc,extract_extradata,h264_mp4toannexb
--enable-filter=scale,format,aresample,aformat,null,anull
--enable-protocol=file,pipe
```

`--prefix` is omitted above because it names a staging directory on the machine
that built it and says nothing about what was built.

## The Independent JPEG Group

This requirement is FFmpeg's, stated in its own `LICENSE.md`, and it is not part
of the LGPL:

> The files `libavcodec/jfdctfst.c`, `libavcodec/jfdctint_template.c` and
> `libavcodec/jrevdct.c` are taken from libjpeg, see the top of the files for
> licensing details. Specifically note that you must credit the IJG in the
> documentation accompanying your program if you only distribute executables.
> You must also indicate any changes including additions and deletions to those
> three files in the documentation.

This software is based in part on the work of the **Independent JPEG Group**.

Those three files are compiled into the shipped binary — the configuration above
enables `mjpeg` and the MPEG-family decoders, which select them — and **no
change of any kind was made to them**: no addition, no deletion, no edit. They
are as they appear in the upstream tarball named above.
