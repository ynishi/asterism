//! Procedural PNG synthesis for the T-file tier.
//!
//! What a bench corpus has to reproduce about a real library is two costs,
//! and they are driven by different numbers:
//!
//! - **decode cost** scales with pixel count → carried by the dimension mix
//!   in [`crate::model`], not by anything in this file;
//! - **IO cost / cache footprint** scales with encoded bytes → carried here.
//!
//! So the shape of an image matters only insofar as it lands the encoded
//! size in the reference band (0.8–2.0 MB, median ~1.3 MB). A flat
//! gradient compresses to almost nothing and a fully random canvas
//! compresses to nothing at all; the control valve is **how much of the
//! canvas is covered by per-pixel noise**.
//!
//! ## Coverage is solved for, not fixed
//!
//! The spec's nominal figure is 30–60 % coverage, which lands the band for a
//! ~1 megapixel canvas. Applying that same fraction to a 2048×2048 canvas
//! would produce ~5 MB files — outside the band, and outside it in a way
//! that would make the corpus non-comparable with the reference workload. So
//! the byte target is drawn first ([`TARGET_BYTES`]) and coverage is solved
//! for from the pixel count. At the 1 Mpx reference this reproduces the
//! nominal 30–60 %; larger canvases get proportionally less coverage and
//! keep the band. Dimension still drives decode cost, which is the reason
//! the mix exists in the first place.
//!
//! [`BYTES_PER_NOISE_PX`] is the calibration constant tying the two: deflate
//! on high-amplitude noise stays near the raw 3 bytes/px, minus what
//! clamping at 0/255 and the opaque primitives give back.
//!
//! ## Determinism
//!
//! `render_png(spec)` is a pure function of `spec.image_seed` plus the
//! canvas size: one `ChaCha20Rng`, drawn in a fixed order, and `image`'s
//! default PNG encoder. Same spec ⇒ same bytes ⇒ same `content_hash`, which
//! is also what keeps every asset distinct instead of collapsing into the
//! importer's dedup path.

use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{ColorType, ImageFormat, RgbImage};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use std::io::Cursor;

use crate::model::AssetSpec;

/// Encoded-size target drawn per asset, in bytes. Slightly inside the
/// 0.8–2.0 MB band so calibration error does not push runs outside it.
const TARGET_BYTES: (u64, u64) = (900_000, 1_800_000);
/// Empirical encoded cost of a noise-covered pixel. Raw RGB8 is 3.0; high
/// amplitude noise deflates to near that, and the opaque primitives painted
/// afterwards erase a little of it back.
const BYTES_PER_NOISE_PX: f64 = 2.7;
const COVERAGE_CLAMP: (f64, f64) = (0.04, 0.70);
const NOISE_RECTS: (u32, u32) = (2, 5);
/// Noise amplitude stays high: a low-amplitude region compresses well and
/// would make coverage a much weaker control valve.
const NOISE_AMPLITUDE: (i32, i32) = (160, 255);
const GRADIENT_STOPS: (usize, usize) = (2, 4);
const PRIMITIVES: (u32, u32) = (3, 8);

/// Quality of the seeded 256 px thumbnails, matching the value the real
/// `thumb_gen` job encodes at (`asterism-infra` `THUMB_JPEG_QUALITY`) —
/// a seeded cache entry should cost the grid what a generated one does.
const THUMB_JPEG_QUALITY: u8 = 82;
/// Noise coverage used when rendering a thumbnail directly.
///
/// A real thumbnail is a *downscale*, and downscaling averages per-pixel
/// noise away — the 256 px JPEG of a noisy 1024 px original is far
/// smoother than a 256 px canvas painted with the same coverage. The
/// solved-for coverage of the full-size path ([`apply_noise`]) would hit
/// its 0.70 clamp at this pixel count and produce a ~3× oversized cache
/// entry, so the thumbnail path fixes coverage at a value that lands the
/// design's ~15–25 KB band instead.
const THUMB_NOISE_COVERAGE: f64 = 0.22;

/// Render `spec` to PNG bytes. Deterministic in `spec.image_seed`.
pub fn render_png(spec: &AssetSpec) -> Result<Vec<u8>> {
    let (w, h) = (spec.width, spec.height);
    anyhow::ensure!(w > 0 && h > 0, "asset {} has an empty canvas", spec.index);

    let img = render_canvas(spec, w, h, None)?;
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .with_context(|| format!("PNG encode failed for asset {}", spec.index))?;
    Ok(out)
}

/// Render the grid thumbnail of `spec` as JPEG bytes: the same canvas,
/// fitted into a `size_px` box, at the quality the thumbnail job uses.
///
/// This is what the T-meta tier writes into `thumb_cache` so a 110,000
/// row grid paints real tiles instead of placeholders. It is *not* a
/// downscale of [`render_png`] — the metadata tier writes no files, so
/// there is no original to scale — but it is derived from the same
/// `image_seed`, so the tile and the (never materialised) original are
/// the same picture at two sizes.
pub fn render_thumb_jpeg(spec: &AssetSpec, size_px: u32) -> Result<Vec<u8>> {
    anyhow::ensure!(size_px > 0, "thumbnail size must be positive");
    let (w, h) = fit_box(spec.width, spec.height, size_px);
    let img = render_canvas(spec, w, h, Some(THUMB_NOISE_COVERAGE))?;

    let mut out = Vec::with_capacity(32 * 1024);
    JpegEncoder::new_with_quality(&mut out, THUMB_JPEG_QUALITY)
        .encode(img.as_raw(), w, h, ColorType::Rgb8.into())
        .with_context(|| format!("thumbnail encode failed for asset {}", spec.index))?;
    Ok(out)
}

/// Longest-side-fits-`size_px` scaling, aspect preserved, never below
/// one pixel — the same rule the thumbnail job applies to an original.
fn fit_box(width: u32, height: u32, size_px: u32) -> (u32, u32) {
    let longest = width.max(height).max(1);
    if longest <= size_px {
        return (width.max(1), height.max(1));
    }
    let scale = size_px as f64 / longest as f64;
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

/// The three paint layers, in the fixed order that makes the output a
/// pure function of `spec.image_seed` and the canvas size.
fn render_canvas(
    spec: &AssetSpec,
    w: u32,
    h: u32,
    coverage_override: Option<f64>,
) -> Result<RgbImage> {
    anyhow::ensure!(w > 0 && h > 0, "asset {} has an empty canvas", spec.index);
    let mut rng = ChaCha20Rng::seed_from_u64(spec.image_seed);
    let mut buf = vec![0u8; w as usize * h as usize * 3];

    fill_gradient(&mut buf, w, h, &mut rng);
    apply_noise(&mut buf, w, h, &mut rng, coverage_override);
    draw_primitives(&mut buf, w, h, &mut rng);

    RgbImage::from_raw(w, h, buf).context("canvas buffer does not match the declared dimensions")
}

/// Linear gradient over 2–4 stops at a random angle. This is the base layer:
/// it costs almost nothing to encode, so it sets the look without disturbing
/// the size budget.
fn fill_gradient(buf: &mut [u8], w: u32, h: u32, rng: &mut ChaCha20Rng) {
    let stop_count = rng.random_range(GRADIENT_STOPS.0..=GRADIENT_STOPS.1);
    let stops: Vec<[u8; 3]> = (0..stop_count)
        .map(|_| [rng.random::<u8>(), rng.random::<u8>(), rng.random::<u8>()])
        .collect();

    let angle = rng.random::<f64>() * std::f64::consts::TAU;
    let (dx, dy) = (angle.cos(), angle.sin());
    // Projection range over the four corners, so `t` spans 0..1 exactly.
    let corners = [
        (0.0, 0.0),
        ((w - 1) as f64, 0.0),
        (0.0, (h - 1) as f64),
        ((w - 1) as f64, (h - 1) as f64),
    ];
    let projections: Vec<f64> = corners.iter().map(|(x, y)| x * dx + y * dy).collect();
    let lo = projections.iter().copied().fold(f64::MAX, f64::min);
    let hi = projections.iter().copied().fold(f64::MIN, f64::max);
    let span = (hi - lo).max(1.0);

    let segments = (stops.len() - 1) as f64;
    for y in 0..h {
        for x in 0..w {
            let t = ((x as f64 * dx + y as f64 * dy) - lo) / span;
            let pos = (t * segments).clamp(0.0, segments);
            let seg = (pos.floor() as usize).min(stops.len() - 2);
            let f = pos - seg as f64;
            let idx = ((y as usize * w as usize) + x as usize) * 3;
            let (from, to) = (stops[seg], stops[seg + 1]);
            for (out, (a, b)) in buf[idx..idx + 3].iter_mut().zip(from.iter().zip(to.iter())) {
                let (a, b) = (*a as f64, *b as f64);
                *out = (a + (b - a) * f).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Per-pixel noise over a few rectangles whose total area is solved for the
/// encoded-size target. This is the only layer that meaningfully costs
/// bytes.
///
/// `coverage_override` is how the thumbnail path opts out of solving for
/// an encoded-size target it does not have; the draw is taken either way
/// so both paths consume the stream identically.
fn apply_noise(
    buf: &mut [u8],
    w: u32,
    h: u32,
    rng: &mut ChaCha20Rng,
    coverage_override: Option<f64>,
) {
    let px = w as f64 * h as f64;
    let target = rng.random_range(TARGET_BYTES.0..=TARGET_BYTES.1) as f64;
    let solved = (target / (px * BYTES_PER_NOISE_PX)).clamp(COVERAGE_CLAMP.0, COVERAGE_CLAMP.1);
    let coverage = coverage_override.unwrap_or(solved);
    let rects = rng.random_range(NOISE_RECTS.0..=NOISE_RECTS.1);
    let per_rect_area = coverage * px / rects as f64;

    for _ in 0..rects {
        // Random aspect, then clamp into the canvas. Rectangles may overlap;
        // that only costs a little coverage, which the band absorbs.
        let aspect = 0.5 + rng.random::<f64>() * 1.5;
        let rw = (per_rect_area * aspect).sqrt().round().max(1.0) as u32;
        let rh = (per_rect_area / rw as f64).round().max(1.0) as u32;
        let rw = rw.min(w);
        let rh = rh.min(h);
        let x0 = rng.random_range(0..=(w - rw));
        let y0 = rng.random_range(0..=(h - rh));
        let amp = rng.random_range(NOISE_AMPLITUDE.0..=NOISE_AMPLITUDE.1);

        for y in y0..y0 + rh {
            for x in x0..x0 + rw {
                let idx = ((y as usize * w as usize) + x as usize) * 3;
                for out in buf[idx..idx + 3].iter_mut() {
                    let n = rng.random::<u8>() as i32;
                    let delta = ((n - 128) * amp) / 128;
                    *out = (*out as i32 + delta).clamp(0, 255) as u8;
                }
            }
        }
    }
}

/// Opaque solid rectangles and circles. Kept small (a few percent of the
/// canvas each) so they read as structure without erasing enough noise to
/// move the size band.
fn draw_primitives(buf: &mut [u8], w: u32, h: u32, rng: &mut ChaCha20Rng) {
    let count = rng.random_range(PRIMITIVES.0..=PRIMITIVES.1);
    for _ in 0..count {
        let color = [rng.random::<u8>(), rng.random::<u8>(), rng.random::<u8>()];
        if rng.random_bool(0.5) {
            let rw = rng.random_range(w / 20..=w / 7).max(1);
            let rh = rng.random_range(h / 20..=h / 7).max(1);
            let x0 = rng.random_range(0..=(w - rw));
            let y0 = rng.random_range(0..=(h - rh));
            for y in y0..y0 + rh {
                for x in x0..x0 + rw {
                    put(buf, w, x, y, color);
                }
            }
        } else {
            let min_dim = w.min(h);
            let r = rng.random_range(min_dim / 32..=min_dim / 10).max(1);
            let cx = rng.random_range(r..w.saturating_sub(r).max(r + 1));
            let cy = rng.random_range(r..h.saturating_sub(r).max(r + 1));
            let r2 = (r * r) as i64;
            for y in cy.saturating_sub(r)..(cy + r).min(h) {
                for x in cx.saturating_sub(r)..(cx + r).min(w) {
                    let dx = x as i64 - cx as i64;
                    let dy = y as i64 - cy as i64;
                    if dx * dx + dy * dy <= r2 {
                        put(buf, w, x, y, color);
                    }
                }
            }
        }
    }
}

#[inline]
fn put(buf: &mut [u8], w: u32, x: u32, y: u32, color: [u8; 3]) {
    let idx = ((y as usize * w as usize) + x as usize) * 3;
    buf[idx..idx + 3].copy_from_slice(&color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SpecStream;

    /// A spec with the canvas shrunk: the pixel work is what makes these
    /// tests slow, and none of the properties under test depend on size.
    fn small_spec(index: u64) -> AssetSpec {
        let mut spec = SpecStream::new(42).nth(index as usize).expect("infinite");
        spec.width = 96;
        spec.height = 72;
        spec
    }

    #[test]
    fn same_spec_renders_identical_bytes() {
        let spec = small_spec(3);
        let a = render_png(&spec).expect("render");
        let b = render_png(&spec).expect("render");
        assert_eq!(a, b);
        assert!(a.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
    }

    #[test]
    fn different_assets_render_different_bytes() {
        let a = render_png(&small_spec(1)).expect("render");
        let b = render_png(&small_spec(2)).expect("render");
        assert_ne!(
            a, b,
            "distinct assets must not collapse into one content hash"
        );
    }

    #[test]
    fn thumbnails_are_deterministic_and_fit_the_box() {
        let spec = SpecStream::new(42).nth(5).expect("infinite");
        let a = render_thumb_jpeg(&spec, 256).expect("thumb");
        let b = render_thumb_jpeg(&spec, 256).expect("thumb");
        assert_eq!(a, b, "a seeded cache entry must be reproducible");
        assert!(a.starts_with(&[0xFF, 0xD8]), "not a JPEG");

        // Aspect preserved, longest side exactly the box.
        assert_eq!(fit_box(1024, 1024, 256), (256, 256));
        assert_eq!(fit_box(832, 1216, 256), (175, 256));
        assert_eq!(fit_box(2048, 2048, 256), (256, 256));
        // Already smaller than the box: left alone rather than upscaled.
        assert_eq!(fit_box(120, 90, 256), (120, 90));
    }

    #[test]
    fn thumbnail_size_lands_in_the_cache_budget() {
        // 110,000 × this is the L preset's `thumb_cache` footprint, which
        // the design budgets at ~2.5 GB (~15–25 KB per entry). The band
        // here is wider so a calibration drift shows up as a number in
        // the seed report rather than as a flaky test, but it still
        // fails long before the footprint doubles.
        let mut sizes: Vec<usize> = SpecStream::new(42)
            .take(24)
            .map(|s| render_thumb_jpeg(&s, 256).expect("thumb").len())
            .collect();
        sizes.sort_unstable();
        let median = sizes[sizes.len() / 2];
        assert!(
            (8_000..=45_000).contains(&median),
            "thumbnail median out of the cache budget: {median} bytes (all: {sizes:?})"
        );
    }

    #[test]
    fn encoded_size_lands_in_the_bench_band() {
        // Eight full-size 1024×1024 renders: enough for a median, cheap
        // enough to stay in the normal test run. The band is deliberately
        // wider than the design's 0.8–2.0 MB so calibration drift reports as
        // a size shift in the manifest rather than as a flaky test.
        let mut sizes: Vec<usize> = SpecStream::new(42)
            .take(64)
            .filter(|s| s.width == 1024 && s.height == 1024)
            .take(8)
            .map(|s| render_png(&s).expect("render").len())
            .collect();
        assert_eq!(sizes.len(), 8, "fixture did not yield 8 square specs");
        sizes.sort_unstable();
        let median = (sizes[3] + sizes[4]) / 2;
        assert!(
            (600_000..=2_600_000).contains(&median),
            "median encoded size out of band: {median} bytes (all: {sizes:?})"
        );
    }
}
