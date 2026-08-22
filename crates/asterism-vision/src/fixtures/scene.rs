//! Deterministic scene rendering: the spec is the ground truth.
//!
//! An evaluation image must have describable content so that tag
//! suggestion and text pairing can be graded against something that was
//! never hand-annotated. So this module renders from an explicit
//! [`SceneSpec`] — a gradient background plus a handful of named shapes
//! in named colors — and the EN/JA tags and the paired caption are
//! derived from that spec, nothing else.
//!
//! ## Determinism
//!
//! Rendering takes no RNG: same spec ⇒ same bytes. All randomness lives
//! in [`crate::fixtures::relations`], keyed by a seed, so every fixture
//! is reproducible from `(seed)` alone.

use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ColorType, ImageFormat, RgbImage};
use std::io::Cursor;

/// The palette is deliberately small and nameable in both languages: a
/// tag vocabulary the encoder could plausibly match, not a color
/// histogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PaletteColor {
    Red,
    Blue,
    Green,
    Yellow,
    Purple,
    Orange,
}

/// Every palette color, in a stable order.
pub const PALETTE: [PaletteColor; 6] = [
    PaletteColor::Red,
    PaletteColor::Blue,
    PaletteColor::Green,
    PaletteColor::Yellow,
    PaletteColor::Purple,
    PaletteColor::Orange,
];

impl PaletteColor {
    /// The pigment painted for this color name.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            PaletteColor::Red => [214, 45, 32],
            PaletteColor::Blue => [0, 87, 183],
            PaletteColor::Green => [0, 135, 62],
            PaletteColor::Yellow => [255, 196, 0],
            PaletteColor::Purple => [110, 38, 145],
            PaletteColor::Orange => [239, 125, 0],
        }
    }

    /// English name.
    pub fn en(self) -> &'static str {
        match self {
            PaletteColor::Red => "red",
            PaletteColor::Blue => "blue",
            PaletteColor::Green => "green",
            PaletteColor::Yellow => "yellow",
            PaletteColor::Purple => "purple",
            PaletteColor::Orange => "orange",
        }
    }

    /// Adjectival Japanese form, so a tag reads as natural Japanese
    /// ("赤い円"), not as a literal translation stub.
    pub fn ja(self) -> &'static str {
        match self {
            PaletteColor::Red => "赤い",
            PaletteColor::Blue => "青い",
            PaletteColor::Green => "緑の",
            PaletteColor::Yellow => "黄色い",
            PaletteColor::Purple => "紫の",
            PaletteColor::Orange => "オレンジの",
        }
    }

    /// Noun form for captions ("赤と青のグラデーション").
    pub fn ja_noun(self) -> &'static str {
        match self {
            PaletteColor::Red => "赤",
            PaletteColor::Blue => "青",
            PaletteColor::Green => "緑",
            PaletteColor::Yellow => "黄色",
            PaletteColor::Purple => "紫",
            PaletteColor::Orange => "オレンジ",
        }
    }
}

/// The drawable shape kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ShapeKind {
    Circle,
    Square,
    Triangle,
}

/// Every shape kind, in a stable order.
pub const SHAPES: [ShapeKind; 3] = [ShapeKind::Circle, ShapeKind::Square, ShapeKind::Triangle];

impl ShapeKind {
    /// English name.
    pub fn en(self) -> &'static str {
        match self {
            ShapeKind::Circle => "circle",
            ShapeKind::Square => "square",
            ShapeKind::Triangle => "triangle",
        }
    }

    /// Japanese name.
    pub fn ja(self) -> &'static str {
        match self {
            ShapeKind::Circle => "円",
            ShapeKind::Square => "四角",
            ShapeKind::Triangle => "三角形",
        }
    }
}

/// One shape, in canvas-relative coordinates so a spec renders the same
/// picture at any canvas size (which is what makes the resize variant a
/// *transform* of the base rather than a different scene).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeSpec {
    /// What is drawn.
    pub kind: ShapeKind,
    /// What color it is drawn in.
    pub color: PaletteColor,
    /// Center, in `0.0..1.0` of the canvas.
    pub cx: f64,
    /// Center, in `0.0..1.0` of the canvas.
    pub cy: f64,
    /// Radius / half-side, in `0.0..1.0` of the shorter canvas side.
    pub size: f64,
}

/// The full ground truth of one image: background gradient and shapes.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSpec {
    /// Canvas width in pixels.
    pub width: u32,
    /// Canvas height in pixels.
    pub height: u32,
    /// The two gradient endpoint colors.
    pub bg: [PaletteColor; 2],
    /// Gradient direction in degrees.
    pub angle_deg: f64,
    /// The shapes, each fully visible (see [`crate::fixtures::relations`]).
    pub shapes: Vec<ShapeSpec>,
}

impl SceneSpec {
    /// `"<color> <shape>"` per shape, deduplicated, order-stable. This
    /// is the tag ground truth in both languages; a fixture never tags
    /// an image with anything the spec cannot show.
    pub fn tags_en(&self) -> Vec<String> {
        dedup_stable(
            self.shapes
                .iter()
                .map(|s| format!("{} {}", s.color.en(), s.kind.en())),
        )
    }

    /// Japanese tag ground truth; see [`Self::tags_en`].
    pub fn tags_ja(&self) -> Vec<String> {
        dedup_stable(
            self.shapes
                .iter()
                .map(|s| format!("{}{}", s.color.ja(), s.kind.ja())),
        )
    }

    /// One-sentence English caption for the text-pairing evaluation.
    pub fn caption_en(&self) -> String {
        let shapes = join_list_en(&self.tags_en());
        let gradient = format!(
            "{}-{} gradient background",
            self.bg[0].en(),
            self.bg[1].en()
        );
        format!(
            "{} on {} {}.",
            capitalize(&shapes),
            article(&gradient),
            gradient
        )
    }

    /// One-sentence Japanese caption for the text-pairing evaluation.
    pub fn caption_ja(&self) -> String {
        format!(
            "{}と{}のグラデーション背景に{}。",
            self.bg[0].ja_noun(),
            self.bg[1].ja_noun(),
            self.tags_ja().join("と")
        )
    }
}

fn dedup_stable(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = Vec::new();
    for item in items {
        if !seen.contains(&item) {
            seen.push(item);
        }
    }
    seen
}

fn join_list_en(items: &[String]) -> String {
    let with_article = |s: &String| format!("{} {s}", article(s));
    match items {
        [] => String::from("nothing"),
        [one] => with_article(one),
        [head @ .., last] => {
            let head = head.iter().map(with_article).collect::<Vec<_>>().join(", ");
            format!("{head} and {}", with_article(last))
        }
    }
}

/// "a" / "an" by leading vowel — enough for this fixed vocabulary.
fn article(word: &str) -> &'static str {
    match word.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Render a spec at its declared size. Pure function of the spec.
pub fn render(spec: &SceneSpec) -> Result<RgbImage> {
    let (w, h) = (spec.width, spec.height);
    anyhow::ensure!(w > 0 && h > 0, "empty canvas");
    let mut buf = vec![0u8; w as usize * h as usize * 3];

    fill_gradient(&mut buf, w, h, spec.bg, spec.angle_deg);
    for shape in &spec.shapes {
        draw_shape(&mut buf, w, h, shape);
    }

    RgbImage::from_raw(w, h, buf).context("canvas buffer does not match the declared dimensions")
}

fn fill_gradient(buf: &mut [u8], w: u32, h: u32, bg: [PaletteColor; 2], angle_deg: f64) {
    let (from, to) = (bg[0].rgb(), bg[1].rgb());
    let angle = angle_deg.to_radians();
    let (dx, dy) = (angle.cos(), angle.sin());
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

    for y in 0..h {
        for x in 0..w {
            let t = ((x as f64 * dx + y as f64 * dy) - lo) / span;
            let idx = ((y as usize * w as usize) + x as usize) * 3;
            for (out, (a, b)) in buf[idx..idx + 3].iter_mut().zip(from.iter().zip(to.iter())) {
                let (a, b) = (*a as f64, *b as f64);
                *out = (a + (b - a) * t).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// White rim around every shape, as a fraction of its size. A gradient
/// between two palette endpoints passes through in-between hues — red
/// to yellow crosses orange — so an unlucky shape color can melt into
/// the background and silently falsify the tag ground truth ("orange
/// square" that no eye or encoder can see). The rim guarantees every
/// asserted shape is visible on every background the stream can
/// produce; white is not a palette color, so it never collides with a
/// tag.
const OUTLINE_FRAC: f64 = 0.12;
const OUTLINE_RGB: [u8; 3] = [255, 255, 255];

fn draw_shape(buf: &mut [u8], w: u32, h: u32, shape: &ShapeSpec) {
    let color = shape.color.rgb();
    let min_dim = w.min(h) as f64;
    let cx = shape.cx * w as f64;
    let cy = shape.cy * h as f64;
    let r = (shape.size * min_dim).max(1.0);
    let inner = (r - (r * OUTLINE_FRAC).max(2.0)).max(0.0);

    let x0 = ((cx - r).floor().max(0.0)) as u32;
    let x1 = ((cx + r).ceil().min(w as f64 - 1.0)) as u32;
    let y0 = ((cy - r).floor().max(0.0)) as u32;
    let y1 = ((cy + r).ceil().min(h as f64 - 1.0)) as u32;

    let inside = |px: f64, py: f64, r: f64| -> bool {
        match shape.kind {
            ShapeKind::Circle => px * px + py * py <= r * r,
            ShapeKind::Square => px.abs() <= r && py.abs() <= r,
            // Upright isosceles triangle: apex at the top of the box,
            // base along the bottom.
            ShapeKind::Triangle => {
                r > 0.0 && py >= -r && py <= r && px.abs() <= (py + r) / (2.0 * r) * r
            }
        }
    };

    for y in y0..=y1 {
        for x in x0..=x1 {
            let px = x as f64 - cx;
            let py = y as f64 - cy;
            if inside(px, py, r) {
                let idx = ((y as usize * w as usize) + x as usize) * 3;
                let paint = if inside(px, py, inner) {
                    color
                } else {
                    OUTLINE_RGB
                };
                buf[idx..idx + 3].copy_from_slice(&paint);
            }
        }
    }
}

/// A canvas of seeded per-pixel noise: outside the scene family
/// entirely, with no gradient, no shapes and no tags. The
/// honest-failure control — retrieval must be allowed to answer it with
/// nothing.
pub fn noise_image(seed: u64, w: u32, h: u32) -> RgbImage {
    use rand::{Rng, SeedableRng};
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(seed);
    let buf: Vec<u8> = (0..w as usize * h as usize * 3)
        .map(|_| rng.random::<u8>())
        .collect();
    RgbImage::from_raw(w, h, buf).expect("buffer sized from the dimensions")
}

/// Encode to PNG with `image`'s default encoder.
pub fn to_png(img: &RgbImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .context("PNG encode failed")?;
    Ok(out)
}

/// Encode to JPEG at the given quality — the recompression variant.
pub fn to_jpeg(img: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode(
            img.as_raw(),
            img.width(),
            img.height(),
            ColorType::Rgb8.into(),
        )
        .context("JPEG encode failed")?;
    Ok(out)
}

/// Downscale by an exact factor — the resize variant.
pub fn resize(img: &RgbImage, factor: f64) -> RgbImage {
    let w = ((img.width() as f64 * factor).round() as u32).max(1);
    let h = ((img.height() as f64 * factor).round() as u32).max(1);
    image::imageops::resize(img, w, h, FilterType::CatmullRom)
}

/// Centered crop keeping `frac` of each side — the crop variant.
pub fn crop_center(img: &RgbImage, frac: f64) -> RgbImage {
    let w = ((img.width() as f64 * frac).round() as u32).max(1);
    let h = ((img.height() as f64 * frac).round() as u32).max(1);
    let x = (img.width() - w) / 2;
    let y = (img.height() - h) / 2;
    image::imageops::crop_imm(img, x, y, w, h).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SceneSpec {
        SceneSpec {
            width: 96,
            height: 72,
            bg: [PaletteColor::Orange, PaletteColor::Purple],
            angle_deg: 30.0,
            shapes: vec![
                ShapeSpec {
                    kind: ShapeKind::Circle,
                    color: PaletteColor::Blue,
                    cx: 0.3,
                    cy: 0.4,
                    size: 0.2,
                },
                ShapeSpec {
                    kind: ShapeKind::Square,
                    color: PaletteColor::Red,
                    cx: 0.7,
                    cy: 0.6,
                    size: 0.15,
                },
            ],
        }
    }

    #[test]
    fn rendering_is_deterministic() {
        let a = to_png(&render(&spec()).expect("render")).expect("png");
        let b = to_png(&render(&spec()).expect("render")).expect("png");
        assert_eq!(a, b);
        assert!(a.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn tags_come_from_the_spec_in_both_languages() {
        let s = spec();
        assert_eq!(s.tags_en(), vec!["blue circle", "red square"]);
        assert_eq!(s.tags_ja(), vec!["青い円", "赤い四角"]);
    }

    #[test]
    fn captions_read_from_the_same_ground_truth() {
        let s = spec();
        assert_eq!(
            s.caption_en(),
            "A blue circle and a red square on an orange-purple gradient background."
        );
        assert_eq!(
            s.caption_ja(),
            "オレンジと紫のグラデーション背景に青い円と赤い四角。"
        );
    }

    #[test]
    fn duplicate_shapes_collapse_into_one_tag() {
        let mut s = spec();
        s.shapes.push(s.shapes[0].clone());
        assert_eq!(s.tags_en().len(), 2);
    }

    #[test]
    fn transforms_change_bytes_but_not_identity_inputs() {
        let img = render(&spec()).expect("render");
        let base = to_png(&img).expect("png");

        let resized = resize(&img, 0.5);
        assert_eq!((resized.width(), resized.height()), (48, 36));
        assert_ne!(to_png(&resized).expect("png"), base);

        let cropped = crop_center(&img, 0.8);
        assert_eq!((cropped.width(), cropped.height()), (77, 58));

        let jpeg = to_jpeg(&img, 75).expect("jpeg");
        assert!(jpeg.starts_with(&[0xFF, 0xD8]));
    }

    #[test]
    fn noise_is_deterministic_and_seed_scoped() {
        let a = noise_image(7, 32, 32);
        let b = noise_image(7, 32, 32);
        let c = noise_image(8, 32, 32);
        assert_eq!(a.as_raw(), b.as_raw());
        assert_ne!(a.as_raw(), c.as_raw());
    }
}
