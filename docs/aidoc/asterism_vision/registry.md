# asterism-vision::registry

The model registry entry (#126): the fetch half of the data
contract [`crate::package`] reads.

A registry entry is what `asterism-model-lab registry` authors — a
package manifest joined with the download URL of every carried
file, plus the qualification report when one was embedded. The
instance re-serves it verbatim (it is a carrier, not an authority);
this module is where the bytes become typed again, on the only two
sides that read them: the provider that authors an entry and the
app that installs from one.

The entry is the trust anchor. Every downloaded byte is verified
against the entry's digests before it lands ([`Staging::accept`]),
and the finished directory must pass the same
[`ModelPackage::open`] the binder uses ([`Staging::finalize`]) —
transport can corrupt or tamper, and either way the install fails
rather than binds.

## Where the pieces run

Everything here is filesystem and hashing — deliberately no
network, so the whole install path is unit-testable with bytes made
up on the spot. The download loop lives in the app's job handler
(`model_fetch`), which feeds what it fetched through [`Staging`].

## The staging directory is not under `models/`

The binder counts every `models/` subdirectory holding a
`manifest.json` and refuses to bind when there is more than one. A
staging area inside `models/` would become a second package the
moment its manifest lands, and a crash between that write and the
final rename would leave the profile ambiguous — feature off — for
no reason a person can see. Staging therefore lives beside
`models/`, and the last step is one rename in.

## Functions

- `is_installed` — Whether `models_dir` already holds this entry's package, byte-for-
- `retire_other_packages` — Removes every package directory under `models_dir` other than

## Types

- `RegistryEntry` — A registry entry — the manifest's identity fields, a URL per file,
- `RegistryFile` — One carried file: the manifest pair plus where its bytes live.
- `Staging` — An install in progress: a per-model directory under the staging

## Constants

- `ENTRY_SCHEMA_V1` — The entry schema this crate authors and consumes.

