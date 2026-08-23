//! # asterism-model-lab — provider-side model preparation (#112)
//!
//! The preparation half of the model split. The app *uses* a model
//! package (`asterism-vision` reads it, digest-verified); this tool is
//! how a package comes to exist and how it earns its place: download
//! the towers from their official source, pin their digests into the
//! manifest, verify the package exactly the way the app will read it,
//! and qualify it against the fixture set. (A `registry` verb once
//! authored a distribution entry here; #132 retired that flow — the
//! encoder ships with the app, and what travels is the trained head.)
//!
//! Deliberately a separate binary in the `asterism-import` category —
//! the actor is the provider, not the user, and nothing in the app's
//! dependency graph reaches back here. The dependency runs the other
//! way: this tool links `asterism-vision` so that `verify` and
//! `qualify` are the app's own reading of the package, not a second
//! implementation of it.
//!
//! ## Charter, and what v1 leaves out
//!
//! `convert` (ONNX export / quantization for a model whose publisher
//! ships none) and `train` belong to this tool's charter and are not
//! implemented: the one supported model has official ONNX exports, and
//! #112 scopes training out. The recipe table below is where a model
//! that needs conversion would declare it.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use asterism_vision::encoder::Encoder;
use asterism_vision::fixtures::eval::{EvalConfig, run as run_eval};
use asterism_vision::fixtures::scene::noise_image;
use asterism_vision::package::{MANIFEST_FILE, ModelPackage, PackageFile, PackageManifest};

/// One supported model: where its files officially live and what the
/// manifest should say about them. Compiled in on purpose — a recipe
/// is a provider decision, and code review is where those happen.
#[derive(Debug)]
struct Recipe {
    model_id: &'static str,
    dim: u32,
    preprocess_ver: u32,
    license: &'static str,
    source_url: &'static str,
    /// `(remote URL, local file name)` per carried file, in manifest
    /// order: image tower, text tower, tokenizer.
    files: [(&'static str, &'static str); 3],
}

const RECIPES: &[Recipe] = &[Recipe {
    model_id: "siglip2-base-patch16-256",
    dim: 768,
    preprocess_ver: 1,
    license: "apache-2.0",
    source_url: "https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX",
    files: [
        (
            "https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX/resolve/main/onnx/vision_model.onnx",
            "vision_model.onnx",
        ),
        (
            "https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX/resolve/main/onnx/text_model.onnx",
            "text_model.onnx",
        ),
        (
            "https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX/resolve/main/tokenizer.json",
            "tokenizer.json",
        ),
    ],
}];

fn recipe(model_id: &str) -> Result<&'static Recipe> {
    RECIPES
        .iter()
        .find(|r| r.model_id == model_id)
        .with_context(|| {
            let known: Vec<&str> = RECIPES.iter().map(|r| r.model_id).collect();
            format!("no recipe for {model_id:?}; known models: {known:?}")
        })
}

#[derive(Debug, Parser)]
#[command(
    name = "asterism-model-lab",
    version,
    about = "Provider-side model preparation: prepare, verify, qualify"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Download a known model's files from their official source and
    /// write the digest-pinned package manifest.
    Prepare {
        /// A model id the recipe table knows.
        model_id: String,
        /// Package directory to create (defaults to `./<model_id>`).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Open, load, and smoke a package exactly the way the app will:
    /// digest verification, session creation, one image and one text
    /// encode with the declared dimension asserted.
    Verify {
        /// Package directory.
        dir: PathBuf,
    },
    /// Measure a package against the fixture set and print the
    /// evaluation JSON, including the suggested floors a person
    /// reviews before they reach any constant.
    Qualify {
        /// Package directory.
        dir: PathBuf,
        /// Fixture seed (the measurement's identity).
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// Base scenes to walk.
        #[arg(long, default_value_t = 24)]
        bases: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Prepare { model_id, out } => prepare(&model_id, out).await,
        Command::Verify { dir } => verify(&dir),
        Command::Qualify { dir, seed, bases } => qualify(&dir, seed, bases),
    }
}

async fn prepare(model_id: &str, out: Option<PathBuf>) -> Result<()> {
    let recipe = recipe(model_id)?;
    let out = out.unwrap_or_else(|| PathBuf::from(recipe.model_id));
    std::fs::create_dir_all(&out)
        .with_context(|| format!("cannot create package dir {}", out.display()))?;

    let client = reqwest::Client::new();
    let mut digests = Vec::new();
    for (url, name) in recipe.files {
        let path = out.join(name);
        eprintln!("model-lab: fetching {name} from {url}");
        let bytes = client
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .with_context(|| format!("cannot fetch {url}"))?
            .bytes()
            .await
            .with_context(|| format!("cannot read the body of {url}"))?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        std::fs::write(&path, &bytes)
            .with_context(|| format!("cannot write {}", path.display()))?;
        eprintln!("model-lab: {name} sha256={digest} ({} bytes)", bytes.len());
        digests.push(digest);
    }

    let manifest = PackageManifest {
        model_id: recipe.model_id.to_string(),
        dim: recipe.dim,
        preprocess_ver: recipe.preprocess_ver,
        license: recipe.license.to_string(),
        source_url: recipe.source_url.to_string(),
        image_model: PackageFile {
            path: recipe.files[0].1.to_string(),
            sha256: digests[0].clone(),
        },
        text_model: PackageFile {
            path: recipe.files[1].1.to_string(),
            sha256: digests[1].clone(),
        },
        tokenizer: PackageFile {
            path: recipe.files[2].1.to_string(),
            sha256: digests[2].clone(),
        },
    };
    let manifest_path = out.join(MANIFEST_FILE);
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
        .with_context(|| format!("cannot write {}", manifest_path.display()))?;

    // The package must satisfy the app's own reading before this
    // command may call it prepared.
    verify(&out)?;
    eprintln!("model-lab: package ready at {}", out.display());
    Ok(())
}

fn verify(dir: &Path) -> Result<()> {
    let package = ModelPackage::open(dir)?;
    let manifest = package.manifest();
    let mut encoder = Encoder::load(&package)?;

    // One encode per tower: the declared dimension is asserted inside
    // the encoder (its one place), so surviving these two calls is the
    // assertion.
    let img = noise_image(1, 64, 64);
    encoder.encode_image(img.as_raw(), 64, 64)?;
    encoder.encode_text("a red circle")?;
    eprintln!(
        "model-lab: verified {} (dim {}, preprocess_ver {}, license {})",
        manifest.model_id, manifest.dim, manifest.preprocess_ver, manifest.license
    );
    Ok(())
}

fn qualify(dir: &Path, seed: u64, bases: usize) -> Result<()> {
    let package = ModelPackage::open(dir)?;
    let mut encoder = Encoder::load(&package)?;
    let outcome = run_eval(&mut encoder, &EvalConfig { seed, bases })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&outcome.to_json(encoder.model_id()))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recipe_is_internally_consistent() {
        for recipe in RECIPES {
            assert!(!recipe.model_id.is_empty());
            assert!(recipe.dim > 0);
            for (url, name) in recipe.files {
                assert!(url.starts_with("https://"), "{url}");
                assert!(!name.contains('/'), "local names are flat: {name}");
            }
            // The recipe's files download from under its source.
            let host = |u: &str| u.split('/').nth(2).map(str::to_string);
            for (url, _) in recipe.files {
                assert_eq!(host(url), host(recipe.source_url), "{url}");
            }
        }
    }

    #[test]
    fn unknown_models_are_refused_with_the_known_list() {
        let err = recipe("no-such-model").unwrap_err();
        assert!(
            err.to_string().contains("siglip2-base-patch16-256"),
            "{err}"
        );
    }

    #[test]
    fn cli_parses_the_three_verbs_and_no_longer_the_retired_one() {
        use clap::Parser;
        for args in [
            vec!["asterism-model-lab", "prepare", "siglip2-base-patch16-256"],
            vec!["asterism-model-lab", "verify", "/tmp/p"],
            vec!["asterism-model-lab", "qualify", "/tmp/p", "--bases", "8"],
        ] {
            Cli::try_parse_from(args).expect("parse");
        }
        // `registry` authored the fetch flow's entry; #132 retired
        // both sides together, so the verb refusing is part of the
        // retirement.
        assert!(Cli::try_parse_from(["asterism-model-lab", "registry", "/tmp/p"]).is_err());
    }
}
