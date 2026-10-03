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

Usage: cargo-build-dir.py [dir | sidecar-dir | sidecar-config]

  dir             cargo's build directory (the default)
  sidecar-dir     the directory `scripts/build-ffmpeg-sidecar.sh` writes
  sidecar-config  a Tauri config, as JSON, naming the sidecar as the
                  bundle's one external binary — the value for a
                  `--config` given after the file it is merged over
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def build_dir() -> str:
    out = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
            str(ROOT / "Cargo.toml"),
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
    what = argv[1] if len(argv) > 1 else "dir"
    answers = {
        "dir": build_dir,
        "sidecar-dir": sidecar_dir,
        "sidecar-config": sidecar_config,
    }
    if len(argv) > 2 or what not in answers:
        print(__doc__, file=sys.stderr)
        return 2
    print(answers[what]())
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
