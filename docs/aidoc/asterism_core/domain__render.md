# asterism-core::domain::render

How an asset is rendered — thumbnail eligibility, media path, and
preview mode, decided in one place.

## Why this exists

"Can this be shown as a tile?" is a question about the **bytes**, and
the bytes answer it: `image/png` is an image whatever the asset is
*about*. Before this module the question was routed through the
semantic axis instead — `asset.modality` → the Modality master's
`kind` → `ContentKind::capabilities()` — with a second, mime-based
path bolted on for rows carrying no modality (asset-model v4 left
conversation rows unclassified).

Two paths answering one question is not merely redundant; they
disagreed. Classifying a PNG as `memory` (a `text` kind) made it
stop being thumbnailable, so the same file rendered differently
depending on what it was filed under [measured 2026-07-29, dogfood:
25 unclassified PNGs all had thumbnails, the 5 classified as
`memory` / `work_product` had none].

## The split

Material (mime) answers what the bytes *are* — thumbnail, media
path, "is this text". Modality answers only what mime cannot: a
terminal transcript is `text/plain` like any other note, and no
amount of byte inspection reveals that it should render as a
terminal. That is a genuine semantic input, and it is the *only*
one this policy takes.

## Functions

- `needs_video_preview` — Whether the detail player needs a transcoded preview rendition for
- `render_policy` — Decides the render policy from the physical fact first.
- `video_preview_failed_path` — The failure marker beside the rendition — written when a transcode
- `video_preview_part_path` — The staging file while a transcode runs. Stale ones (a crash) are
- `video_preview_path` — The rendition file for an asset under the previews directory.

## Types

- `RenderPolicy` — Everything the jobs and the UI need to know about painting an asset.

