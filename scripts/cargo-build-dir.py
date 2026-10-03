#!/usr/bin/env python3
"""Say where cargo puts this workspace's build output, and what lives there.

Cargo decides where its build directory is — `CARGO_TARGET_DIR`,
`build.target-dir` in a config file, or its own default — and every one of
those is legitimate on somebody's machine. A tracked file that wrote the
location down would be right only for the default, so nothing here writes it
down: this asks cargo (`cargo metadata --no-deps`, which reads the manifest
and touches no network) and prints what it says.

Anything this workspace writes into that directory by hand gets its path from
here, so the writer and every reader agree: today that is the ffmpeg sidecar —
the build script that writes it, the recipes that hand it to the Tauri CLI,
and the release workflow that uploads its source.

That agreement cannot hold for one setting, so this refuses it rather than
answer: a relative `CARGO_TARGET_DIR`, or a relative `CARGO_BUILD_TARGET_DIR`
when that is the one in force. Cargo resolves a relative path taken from the
environment against the working directory of whoever runs it, and the readers
here run from different ones — the repository root, `crates/asterism-ui`
after a recipe's `cd`, Tauri's own cargo under `src-tauri` — so each would be
told a different directory, and none would be wrong by cargo's rule. Set it to
an absolute path instead. A relative `build.target-dir` in a config file is
not refused: cargo resolves that against the directory holding the file's
`.cargo/`, whoever asks.

Cargo finds config files by walking up from the working directory, not from
the manifest, so run this from inside the checkout whose answer you want.

Usage: cargo-build-dir.py [--manifest-path PATH]
                          [dir | sidecar-dir | sidecar-config]

  --manifest-path PATH  the workspace to ask about (default: the one this
                        script sits in)
  dir                   cargo's build directory (the default)
  sidecar-dir           the directory `scripts/build-ffmpeg-sidecar.sh`
                        writes
  sidecar-config        a Tauri config, as JSON, naming the sidecar as the
                        bundle's one external binary — the value for a
                        `--config` given after the file it is merged over
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

manifest = ROOT / "Cargo.toml"


class Refused(Exception):
    pass


def refuse_relative_env() -> None:
    # `CARGO_TARGET_DIR` outranks `build.target-dir`, and so the variable
    # that sets the latter, so only the first one present is in force. An
    # empty value counts as relative: cargo fails on it, and this says why.
    name = "CARGO_TARGET_DIR"
    value = os.environ.get(name)
    if value is None:
        name = "CARGO_BUILD_TARGET_DIR"
        value = os.environ.get(name)
    if value is not None and not Path(value).is_absolute():
        raise Refused(
            f"{name}={value!r} is relative; cargo resolves it against each "
            "caller's working directory, and the callers here run from "
            "different ones, so there is no one build directory to name. "
            f"Set {name} to an absolute path."
        )


def build_dir() -> str:
    refuse_relative_env()
    out = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
            str(manifest),
        ],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout
    return json.loads(out)["target_directory"]


def sidecar_dir() -> str:
    return str(Path(build_dir()) / "ffmpeg-sidecar")


def sidecar_config() -> str:
    # The path without the `-<host-triple>` suffix: Tauri appends it when it
    # looks the binary up, which is why the build script writes it with one.
    binary = str(Path(sidecar_dir()) / "ffmpeg")
    return json.dumps({"bundle": {"externalBin": [binary]}})


def main(argv: list[str]) -> int:
    global manifest
    args = argv[1:]
    if len(args) >= 2 and args[0] == "--manifest-path":
        manifest = Path(args[1])
        args = args[2:]
    what = args[0] if args else "dir"
    answers = {
        "dir": build_dir,
        "sidecar-dir": sidecar_dir,
        "sidecar-config": sidecar_config,
    }
    if len(args) > 1 or what not in answers:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        print(answers[what]())
    except Refused as refused:
        print(f"cargo-build-dir.py: {refused}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
