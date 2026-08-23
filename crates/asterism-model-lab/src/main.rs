//! # asterism-model-lab — provider-side model preparation (#112)
//!
//! The preparation half of the model split. The app *uses* a model
//! package (`asterism-vision` reads it, digest-verified); this tool is
//! how a package comes to exist and how it earns its place: download
//! the towers from their official source, pin their digests into the
//! manifest, verify the package exactly the way the app will read it,
//! qualify it against the fixture set, and author the registry entry a
//! future in-app fetch flow consumes.
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
use asterism_vision::registry::{ENTRY_SCHEMA_V1, RegistryEntry, RegistryFile};

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

const RECIPES: &[Recipe] = &[
    // The bundled encoder (#132 phase 0): the current model with a
    // q4f16 vision tower over an int8 text tower — ~372 MB against
    // the fp32 pair's ~1.5 GB. Chosen by the fixture qualification
    // (seed 42, 24 bases): at its floor of 0.10 it holds precision
    // 0.29 / recall 0.79 against the fp32 recording of 0.32 / 0.68
    // at 0.12, Japanese matching intact. Two candidates measured and
    // rejected on the same fixture: the all-int8 pair (recall 0.59
    // at the same floor, 40 MB more) and SigLIP v1 224/int8
    // (Japanese zero, English recall 0.11 — its 32k vocabulary
    // against this family's 256k). URLs pin the repo revision by
    // commit, so what `prepare` fetches cannot drift under the
    // digests it records.
    Recipe {
        model_id: "siglip2-base-patch16-256-q4v",
        dim: 768,
        preprocess_ver: 1,
        license: "apache-2.0",
        source_url: "https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX",
        files: [
            (
                "https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX/resolve/d1114256522a37ffa257a0a58017348ab0058db2/onnx/vision_model_q4f16.onnx",
                "vision_model.onnx",
            ),
            (
                "https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX/resolve/d1114256522a37ffa257a0a58017348ab0058db2/onnx/text_model_int8.onnx",
                "text_model.onnx",
            ),
            (
                "https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX/resolve/d1114256522a37ffa257a0a58017348ab0058db2/tokenizer.json",
                "tokenizer.json",
            ),
        ],
    },
    Recipe {
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
    },
];

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
    about = "Provider-side model preparation: prepare, verify, qualify, registry"
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
    /// Author the registry entry for a prepared package — the artifact
    /// a future in-app fetch flow consumes. Reads the manifest, joins
    /// the recipe's official URLs, and embeds a qualification report
    /// when one is given.
    Registry {
        /// Package directory.
        dir: PathBuf,
        /// A `qualify` output file to embed.
        #[arg(long)]
        qualification: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Prepare { model_id, out } => prepare(&model_id, out).await,
        Command::Verify { dir } => verify(&dir),
        Command::Qualify { dir, seed, bases } => qualify(&dir, seed, bases),
        Command::Registry { dir, qualification } => registry(&dir, qualification.as_deref()),
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

fn registry(dir: &Path, qualification: Option<&Path>) -> Result<()> {
    let package = ModelPackage::open(dir)?;
    let manifest = package.manifest();
    // A registry entry exists so a fetch flow can download the files;
    // an entry with no URLs cannot do its one job, so an unknown model
    // is refused here the same way `prepare` refuses it.
    let recipe = recipe(&manifest.model_id)?;
    let qualification: Option<serde_json::Value> = match qualification {
        Some(path) => Some(
            serde_json::from_str(
                &std::fs::read_to_string(path)
                    .with_context(|| format!("cannot read {}", path.display()))?,
            )
            .with_context(|| format!("{} is not a qualification report", path.display()))?,
        ),
        None => None,
    };
    // The typed entry the app's fetch flow parses back
    // (`asterism_vision::registry::RegistryEntry::parse`) — authored
    // through the same type so the two sides cannot drift.
    let files = |file: &PackageFile, url: &str| RegistryFile {
        path: file.path.clone(),
        sha256: file.sha256.clone(),
        url: url.to_string(),
    };
    let entry = RegistryEntry {
        schema: ENTRY_SCHEMA_V1.to_string(),
        model_id: manifest.model_id.clone(),
        dim: manifest.dim,
        preprocess_ver: manifest.preprocess_ver,
        license: manifest.license.clone(),
        source_url: manifest.source_url.clone(),
        image_model: files(&manifest.image_model, recipe.files[0].0),
        text_model: files(&manifest.text_model, recipe.files[1].0),
        tokenizer: files(&manifest.tokenizer, recipe.files[2].0),
        qualification,
    };
    println!("{}", serde_json::to_string_pretty(&entry)?);
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
    fn cli_parses_the_four_verbs() {
        use clap::Parser;
        for args in [
            vec!["asterism-model-lab", "prepare", "siglip2-base-patch16-256"],
            vec!["asterism-model-lab", "verify", "/tmp/p"],
            vec!["asterism-model-lab", "qualify", "/tmp/p", "--bases", "8"],
            vec!["asterism-model-lab", "registry", "/tmp/p"],
        ] {
            Cli::try_parse_from(args).expect("parse");
        }
    }
}
