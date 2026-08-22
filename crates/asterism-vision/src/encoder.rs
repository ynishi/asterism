//! The ONNX Runtime encoder: pixels or text in, one normalized vector
//! out (#112).
//!
//! Loads a digest-verified [`ModelPackage`](crate::package::ModelPackage)
//! into two `ort` sessions (image tower, text tower) plus the packaged
//! tokenizer, and owns the preprocessing recipe the package's
//! `preprocess_ver` names. For revision 1 that is the SigLIP recipe:
//! resize to 256×256 (Catmull-Rom bicubic, no crop), rescale each
//! channel to `[-1, 1]`, NCHW; text is tokenized and padded to a fixed
//! 64-token window. Every vector is L2-normalized on the way out, so
//! similarity downstream is a plain dot product.
//!
//! The declared dimension is asserted against the tower's actual
//! output at first use — the manifest promised it, the model decides
//! it, and a disagreement is a broken package rather than a silent
//! reinterpretation of every stored vector.

use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use ort::session::Session;
use ort::value::TensorRef;
use tokenizers::Tokenizer;

use crate::package::ModelPackage;

/// Side length preprocessing revision 1 resizes to.
pub const PREPROCESS_SIDE: u32 = 256;
/// Token window preprocessing revision 1 pads/truncates text to — the
/// SigLIP text preprocessing default.
pub const TEXT_TOKENS: usize = 64;

/// A loaded model package, ready to encode.
pub struct Encoder {
    image: Session,
    text: Session,
    tokenizer: Tokenizer,
    model_id: String,
    dim: u32,
    preprocess_ver: u32,
}

impl Encoder {
    /// Loads the towers and tokenizer of an opened package.
    pub fn load(package: &ModelPackage) -> Result<Self> {
        let manifest = package.manifest();
        if manifest.preprocess_ver != 1 {
            bail!(
                "package {} declares preprocess_ver {}, and this build only knows revision 1",
                manifest.model_id,
                manifest.preprocess_ver
            );
        }
        let image = Session::builder()?
            .commit_from_file(package.image_model_path())
            .with_context(|| format!("cannot load image tower of {}", manifest.model_id))?;
        let text = Session::builder()?
            .commit_from_file(package.text_model_path())
            .with_context(|| format!("cannot load text tower of {}", manifest.model_id))?;
        let tokenizer = Tokenizer::from_file(package.tokenizer_path())
            .map_err(|e| anyhow::anyhow!("cannot load tokenizer of {}: {e}", manifest.model_id))?;
        Ok(Self {
            image,
            text,
            tokenizer,
            model_id: manifest.model_id.clone(),
            dim: manifest.dim,
            preprocess_ver: manifest.preprocess_ver,
        })
    }

    /// Stable id of the loaded model.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Vector dimensionality, as declared and asserted.
    pub fn dim(&self) -> u32 {
        self.dim
    }

    /// Preprocessing revision the recipe below implements.
    pub fn preprocess_ver(&self) -> u32 {
        self.preprocess_ver
    }

    /// Encodes one decoded image (tightly-packed RGB8).
    pub fn encode_image(&mut self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<f32>> {
        let expected = width as usize * height as usize * 3;
        if rgb.len() != expected {
            bail!(
                "rgb buffer is {} bytes, dimensions say {expected}",
                rgb.len()
            );
        }
        let pixels = preprocess_rev1(rgb, width, height)?;
        let side = PREPROCESS_SIDE as usize;
        let input = TensorRef::from_array_view(([1usize, 3, side, side], pixels.as_slice()))?;
        let outputs = self.image.run(ort::inputs!["pixel_values" => input])?;
        let vector = pooled_output(&outputs, self.dim)?;
        Ok(l2_normalize(vector))
    }

    /// Encodes a text (a tag name, a query) into the same space.
    pub fn encode_text(&mut self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenize failed: {e}"))?;
        let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        // Fixed window, the SigLIP text recipe: truncate long, pad
        // short with the tokenizer's pad id (0 when it has none).
        ids.truncate(TEXT_TOKENS);
        let pad = self
            .tokenizer
            .get_padding()
            .map(|p| p.pad_id as i64)
            .unwrap_or(0);
        ids.resize(TEXT_TOKENS, pad);
        let input = TensorRef::from_array_view(([1usize, TEXT_TOKENS], ids.as_slice()))?;
        let outputs = self.text.run(ort::inputs!["input_ids" => input])?;
        let vector = pooled_output(&outputs, self.dim)?;
        Ok(l2_normalize(vector))
    }
}

/// Revision 1: resize to [`PREPROCESS_SIDE`]² (bicubic, no crop),
/// rescale to `[-1, 1]`, NCHW.
fn preprocess_rev1(rgb: &[u8], width: u32, height: u32) -> Result<Vec<f32>> {
    let img = image::RgbImage::from_raw(width, height, rgb.to_vec())
        .context("rgb buffer does not match the declared dimensions")?;
    let resized = image::imageops::resize(
        &img,
        PREPROCESS_SIDE,
        PREPROCESS_SIDE,
        FilterType::CatmullRom,
    );
    let side = PREPROCESS_SIDE as usize;
    let mut out = vec![0f32; 3 * side * side];
    for (x, y, pixel) in resized.enumerate_pixels() {
        let (x, y) = (x as usize, y as usize);
        for channel in 0..3 {
            out[channel * side * side + y * side + x] = pixel.0[channel] as f32 / 127.5 - 1.0;
        }
    }
    Ok(out)
}

/// Picks the pooled embedding out of a tower's outputs: the output
/// named `pooler_output` when the export names one, otherwise the sole
/// output. The declared dimension is asserted here — the one place
/// every encode passes through.
fn pooled_output(outputs: &ort::session::SessionOutputs<'_>, dim: u32) -> Result<Vec<f32>> {
    if let Some(value) = outputs.get("pooler_output") {
        return extract_vector(value, dim);
    }
    let (_, value) = outputs
        .iter()
        .next()
        .context("the tower returned no outputs")?;
    extract_vector(&value, dim)
}

fn extract_vector(value: &ort::value::Value, dim: u32) -> Result<Vec<f32>> {
    let (shape, data) = value.try_extract_tensor::<f32>()?;
    let last = shape.iter().last().copied().unwrap_or(0) as usize;
    if last != dim as usize {
        bail!(
            "the tower produced dim {last}, the manifest declared {dim} — \
             a broken package, not a reinterpretation"
        );
    }
    // [1, dim] (or [1, tokens, dim] would disagree above): the batch of
    // one collapses to the vector.
    Ok(data[data.len() - last..].to_vec())
}

fn l2_normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    for x in &mut v {
        *x /= norm;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_lands_on_the_unit_sphere() {
        let v = l2_normalize(vec![3.0, 4.0]);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-6);
    }

    #[test]
    fn preprocessing_rescales_to_minus_one_one_nchw() {
        // A 2×1 image: pure white and pure black.
        let rgb = [255u8, 255, 255, 0, 0, 0];
        let out = preprocess_rev1(&rgb, 2, 1).unwrap();
        let side = PREPROCESS_SIDE as usize;
        assert_eq!(out.len(), 3 * side * side);
        assert!(out.iter().all(|v| (-1.0..=1.0).contains(v)));
        // The left half resolves near white (+1), the right near black.
        assert!(out[0] > 0.5, "top-left red channel should be near +1");
        assert!(out[side - 1] < -0.5, "top-right should be near -1");
    }
}
