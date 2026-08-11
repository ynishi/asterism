//! Image `RawItem` → `Footprint::Image` parser.
//!
//! Reads EXIF from the payload with `kamadak-exif` (V2 design D-15 —
//! Rust-side, never JS-side). Falls back to file mtime when no EXIF is
//! present so PNG screenshots and freshly-generated images still land
//! with a meaningful `occurred_at`.
//!
//! # One file, one footprint — including the text inside it
//!
//! This parser used to read the PNG's `tEXt` chunks and emit one
//! `Footprint::Note` per chunk, each addressed `<image>.png#<keyword>`,
//! so a ComfyUI export arrived as N + 1 assets. It does not any more:
//! **an image file is one record, and the text inside it is that
//! record's metadata**. The chunks reach the image's own row on the
//! `Meta` axis, and
//! nothing on this side has to carry them there — the server's
//! `material_hash` job already reads the artefact's bytes and hands
//! them to the PNG probe, which writes `material.meta_hash` and
//! `material.meta_kv` on the row this parser produced.
//!
//! So there is no hand-off to build here, and deliberately none to add:
//! a metadata set declared by the importer *and* computed from the
//! bytes by the server would be two authorities for one value, which is
//! the failure `declared_content_hash` is fenced against. The importer
//! states where the bytes are; what they contain is read from them.

use std::path::PathBuf;

use asterism_importer_sdk::{
    Footprint, FootprintSource, Image, ParseError, RawItem, SIDECAR_SUFFIX, SourceParser,
};
use asterism_media_probe::{coded_dims_with_exif, exif_fields};
use chrono::Utc;
use serde_json::json;

/// What the parser declares when it finds a sidecar. The server
/// reads the file and decides what it names — the parser only
/// reports that one is there.
const SIDECAR_DECLARATION: &str = "sidecar";

/// Parses one image file into one `Footprint::Image`.
pub struct ImageParser {
    /// Human-readable platform label (e.g. `"Camera roll"`) recorded
    /// on every emitted footprint. Optional — supplied by the CLI.
    platform: Option<String>,
}

impl ImageParser {
    /// Builds a parser tagged with `platform`.
    pub fn new(platform: Option<String>) -> Self {
        Self { platform }
    }
}

impl SourceParser for ImageParser {
    fn parse(&self, item: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let path = PathBuf::from(&item.locator);
        let parent_dir = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let file_size_bytes = item.extra.get("file_size_bytes").and_then(|v| v.as_u64());

        // EXIF is optional — many PNG / screenshot files ship without
        // it, and we still want to import those. `kamadak-exif` errs
        // when the container has no EXIF, which we treat as "just
        // fall back to file mtime".
        let exif_fields = exif_fields(&item.payload);

        let occurred_at = exif_fields
            .as_ref()
            .and_then(|f| f.datetime_original)
            .or(item.occurred_at)
            .unwrap_or_else(Utc::now);
        // EXIF is present on camera-origin files (JPEG / HEIC / TIFF)
        // but absent on most PNG screenshots, GIF, BMP, AI-generated
        // AVIF, etc. The probe falls back to a cheap header-only decode
        // so those still land with dimensions — the grid UI uses them
        // for layout, so `dims = None` is a UX floor we want to avoid
        // whenever possible.
        //
        // Passed the already-parsed EXIF rather than re-reading the
        // container: the composition ("EXIF first, header second") is
        // the shared definition, and this arm is only skipping a second
        // parse of bytes it just read.
        let dims = coded_dims_with_exif(exif_fields.as_ref(), &item.payload);
        let camera_make = exif_fields.as_ref().and_then(|f| f.camera_make.clone());
        let camera_model = exif_fields.as_ref().and_then(|f| f.camera_model.clone());
        let orientation = exif_fields.as_ref().and_then(|f| f.orientation);

        let mut labels = vec!["photo".to_string()];
        if !parent_dir.is_empty() {
            labels.push(parent_dir.clone());
        }
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            labels.push(ext.to_ascii_lowercase());
        }

        let extra = json!({
            "filename": path.file_name().map(|s| s.to_string_lossy().to_string()),
            "parent_dir": parent_dir,
            "camera_make": camera_make,
            "camera_model": camera_model,
            "orientation": orientation,
            "exif_seen": exif_fields.is_some(),
        });

        let alt = if stem.is_empty() { None } else { Some(stem) };

        // An exported artefact travels with a `<name>.meta.json`
        // sidecar naming the export it came out of. If one is sitting
        // next to this file, the image is very likely a return trip —
        // declare it and let the server resolve the link (it owns the
        // dispatch / asset lookup; the parser only reports what it
        // sees).
        let derived_from = std::path::Path::new(&format!("{}{}", item.locator, SIDECAR_SUFFIX))
            .is_file()
            .then(|| SIDECAR_DECLARATION.to_string());

        Ok(vec![Footprint::Image(Image {
            source: FootprintSource {
                kind: item.source_kind,
                locator: item.locator,
                platform: self.platform.clone(),
                external_id: None,
            },
            occurred_at,
            // Standalone image import — no conversation container.
            external_session_key: None,
            alt,
            dims,
            file_size_bytes,
            labels,
            // Nothing to group it with: the file's own text is its
            // metadata rather than a sibling asset, so a synthetic
            // bundle would have one member.
            bundle_id: None,
            extra,
            derived_from,
            // Empty, and measured rather than pending: the graph a
            // ComfyUI PNG carries has no identifier for the artefact —
            // its node ids are unique only inside the one file and its
            // input reference is a bare filename (`album_meta` module
            // doc). Seed / sampler / checkpoint are generation
            // parameters, and the chunks holding them are hashed off
            // the artefact's bytes server-side on the `Meta` axis
            // rather than read here. So there is nothing here to state,
            // which is a different situation from a value being dropped.
            album_meta: Default::default(),
        })])
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use asterism_importer_sdk::RawItem;
    use image::{ImageFormat, RgbImage};
    use serde_json::json;

    fn encode(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let img = RgbImage::new(width, height);
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, format)
            .expect("encode");
        buf.into_inner()
    }

    /// PNG's CRC-32 (the standard reflected polynomial), so a spliced
    /// chunk leaves the file a file.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A real PNG carrying real `tEXt` chunks: the encoder's own
    /// output with `keyword \0 text` chunks spliced in after `IHDR`.
    ///
    /// The header offset is asserted rather than assumed — a splice at
    /// the wrong offset would produce a file whose chunks nothing can
    /// find, and a fixture that carries no chunks proves nothing about
    /// a change whose subject is chunks.
    fn png_with_text(chunks: &[(&str, &str)]) -> Vec<u8> {
        const SIGNATURE: usize = 8;
        // length + type + 13 bytes of IHDR payload + CRC.
        const IHDR_END: usize = SIGNATURE + 4 + 4 + 13 + 4;

        let base = encode(4, 4, ImageFormat::Png);
        assert_eq!(
            &base[SIGNATURE + 4..SIGNATURE + 8],
            b"IHDR",
            "the encoder still writes IHDR first"
        );

        let mut out = base[..IHDR_END].to_vec();
        for (keyword, text) in chunks {
            let mut payload = keyword.as_bytes().to_vec();
            payload.push(0);
            payload.extend_from_slice(text.as_bytes());
            out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            let mut crc_input = b"tEXt".to_vec();
            crc_input.extend_from_slice(&payload);
            out.extend_from_slice(b"tEXt");
            out.extend_from_slice(&payload);
            out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        }
        out.extend_from_slice(&base[IHDR_END..]);
        out
    }

    /// How many `tEXt` chunk headers the bytes carry. Counting the
    /// literal is enough here: the fixtures put nothing else in the
    /// file that could spell it.
    fn text_chunk_count(bytes: &[u8]) -> usize {
        bytes.windows(4).filter(|w| *w == b"tEXt").count()
    }

    /// The chunks a ComfyUI-style export carries, in the two shapes the
    /// old extractor registry recognised (a `prompt` graph with a
    /// `CLIPTextEncode` node, a `vdsl` script) plus one it did not
    /// (`parameters`, which fell through to the generic extractor).
    /// Every one of the three would have become its own asset.
    fn three_chunks() -> Vec<u8> {
        png_with_text(&[
            (
                "prompt",
                r#"{"2":{"class_type":"CLIPTextEncode","inputs":{"text":"1girl, purple eyes"}}}"#,
            ),
            ("vdsl", r#"{"script":"local s = C.camera.medium_shot"}"#),
            ("parameters", "steps: 30, sampler: euler"),
        ])
    }

    fn parse_payload(locator: &str, payload: Vec<u8>) -> Vec<Footprint> {
        ImageParser::new(Some("Camera roll".into()))
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: locator.into(),
                payload,
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok")
    }

    /// **One PNG is one record.** The file's `tEXt` chunks are its
    /// metadata, not three more assets.
    ///
    /// The fixture carries three chunks, and that is the whole weight
    /// of the assertion: a PNG with no chunks yields one footprint
    /// under either the old code or the new, so `chunk_count > 1` is
    /// what makes "one" mean something. Before this change the same
    /// bytes produced four footprints — the image plus one
    /// `Footprint::Note` per chunk, each addressed
    /// `<image>.png#<keyword>`.
    #[test]
    fn a_png_carrying_several_text_chunks_is_one_footprint() {
        let payload = three_chunks();
        assert_eq!(
            text_chunk_count(&payload),
            3,
            "the fixture has to carry more than one chunk or it measures nothing"
        );

        let out = parse_payload("/tmp/comfy-export.png", payload);

        assert_eq!(
            out.len(),
            1,
            "one file, one footprint — not one plus a note per chunk"
        );
        let Footprint::Image(img) = &out[0] else {
            panic!("the one footprint is the image itself, got {:?}", out[0])
        };
        assert_eq!(
            img.bundle_id, None,
            "nothing to bundle it with: a synthetic group of one is not a group"
        );
        assert!(
            img.extra.get("png_text_keys").is_none(),
            "the chunk keywords are read off the bytes server-side, not summarised here: {}",
            img.extra
        );
    }

    /// **`#` has left image locators.** The delimiter addresses one
    /// record inside a container, and a PNG is not a container.
    ///
    /// Asserted over the specs rather than the footprints, because the
    /// spec's `locator` is the value that actually travels to the
    /// server, and over the chunk-carrying fixture rather than a bare
    /// PNG — a file with nothing in it to split on could not have been
    /// split whatever the code did.
    #[test]
    fn no_locator_from_an_image_import_carries_a_record_delimiter() {
        let payload = three_chunks();
        assert!(text_chunk_count(&payload) > 1, "see the test above");

        let specs: Vec<_> = parse_payload("/tmp/comfy-export.png", payload)
            .into_iter()
            .map(Footprint::into_asset_spec)
            .collect();

        assert_eq!(specs.len(), 1);
        for spec in &specs {
            assert!(
                !spec.locator.contains('#'),
                "an image addresses a whole file: {}",
                spec.locator
            );
        }
        assert_eq!(specs[0].locator, "/tmp/comfy-export.png");
    }

    // The header-probe cases that used to sit here moved to
    // `asterism-media-probe` with the function they exercise. What stays
    // in this file is the wiring: that `parse` reaches for the probe at
    // all, and which evidence it prefers when both are available.

    #[test]
    fn parser_emits_dims_for_exif_less_png() {
        // Screenshot-style PNG: no EXIF, header carries dims. This
        // used to land as `dims=None`; the fallback fixes it.
        let bytes = encode(100, 50, ImageFormat::Png);
        let parser = ImageParser::new(Some("Camera roll".into()));
        let item = RawItem {
            source_kind: "fs".into(),
            locator: "/tmp/screenshot.png".into(),
            payload: bytes,
            occurred_at: None,
            extra: json!({}),
        };
        let out = parser.parse(item).unwrap();
        let Footprint::Image(img) = &out[0] else {
            panic!("first footprint should be Image")
        };
        assert_eq!(img.dims, Some((100, 50)));
    }

    #[test]
    fn parser_emits_dims_for_gif() {
        let bytes = encode(80, 40, ImageFormat::Gif);
        let parser = ImageParser::new(None);
        let item = RawItem {
            source_kind: "fs".into(),
            locator: "/tmp/anim.gif".into(),
            payload: bytes,
            occurred_at: None,
            extra: json!({}),
        };
        let out = parser.parse(item).unwrap();
        let Footprint::Image(img) = &out[0] else {
            panic!()
        };
        assert_eq!(img.dims, Some((80, 40)));
        // Extension flows into labels so the grid can facet by format.
        assert!(img.labels.iter().any(|l| l == "gif"));
    }

    #[test]
    fn a_sidecar_next_to_the_file_is_declared_as_the_origin() {
        // The return leg of a round trip: the file came back with the
        // sidecar the exporter wrote beside it. The parser reports
        // that fact; resolving what the sidecar names is the server's
        // job (it owns the dispatch / asset lookup).
        let tmp = tempfile::tempdir().expect("tempdir");
        let image_path = tmp.path().join("returned.png");
        std::fs::write(
            format!("{}{}", image_path.display(), SIDECAR_SUFFIX),
            r#"{"id":"0198c1c2-0000-7000-8000-000000000001"}"#,
        )
        .expect("write sidecar");

        let parser = ImageParser::new(None);
        let out = parser
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: image_path.display().to_string(),
                payload: encode(10, 10, ImageFormat::Png),
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok");
        let Footprint::Image(img) = &out[0] else {
            panic!("first footprint should be Image")
        };
        assert_eq!(img.derived_from.as_deref(), Some("sidecar"));
    }

    #[test]
    fn a_file_without_a_sidecar_declares_nothing() {
        // Every ordinary photo import goes down this path. Declaring
        // an origin here would put an unresolvable claim on every
        // asset in the library.
        let tmp = tempfile::tempdir().expect("tempdir");
        let image_path = tmp.path().join("plain.png");

        let parser = ImageParser::new(None);
        let out = parser
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: image_path.display().to_string(),
                payload: encode(10, 10, ImageFormat::Png),
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok");
        let Footprint::Image(img) = &out[0] else {
            panic!("first footprint should be Image")
        };
        assert_eq!(img.derived_from, None);
    }

    // ---- Real fixture tests (α: broader format support) ---------------
    //
    // The fixtures are committed and synthetic: one still frame of
    // ffmpeg's `testsrc` per container, written by
    // `scripts/gen-test-fixtures.py`. They were previously downloaded
    // from the upstream test corpora of `libavif`, `image-rs` and
    // `libheif` and then committed anyway, which left this repository
    // redistributing third-party files (one of them LGPL-3.0) with no
    // attribution and a doc comment claiming they were not committed.
    //
    // What each format is asked to prove is unchanged: the header read
    // reaches `dims` without a decoder for that format being linked in.

    fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "fixture missing at {} ({e}). Regenerate with:\n  \
                 python3 scripts/gen-test-fixtures.py",
                path.display(),
            )
        })
    }

    fn parse_fixture(name: &str) -> Image {
        let bytes = fixture_bytes(name);
        let parser = ImageParser::new(Some("test".into()));
        let item = RawItem {
            source_kind: "fs".into(),
            locator: format!("tests/fixtures/{name}"),
            payload: bytes,
            occurred_at: None,
            extra: json!({}),
        };
        let out = parser.parse(item).expect("parse ok");
        match out.into_iter().next().expect("at least one footprint") {
            Footprint::Image(img) => img,
            other => panic!("first footprint should be Image, got {other:?}"),
        }
    }

    #[test]
    fn fixture_avif_lands_with_header_dims() {
        // `imagesize` reads AVIF headers natively (no libavif dep),
        // so the fallback cascade covers this format even though
        // `image` crate default features skip AVIF.
        //
        // 64×64 rather than the 1×1 the old upstream fixture used:
        // SVT-AV1 refuses to encode a 1×1 frame, and a synthetic
        // fixture we can regenerate is worth more here than the
        // smallest possible one.
        let img = parse_fixture("testcard.avif");
        assert_eq!(img.dims, Some((64, 64)), "AVIF header dims via imagesize");
        assert!(img.labels.iter().any(|l| l == "avif"));
    }

    #[test]
    fn fixture_gif_lands_with_header_dims() {
        let img = parse_fixture("testcard.gif");
        assert!(img.dims.is_some(), "GIF header dims");
        let (w, h) = img.dims.unwrap();
        assert!(w > 0 && h > 0, "positive dims");
        assert!(img.labels.iter().any(|l| l == "gif"));
    }

    #[test]
    fn fixture_tiff_lands_with_header_dims() {
        let img = parse_fixture("testcard.tiff");
        // 157×151 is deliberately not a round number: it is the one
        // format whose header read is pinned exactly rather than
        // "positive", so the size has to be one nothing else would
        // produce by default. `gen-test-fixtures.py` encodes it.
        assert_eq!(
            img.dims,
            Some((157, 151)),
            "TIFF header dims from real fixture"
        );
        assert!(img.labels.iter().any(|l| l == "tiff"));
    }

    #[test]
    fn fixture_bmp_lands_with_header_dims() {
        let img = parse_fixture("testcard.bmp");
        assert!(img.dims.is_some(), "BMP header dims");
        assert!(img.labels.iter().any(|l| l == "bmp"));
    }

    #[test]
    fn fixture_heic_lands_with_header_dims() {
        // `imagesize` reads HEIF/HEIC ftyp+ispe boxes without
        // libheif, so iPhone photos land with dims even though
        // `image` crate has no HEIC decoder in default features.
        //
        // The fixture comes out of macOS `sips` (ffmpeg has no HEIF
        // muxer), which is the same encoder path an iPhone photo takes
        // before it reaches a Mac.
        let img = parse_fixture("testcard.heic");
        assert!(img.labels.iter().any(|l| l == "heic"));
        assert!(
            img.dims.is_some(),
            "HEIC header dims via imagesize (got None)"
        );
        let (w, h) = img.dims.unwrap();
        assert!(w > 0 && h > 0);
    }
}
