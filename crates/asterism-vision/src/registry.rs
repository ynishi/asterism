//! The model registry entry (#126): the fetch half of the data
//! contract [`crate::package`] reads.
//!
//! A registry entry is what `asterism-model-lab registry` authors — a
//! package manifest joined with the download URL of every carried
//! file, plus the qualification report when one was embedded. The
//! instance re-serves it verbatim (it is a carrier, not an authority);
//! this module is where the bytes become typed again, on the only two
//! sides that read them: the provider that authors an entry and the
//! app that installs from one.
//!
//! The entry is the trust anchor. Every downloaded byte is verified
//! against the entry's digests before it lands ([`Staging::accept`]),
//! and the finished directory must pass the same
//! [`ModelPackage::open`] the binder uses ([`Staging::finalize`]) —
//! transport can corrupt or tamper, and either way the install fails
//! rather than binds.
//!
//! ## Where the pieces run
//!
//! Everything here is filesystem and hashing — deliberately no
//! network, so the whole install path is unit-testable with bytes made
//! up on the spot. The download loop lives in the app's job handler
//! (`model_fetch`), which feeds what it fetched through [`Staging`].
//!
//! ## The staging directory is not under `models/`
//!
//! The binder counts every `models/` subdirectory holding a
//! `manifest.json` and refuses to bind when there is more than one. A
//! staging area inside `models/` would become a second package the
//! moment its manifest lands, and a crash between that write and the
//! final rename would leave the profile ambiguous — feature off — for
//! no reason a person can see. Staging therefore lives beside
//! `models/`, and the last step is one rename in.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::package::{MANIFEST_FILE, ModelPackage, PackageFile, PackageManifest};

/// The entry schema this crate authors and consumes.
pub const ENTRY_SCHEMA_V1: &str = "asterism-model-registry-entry-v1";

/// One carried file: the manifest pair plus where its bytes live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryFile {
    /// Flat file name inside the package directory (validated by
    /// [`RegistryEntry::parse`]: no separators, no traversal).
    pub path: String,
    /// Lowercase hex SHA-256 the bytes must hash to — the anchor the
    /// install verifies against.
    pub sha256: String,
    /// Where to download the bytes.
    pub url: String,
}

/// A registry entry — the manifest's identity fields, a URL per file,
/// and the embedded qualification when the provider attached one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Always [`ENTRY_SCHEMA_V1`]; [`Self::parse`] refuses anything
    /// else rather than guessing at a future shape.
    pub schema: String,
    /// Stable id; the directory name the install lands under.
    pub model_id: String,
    /// Joint-space dimensionality the towers produce.
    pub dim: u32,
    /// Revision of the preprocessing recipe the image tower expects.
    pub preprocess_ver: u32,
    /// License of the weights, by SPDX-ish name.
    pub license: String,
    /// Where the weights officially come from.
    pub source_url: String,
    /// The image tower (ONNX).
    pub image_model: RegistryFile,
    /// The text tower (ONNX).
    pub text_model: RegistryFile,
    /// The tokenizer definition.
    pub tokenizer: RegistryFile,
    /// The `qualify` report embedded at authoring time, if any. Opaque
    /// here: measurements are for a person to read, not for the
    /// install to gate on.
    #[serde(default)]
    pub qualification: Option<serde_json::Value>,
}

impl RegistryEntry {
    /// Parses an entry and validates what the install relies on: the
    /// schema tag, and that every file name is flat — a path with a
    /// separator or a dot-dot would let an entry write outside its
    /// package directory, so it is refused here, before any byte is
    /// fetched.
    pub fn parse(raw: &str) -> Result<Self> {
        let entry: Self = serde_json::from_str(raw).context("not a model registry entry")?;
        if entry.schema != ENTRY_SCHEMA_V1 {
            bail!(
                "entry schema {:?} is not {ENTRY_SCHEMA_V1:?}; refusing to guess at its shape",
                entry.schema
            );
        }
        for file in entry.files() {
            let flat = !file.path.is_empty()
                && !file.path.contains(['/', '\\'])
                && file.path != "."
                && file.path != "..";
            if !flat {
                bail!("entry file name {:?} is not a flat file name", file.path);
            }
            if file.path == MANIFEST_FILE {
                bail!(
                    "entry file name {MANIFEST_FILE:?} collides with the manifest the install writes"
                );
            }
        }
        Ok(entry)
    }

    /// The carried files, manifest order.
    pub fn files(&self) -> [&RegistryFile; 3] {
        [&self.image_model, &self.text_model, &self.tokenizer]
    }

    /// The package manifest this entry describes — what
    /// [`Staging::finalize`] writes, so an installed package and a
    /// prepared one are indistinguishable to the reader.
    pub fn manifest(&self) -> PackageManifest {
        let file = |f: &RegistryFile| PackageFile {
            path: f.path.clone(),
            sha256: f.sha256.clone(),
        };
        PackageManifest {
            model_id: self.model_id.clone(),
            dim: self.dim,
            preprocess_ver: self.preprocess_ver,
            license: self.license.clone(),
            source_url: self.source_url.clone(),
            image_model: file(&self.image_model),
            text_model: file(&self.text_model),
            tokenizer: file(&self.tokenizer),
        }
    }
}

/// Whether `models_dir` already holds this entry's package, byte-for-
/// byte: the manifest matches and every digest verifies through the
/// reader. A directory that exists but does not verify answers `false`
/// — the install replaces it rather than trusting its name.
pub fn is_installed(models_dir: &Path, entry: &RegistryEntry) -> Result<bool> {
    let dir = models_dir.join(&entry.model_id);
    if !dir.join(MANIFEST_FILE).exists() {
        return Ok(false);
    }
    match ModelPackage::open(&dir) {
        Ok(package) => Ok(package.manifest() == &entry.manifest()),
        Err(_) => Ok(false),
    }
}

/// Removes every package directory under `models_dir` other than
/// `keep`, returning the names removed. This is what makes an install
/// a **replacement** (#126 decision 1): the binder refuses a directory
/// holding two packages, so anything left beside the new one would
/// turn the feature off.
pub fn retire_other_packages(models_dir: &Path, keep: &str) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for dir_entry in std::fs::read_dir(models_dir)
        .with_context(|| format!("cannot read {}", models_dir.display()))?
    {
        let path = dir_entry?.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        if name == keep || !path.is_dir() || !path.join(MANIFEST_FILE).exists() {
            continue;
        }
        std::fs::remove_dir_all(&path)
            .with_context(|| format!("cannot retire {}", path.display()))?;
        removed.push(name);
    }
    Ok(removed)
}

/// An install in progress: a per-model directory under the staging
/// root, filled file by file and moved into `models/` whole.
///
/// Resumable by construction — the job pipeline has no retry policy,
/// so a re-run must pick up where the last one stopped:
/// [`Self::needs`] answers with the files whose staged bytes are
/// missing or fail their digest, and everything already landed is
/// skipped rather than re-downloaded.
#[derive(Debug)]
pub struct Staging {
    dir: PathBuf,
    entry: RegistryEntry,
}

impl Staging {
    /// Opens (or resumes) the staging directory for this entry.
    pub fn begin(staging_root: &Path, entry: &RegistryEntry) -> Result<Self> {
        let dir = staging_root.join(&entry.model_id);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create staging dir {}", dir.display()))?;
        Ok(Self {
            dir,
            entry: entry.clone(),
        })
    }

    /// The files still to fetch: absent, or present with bytes that do
    /// not hash to the entry's digest (a torn write from an
    /// interrupted run — re-fetched, not trusted).
    pub fn needs(&self) -> Result<Vec<RegistryFile>> {
        let mut needs = Vec::new();
        for file in self.entry.files() {
            let path = self.dir.join(&file.path);
            let staged_ok = match std::fs::read(&path) {
                Ok(bytes) => digest_hex(&bytes) == file.sha256.to_lowercase(),
                Err(_) => false,
            };
            if !staged_ok {
                needs.push(file.clone());
            }
        }
        Ok(needs)
    }

    /// Verifies fetched bytes against the entry's digest and stages
    /// them. A mismatch stages nothing: the entry is the trust anchor,
    /// and bytes that do not hash to it — a tampering carrier, a
    /// truncated download — never touch the directory.
    pub fn accept(&self, file: &RegistryFile, bytes: &[u8]) -> Result<()> {
        let digest = digest_hex(bytes);
        if digest != file.sha256.to_lowercase() {
            bail!(
                "digest mismatch for {}: entry says {}, bytes say {digest}",
                file.path,
                file.sha256
            );
        }
        let path = self.dir.join(&file.path);
        std::fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(())
    }

    /// Completes the install: writes the manifest, verifies the staged
    /// directory through the same [`ModelPackage::open`] the binder
    /// uses, retires every other package, and renames the staging
    /// directory into `models/`.
    ///
    /// What the ordering guarantees is that the new package only ever
    /// appears **whole** — verification precedes the one rename that
    /// makes it visible. What it does not guarantee is `models/`
    /// untouched on failure: retirement runs before the rename, so a
    /// failure in that window leaves no package installed. That
    /// window is accepted because it heals — the staged files are all
    /// verified by then, and a re-run finalizes without fetching —
    /// where the reverse order (rename first, retire after) would
    /// fail into the two-package shape the binder refuses, which no
    /// re-run of this install would fix once the *new* package is the
    /// one a person meant to keep.
    pub fn finalize(self, models_dir: &Path) -> Result<PathBuf> {
        let manifest_path = self.dir.join(MANIFEST_FILE);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&self.entry.manifest())?,
        )
        .with_context(|| format!("cannot write {}", manifest_path.display()))?;
        ModelPackage::open(&self.dir).context("staged package failed verification")?;
        retire_other_packages(models_dir, &self.entry.model_id)?;
        let target = models_dir.join(&self.entry.model_id);
        if target.exists() {
            // The same id, not verifying as current (is_installed said
            // no, or the caller skipped asking): replaced whole.
            std::fs::remove_dir_all(&target)
                .with_context(|| format!("cannot retire {}", target.display()))?;
        }
        std::fs::rename(&self.dir, &target).with_context(|| {
            format!(
                "cannot move {} into place as {}",
                self.dir.display(),
                target.display()
            )
        })?;
        Ok(target)
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_for(files: [(&str, &[u8]); 3]) -> RegistryEntry {
        let file = |(name, bytes): (&str, &[u8])| RegistryFile {
            path: name.to_string(),
            sha256: digest_hex(bytes),
            url: format!("https://example.invalid/{name}"),
        };
        RegistryEntry {
            schema: ENTRY_SCHEMA_V1.to_string(),
            model_id: "test-model".into(),
            dim: 4,
            preprocess_ver: 1,
            license: "apache-2.0".into(),
            source_url: "https://example.invalid/model".into(),
            image_model: file(files[0]),
            text_model: file(files[1]),
            tokenizer: file(files[2]),
            qualification: None,
        }
    }

    const FILES: [(&str, &[u8]); 3] = [
        ("image.onnx", b"img"),
        ("text.onnx", b"txt"),
        ("tokenizer.json", b"tok"),
    ];

    #[test]
    fn an_entry_round_trips_through_serde_and_parse() {
        let entry = entry_for(FILES);
        let raw = serde_json::to_string_pretty(&entry).unwrap();
        let parsed = RegistryEntry::parse(&raw).unwrap();
        assert_eq!(parsed, entry);
        // The manifest it derives is the package contract, field for
        // field.
        assert_eq!(parsed.manifest().model_id, "test-model");
        assert_eq!(parsed.manifest().image_model.path, "image.onnx");
    }

    #[test]
    fn a_wrong_schema_and_an_unflat_path_are_refused() {
        let mut entry = entry_for(FILES);
        entry.schema = "asterism-model-registry-entry-v2".into();
        let raw = serde_json::to_string(&entry).unwrap();
        assert!(
            RegistryEntry::parse(&raw)
                .unwrap_err()
                .to_string()
                .contains("schema")
        );

        for bad in ["../escape.onnx", "sub/dir.onnx", "", MANIFEST_FILE] {
            let mut entry = entry_for(FILES);
            entry.image_model.path = bad.into();
            let raw = serde_json::to_string(&entry).unwrap();
            assert!(
                RegistryEntry::parse(&raw).is_err(),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn staging_accepts_only_bytes_that_hash_to_the_entry() {
        let root = tempfile::tempdir().unwrap();
        let entry = entry_for(FILES);
        let staging = Staging::begin(root.path(), &entry).unwrap();

        let err = staging.accept(&entry.image_model, b"tampered").unwrap_err();
        assert!(err.to_string().contains("digest mismatch"), "{err}");
        // Nothing landed: the file is still needed.
        assert_eq!(staging.needs().unwrap().len(), 3);
    }

    #[test]
    fn needs_resumes_past_what_already_landed() {
        let root = tempfile::tempdir().unwrap();
        let entry = entry_for(FILES);
        let staging = Staging::begin(root.path(), &entry).unwrap();
        staging.accept(&entry.image_model, b"img").unwrap();

        // A torn write from an interrupted run fails its digest and is
        // re-offered; the good file is not.
        std::fs::write(root.path().join("test-model/text.onnx"), b"torn").unwrap();
        let needs: Vec<String> = staging
            .needs()
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(needs, vec!["text.onnx", "tokenizer.json"]);
    }

    #[test]
    fn finalize_installs_verifies_and_retires_the_previous_package() {
        let home = tempfile::tempdir().unwrap();
        let models = home.path().join("models");
        let staging_root = home.path().join("models-staging");
        std::fs::create_dir_all(&models).unwrap();

        // A previous package sits installed.
        let old = models.join("old-model");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join(MANIFEST_FILE), "{}").unwrap();
        // A non-package directory is none of the install's business.
        std::fs::create_dir_all(models.join("not-a-package")).unwrap();

        let entry = entry_for(FILES);
        let staging = Staging::begin(&staging_root, &entry).unwrap();
        for (file, (_, bytes)) in entry.files().map(Clone::clone).iter().zip(FILES) {
            staging.accept(file, bytes).unwrap();
        }
        let target = staging.finalize(&models).unwrap();

        assert_eq!(target, models.join("test-model"));
        assert!(is_installed(&models, &entry).unwrap());
        assert!(!old.exists(), "the previous package is retired");
        assert!(
            models.join("not-a-package").exists(),
            "a directory with no manifest is left alone"
        );
        assert!(
            !staging_root.join("test-model").exists(),
            "staging moved, not copied"
        );
    }

    #[test]
    fn is_installed_answers_false_for_a_absent_or_corrupt_package() {
        let home = tempfile::tempdir().unwrap();
        let models = home.path().join("models");
        std::fs::create_dir_all(&models).unwrap();
        let entry = entry_for(FILES);
        assert!(!is_installed(&models, &entry).unwrap());

        // Install, then corrupt one file: the name still matches, the
        // bytes do not, and the answer is what the reader says.
        let staging = Staging::begin(&home.path().join("staging"), &entry).unwrap();
        for (file, (_, bytes)) in entry.files().map(Clone::clone).iter().zip(FILES) {
            staging.accept(file, bytes).unwrap();
        }
        staging.finalize(&models).unwrap();
        assert!(is_installed(&models, &entry).unwrap());
        std::fs::write(models.join("test-model/text.onnx"), b"tampered").unwrap();
        assert!(!is_installed(&models, &entry).unwrap());
    }
}
