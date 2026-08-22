# asterism-vision::package

The model package: the data contract between model *preparation*
and model *use* (#112).

A package is a directory holding the ONNX towers, the tokenizer,
and a `manifest.json` naming what they are: the model id, vector
dimension, preprocessing revision, license, official source, and a
SHA-256 per file. Preparation (the provider-side tooling) writes
packages; this module is the app-side reader — no logic crosses the
boundary in either direction, only these bytes.

[`ModelPackage::open`] verifies every digest before the package is
usable. Verification is at open time, not per encode: the open
happens once per binding (startup, or a model install), and a
package that fails it is reported as the corruption it is rather
than served.

## Types

- `ModelPackage` — An opened, digest-verified package.
- `PackageFile` — One file the package carries, with the digest that pins it.
- `PackageManifest` — `manifest.json` — the identity half of the data contract.

## Constants

- `MANIFEST_FILE` — Name of the manifest file inside a package directory.

