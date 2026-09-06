#!/usr/bin/env python3
"""Gather the source archives the release has to offer beside the DMG.

`scripts/copyleft-sources.txt` names the packages whose licence obliges more
than attribution — six MPL-2.0 crates today. MPL-2.0 §3.2 asks that Covered
Software distributed in Executable Form also be made available in Source Code
Form, and this is the half of that answer the release path runs: the exact
`.crate` archive `Cargo.lock` pins, copied out of the registry cache so the
workflow can attach it.

The archives are what cargo itself downloaded and built from, which is what
makes them the corresponding source rather than a plausible substitute. They
are unmodified upstream releases, so no patch has to travel with them.

Fails rather than skipping. A release that quietly offered five of six
sources would be worse than one that stopped: the notice inside the app names
all six.

Usage: collect-copyleft-sources.py <destination directory>
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COPYLEFT = ROOT / "scripts" / "copyleft-sources.txt"


def wanted() -> list[str]:
    names = [
        line.split("#", 1)[0].strip()
        for line in COPYLEFT.read_text().splitlines()
    ]
    out = [n for n in names if n]
    if not out:
        sys.exit(f"{COPYLEFT.name} names no packages")
    return out


def pinned(names: list[str]) -> set[tuple[str, str]]:
    """Every (name, version) the lockfile holds for the recorded packages.

    Pairs, and every version rather than one: a graph this size resolves
    twenty package names at two versions, and a release that offered the
    source of one of two shipped versions would be answering half of what it
    owes. The sibling check makes the same distinction for the same reason.
    """
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--offline", "--locked",
         "--manifest-path", str(ROOT / "Cargo.toml")],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.exit("cargo metadata --locked --offline failed:\n" + result.stderr.strip())
    found = {
        (p["name"], p["version"])
        for p in json.loads(result.stdout)["packages"]
        if p["name"] in names
    }
    missing = sorted(set(names) - {n for n, _ in found})
    if missing:
        sys.exit(
            f"{', '.join(missing)} named in {COPYLEFT.name} but not in the "
            f"dependency graph — the list and the tree disagree, which "
            f"`third-party-check` answers for"
        )
    return found


def main() -> int:
    if len(sys.argv) != 2:
        sys.exit(f"usage: {Path(sys.argv[0]).name} <destination directory>")
    dest = Path(sys.argv[1])
    dest.mkdir(parents=True, exist_ok=True)

    # `CARGO_HOME` where it is set, which is how a CI runner moves the cache
    # somewhere it can be restored from.
    home = Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo")
    cache = sorted((home / "registry" / "cache").glob("*"))
    if not cache:
        sys.exit(
            f"no registry cache under {home / 'registry' / 'cache'} — nothing "
            f"has been downloaded here, so there is no source to offer"
        )

    copied: list[str] = []
    missing: list[str] = []
    for name, version in sorted(pinned(wanted())):
        archive = f"{name}-{version}.crate"
        source = next((c / archive for c in cache if (c / archive).is_file()), None)
        if source is None:
            missing.append(archive)
            continue
        shutil.copy2(source, dest / archive)
        copied.append(f"{archive} ({source.stat().st_size} bytes)")

    if missing:
        print(
            "these source archives are not in the registry cache, so the "
            "release cannot offer them:\n  " + "\n  ".join(missing) + "\n"
            "The cache holds the `.crate` a download produced; an unpacked "
            "`registry/src` alone does not put one back, and a restored cache "
            "can carry one directory and not the other. Fetching is what "
            "fills it: `cargo fetch --locked`.",
            file=sys.stderr,
        )
        return 1

    print(f"collected {len(copied)} source archive(s) into {dest}:")
    for line in copied:
        print("  " + line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
