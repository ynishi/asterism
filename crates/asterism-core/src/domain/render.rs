//! How an asset is rendered — thumbnail eligibility, media path, and
//! preview mode, decided in one place.
//!
//! ## Why this exists
//!
//! "Can this be shown as a tile?" is a question about the **bytes**, and
//! the bytes answer it: `image/png` is an image whatever the asset is
//! *about*. Before this module the question was routed through the
//! semantic axis instead — `asset.modality` → the Modality master's
//! `kind` → `ContentKind::capabilities()` — with a second, mime-based
//! path bolted on for rows carrying no modality (asset-model v4 left
//! conversation rows unclassified).
//!
//! Two paths answering one question is not merely redundant; they
//! disagreed. Classifying a PNG as `memory` (a `text` kind) made it
//! stop being thumbnailable, so the same file rendered differently
//! depending on what it was filed under [measured 2026-07-29, dogfood:
//! 25 unclassified PNGs all had thumbnails, the 5 classified as
//! `memory` / `work_product` had none].
//!
//! ## The split
//!
//! Material (mime) answers what the bytes *are* — thumbnail, media
//! path, "is this text". Modality answers only what mime cannot: a
//! terminal transcript is `text/plain` like any other note, and no
//! amount of byte inspection reveals that it should render as a
//! terminal. That is a genuine semantic input, and it is the *only*
//! one this policy takes.

use crate::domain::value::{AssetRole, MediaKind, MimeType, PreviewMode};

/// Everything the jobs and the UI need to know about painting an asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderPolicy {
    /// Whether a thumbnail can be generated (and is worth caching).
    pub thumbnail: bool,
    /// Which inline media player, if any, the detail view uses.
    pub media: MediaKind,
    /// How the preview overlay reads the body.
    pub preview: PreviewMode,
}

/// Decides the render policy from the physical fact first.
///
/// `mime` is the primary material's format (`None` = unknown, which is
/// not the same as "not applicable" — an unhashed or not-yet-probed
/// original lands here and is treated as text-like, the conservative
/// choice since it never promises a player for bytes we cannot read).
///
/// `terminal` is the single semantic input: the asset's modality is a
/// terminal transcript (the Modality master's `kind` is `Term`). It
/// only matters for text — a PNG filed under a terminal-ish modality
/// is still a PNG.
pub fn render_policy(mime: Option<&MimeType>, role: AssetRole, terminal: bool) -> RenderPolicy {
    // A container owns no bytes; its card summarises its members and
    // its "preview" is the member reader, which the UI keys off
    // `container_id` rather than a preview mode.
    if role == AssetRole::Collection {
        return RenderPolicy {
            thumbnail: false,
            media: MediaKind::None,
            preview: PreviewMode::None,
        };
    }
    match mime {
        // Anything with a player renders as media, and the preview
        // overlay is not a text reader for it. Whether a raster can be
        // cached is the format's own answer — video thumbnails are a
        // frame grab, so a clip is as tileable as a still, while audio
        // draws a waveform from decoded samples instead.
        Some(m) if m.media() != MediaKind::None => RenderPolicy {
            thumbnail: m.thumbnailable(),
            media: m.media(),
            preview: PreviewMode::None,
        },
        // Text, a family this codebase does not act on, or no mime at
        // all. `None` is treated as text-like: the conservative choice,
        // since it never promises a player for bytes we cannot read.
        _ => RenderPolicy {
            thumbnail: false,
            media: MediaKind::None,
            preview: if terminal {
                PreviewMode::Term
            } else {
                PreviewMode::TextSniff
            },
        },
    }
}

/// Whether the detail player needs a transcoded preview rendition for
/// this video, rather than playing the original directly.
///
/// Exactly the formats the embedded webview cannot display [measured
/// 2026-07-31, packaged WKWebView 605.1.15]: WebM because its default
/// codec (VP9) never decodes in the DOM — via `src` it stalls
/// silently, via MSE it throws `MEDIA_ERR_DECODE`, while the same
/// bytes decode on a *detached* element, so `canPlayType`'s
/// "probably" cannot be trusted — and Matroska because the container
/// is rejected outright. VP8 WebM would play natively, but the mime
/// cannot tell VP8 from VP9, so all WebM takes the rendition path
/// (a spurious transcode is cheap; a spurious crossed-out player is
/// the defect this exists to remove).
///
/// AVI is here too, for a subtler failure: the clock advances but the
/// frames render black (measured in the Dogfood pane, 2026-07-31) —
/// a player that pretends to work is the same defect as one that
/// refuses. `video/mp4` and `video/quicktime` play as-is — measured,
/// not assumed.
/// The set itself lives on [`VideoFormat`] so that this question and
/// "does the frame grab need external ffmpeg?" (`thumb_ffmpeg`) read
/// the same three variants. They used to be two copies of the same
/// literals in two crates, which drift the moment one gains a format.
pub fn needs_video_preview(mime: Option<&MimeType>) -> bool {
    mime.is_some_and(MimeType::needs_video_preview)
}

/// The rendition file for an asset under the previews directory.
///
/// The three-path family below is the whole on-disk contract between
/// the transcoder (infra) and the status endpoint (this crate): a
/// `.mp4` that exists is a complete rendition (the transcoder stages
/// into `.part` and renames), a `.part` means a transcode is running,
/// a `.failed` carries the reason the last attempt died. One place
/// owns the naming so the two sides cannot drift.
pub fn video_preview_path(previews_dir: &std::path::Path, asset_id: &str) -> std::path::PathBuf {
    previews_dir.join(format!("{asset_id}.mp4"))
}

/// The failure marker beside the rendition — written when a transcode
/// fails, so the pane can say "failed: why" instead of spinning.
pub fn video_preview_failed_path(
    previews_dir: &std::path::Path,
    asset_id: &str,
) -> std::path::PathBuf {
    previews_dir.join(format!("{asset_id}.failed"))
}

/// The staging file while a transcode runs. Stale ones (a crash) are
/// swept at startup.
pub fn video_preview_part_path(
    previews_dir: &std::path::Path,
    asset_id: &str,
) -> std::path::PathBuf {
    previews_dir.join(format!("{asset_id}.mp4.part"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses a literal for the cases below. The policy takes a parsed
    /// format now, so a test cannot smuggle in a spelling the boundary
    /// would have normalised away.
    fn mime(raw: &str) -> MimeType {
        MimeType::parse(raw)
    }

    /// The regression this module exists for: a PNG stays a PNG no
    /// matter what it is filed under. Routing the question through the
    /// semantic axis made `memory`-filed images unthumbnailable while
    /// identical unclassified ones were fine.
    #[test]
    fn classification_does_not_change_what_the_bytes_are() {
        let unclassified = render_policy(Some(&mime("image/png")), AssetRole::Item, false);
        let filed_as_a_terminal_note =
            render_policy(Some(&mime("image/png")), AssetRole::Item, true);
        assert_eq!(unclassified, filed_as_a_terminal_note);
        assert!(unclassified.thumbnail);
        assert_eq!(unclassified.media, MediaKind::Image);
    }

    #[test]
    fn video_is_tileable_and_audio_is_not() {
        let video = render_policy(Some(&mime("video/mp4")), AssetRole::Item, false);
        assert!(video.thumbnail, "a frame grab is a thumbnail");
        assert_eq!(video.media, MediaKind::Video);

        let audio = render_policy(Some(&mime("audio/wav")), AssetRole::Item, false);
        assert!(!audio.thumbnail, "a waveform is drawn, not cached");
        assert_eq!(audio.media, MediaKind::Audio);
    }

    /// A subtype in no list this codebase keeps still renders by its
    /// family. Under the string form this worked by `starts_with`; the
    /// parse must not quietly demote it to "unknown".
    #[test]
    fn an_unnamed_image_subtype_still_tiles() {
        let icon = render_policy(Some(&mime("image/x-icon")), AssetRole::Item, false);
        assert!(icon.thumbnail);
        assert_eq!(icon.media, MediaKind::Image);
        assert_eq!(icon.preview, PreviewMode::None);
    }

    /// The one thing mime cannot answer: a transcript is `text/plain`
    /// like every other note, so the terminal reading has to come from
    /// the semantic axis.
    #[test]
    fn terminal_reading_comes_from_the_semantic_axis() {
        assert_eq!(
            render_policy(Some(&mime("text/plain")), AssetRole::Item, true).preview,
            PreviewMode::Term
        );
        assert_eq!(
            render_policy(Some(&mime("text/plain")), AssetRole::Item, false).preview,
            PreviewMode::TextSniff
        );
    }

    /// Unknown mime is "we have not read the bytes", not "there are
    /// none" — treat it as text rather than promising a player.
    #[test]
    fn unknown_mime_reads_as_text() {
        let policy = render_policy(None, AssetRole::Item, false);
        assert!(!policy.thumbnail);
        assert_eq!(policy.media, MediaKind::None);
        assert_eq!(policy.preview, PreviewMode::TextSniff);
    }

    #[test]
    fn only_the_measured_unplayable_formats_need_a_preview_rendition() {
        assert!(needs_video_preview(Some(&mime("video/webm"))));
        assert!(needs_video_preview(Some(&mime("video/x-matroska"))));
        // AVI's clock runs but its frames render black — broken in a
        // quieter way, same rendition treatment.
        assert!(needs_video_preview(Some(&mime("video/x-msvideo"))));
        // Measured playable in the packaged webview — transcoding
        // these would burn CPU to replace a working player.
        assert!(!needs_video_preview(Some(&mime("video/mp4"))));
        assert!(!needs_video_preview(Some(&mime("video/quicktime"))));
        assert!(!needs_video_preview(Some(&mime("image/png"))));
        assert!(!needs_video_preview(None));
    }

    #[test]
    fn a_container_paints_nothing_of_its_own() {
        // Even with a mime accidentally attached, the role wins: a
        // container's content is its members.
        let policy = render_policy(Some(&mime("image/png")), AssetRole::Collection, false);
        assert!(!policy.thumbnail);
        assert_eq!(policy.media, MediaKind::None);
        assert_eq!(policy.preview, PreviewMode::None);
    }
}
