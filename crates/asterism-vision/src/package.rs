//! The model package: the data contract between model *preparation*
//! and model *use* (#112).
//!
//! A package is a directory holding the ONNX towers, the tokenizer,
//! and a `manifest.json` naming what they are: the model id, vector
//! dimension, preprocessing revision, license, official source, and a
//! SHA-256 per file. Preparation (the provider-side tooling) writes
//! packages; this module is the app-side reader — no logic crosses the
//! boundary in either direction, only these bytes.
//!
//! [`ModelPackage::open`] verifies every digest before the package is
//! usable. Verification is at open time, not per encode: the open
//! happens once per binding (startup, or a model install), and a
//! package that fails it is reported as the corruption it is rather
//! than served.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Name of the manifest file inside a package directory.
pub const MANIFEST_FILE: &str = "manifest.json";

/// One file the package carries, with the digest that pins it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFile {
    /// Path relative to the package directory.
    pub path: String,
    /// Lowercase hex SHA-256 of the file's bytes.
    pub sha256: String,
}

/// `manifest.json` — the identity half of the data contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    /// Stable id; the key of every derived row the model produces.
    pub model_id: String,
    /// Joint-space dimensionality the towers produce.
    pub dim: u32,
    /// Revision of the preprocessing recipe the image tower expects.
    pub preprocess_ver: u32,
    /// License of the weights, by SPDX-ish name (`"apache-2.0"`).
    pub license: String,
    /// Where the weights officially come from.
    pub source_url: String,
    /// The image tower (ONNX).
    pub image_model: PackageFile,
    /// The text tower (ONNX).
    pub text_model: PackageFile,
    /// The tokenizer definition (`tokenizer.json`).
    pub tokenizer: PackageFile,
}

/// An opened, digest-verified package.
#[derive(Debug, Clone)]
pub struct ModelPackage {
    manifest: PackageManifest,
    root: PathBuf,
}

impl ModelPackage {
    /// Reads and verifies a package directory.
    ///
    /// Every carried file must exist and hash to its manifest digest;
    /// the first mismatch fails the open, because serving a partially
    /// corrupt model would produce vectors under the intact model's
    /// identity.
    pub fn open(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("cannot read {}", manifest_path.display()))?;
        let manifest: PackageManifest = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a package manifest", manifest_path.display()))?;
        let package = Self {
            manifest,
            root: dir.to_path_buf(),
        };
        for file in [
            &package.manifest.image_model,
            &package.manifest.text_model,
            &package.manifest.tokenizer,
        ] {
            package.verify(file)?;
        }
        Ok(package)
    }

    fn verify(&self, file: &PackageFile) -> Result<()> {
        let path = self.root.join(&file.path);
        let bytes =
            std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if digest != file.sha256.to_lowercase() {
            bail!(
                "digest mismatch for {}: manifest says {}, bytes say {digest}",
                path.display(),
                file.sha256
            );
        }
        Ok(())
    }

    /// The verified manifest.
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    /// Absolute path of the image tower.
    pub fn image_model_path(&self) -> PathBuf {
        self.root.join(&self.manifest.image_model.path)
    }

    /// Absolute path of the text tower.
    pub fn text_model_path(&self) -> PathBuf {
        self.root.join(&self.manifest.text_model.path)
    }

    /// Absolute path of the tokenizer definition.
    pub fn tokenizer_path(&self) -> PathBuf {
        self.root.join(&self.manifest.tokenizer.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_package(dir: &Path, corrupt: bool) -> PackageManifest {
        let files = [
            ("image.onnx", b"img".as_slice()),
            ("text.onnx", b"txt"),
            ("tokenizer.json", b"tok"),
        ];
        let mut digests = Vec::new();
        for (name, bytes) in files {
            std::fs::write(dir.join(name), bytes).unwrap();
            digests.push(format!("{:x}", Sha256::digest(bytes)));
        }
        if corrupt {
            std::fs::write(dir.join("text.onnx"), b"tampered").unwrap();
        }
        let manifest = PackageManifest {
            model_id: "test-model".into(),
            dim: 4,
            preprocess_ver: 1,
            license: "apache-2.0".into(),
            source_url: "https://example.invalid/model".into(),
            image_model: PackageFile {
                path: "image.onnx".into(),
                sha256: digests[0].clone(),
            },
            text_model: PackageFile {
                path: "text.onnx".into(),
                sha256: digests[1].clone(),
            },
            tokenizer: PackageFile {
                path: "tokenizer.json".into(),
                sha256: digests[2].clone(),
            },
        };
        std::fs::write(
            dir.join(MANIFEST_FILE),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        manifest
    }

    #[test]
    fn a_package_opens_when_every_digest_agrees() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = write_package(dir.path(), false);
        let package = ModelPackage::open(dir.path()).expect("open");
        assert_eq!(package.manifest(), &manifest);
        assert!(package.image_model_path().ends_with("image.onnx"));
    }

    #[test]
    fn a_tampered_file_fails_the_open() {
        let dir = tempfile::tempdir().unwrap();
        write_package(dir.path(), true);
        let err = ModelPackage::open(dir.path()).unwrap_err();
        assert!(err.to_string().contains("digest mismatch"), "{err}");
    }

    #[test]
    fn a_missing_manifest_is_its_own_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = ModelPackage::open(dir.path()).unwrap_err();
        assert!(err.to_string().contains("cannot read"), "{err}");
    }
}
