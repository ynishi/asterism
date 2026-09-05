#!/usr/bin/env python3
"""Holds the two licence planes apart, in the tree rather than in prose.

Four crates carry `AGPL-3.0-or-later` and every other member inherits the
workspace's `MIT OR Apache-2.0`. README's licence section states what that
arrangement rests on: the direction an `asterism-*` crate depending on a
`teams-*` crate would open stays empty. Until this script that was a claim
nothing checked. `crates/asterism-ui/src-tauri/tests/boundary.rs` is the
boundary test that sits closest to it, and its subject is the wire crate's
vocabulary rather than the licence — so a line reading
`teams-core = { path = "../../teams-core" }` in the UI manifest would have
passed it, passed clippy, and passed every recipe `check` composes.

Two assertions, and the second is the one worth having:

- **The planes are declared.** A member whose package name begins `teams-`
  is licensed `AGPL-3.0-or-later`; every other member is licensed
  `MIT OR Apache-2.0`. Without this the second assertion can be defeated by
  deleting a line rather than by adding one — a `teams-core` that stopped
  declaring AGPL would simply read as permissive here.
- **The forbidden direction is empty.** No member on the permissive plane
  reaches an AGPL member through the dependency graph.

Both answers come from `cargo metadata`, and every part of that choice is
load-bearing:

- **`--locked`** makes the resolution answer for the manifests as they are
  now. A crossing written into a manifest and not yet resolved is a failure
  here rather than a pass, which is what a check reading a stale `Cargo.lock`
  by itself would have given.
- **The resolver's licence field** rather than a line scan of the manifests.
  A regex over TOML cannot see which table a key sits in, so a `license`
  under `[package.metadata.*]` reads as the package's own; and a manifest
  shaped in a way the scan does not expect drops the member out of the check
  silently, in the passing direction. What is compared here is the licence
  cargo itself resolves, inheritance included.
- **The resolved graph** rather than the manifests' own dependency tables.
  The edge that matters is not always written where it is made: of the team
  plane, `asterism-ui` names `asterism-teams-client` and nothing else, and
  what makes that safe is everything `asterism-teams-client` does not reach.
  A manifest-only check answers for one hop and calls it a boundary.

The graph is read with no platform filter and with every dependency kind in
it, so a dev-dependency and a target-gated dependency both count. That is
stricter than distribution requires — a test binary is not shipped, and a
Windows-only edge is not in a macOS bundle — and each of those is an
over-approximation in the safe direction. The arrangement being guarded is
that the permissive plane does not link the AGPL one at all, and a rule with
an exception for test binaries is a rule with a place to put the next
exception.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AGPL = "AGPL-3.0-or-later"
PERMISSIVE = "MIT OR Apache-2.0"
AGPL_PREFIX = "teams-"


def metadata() -> dict:
    """`cargo metadata` for this workspace, or exit saying why not."""
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--offline",
            "--locked",
            "--manifest-path",
            str(ROOT / "Cargo.toml"),
        ],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.exit(
            "cargo metadata --locked --offline failed, so nothing here can "
            "answer for the workspace. A lockfile that does not match the "
            "manifests is the usual cause, and running a build or "
            "`cargo metadata` without --locked is what settles it:\n"
            + result.stderr.strip()
        )
    return json.loads(result.stdout)


def reaches(start: str, edges: dict[str, list[str]], targets: set[str]) -> list[str]:
    """The shortest path from `start` to any target, or an empty list."""
    seen = {start}
    queue: list[list[str]] = [[start]]
    while queue:
        path = queue.pop(0)
        for dep in edges.get(path[-1], []):
            if dep in targets:
                return path + [dep]
            if dep not in seen:
                seen.add(dep)
                queue.append(path + [dep])
    return []


def main() -> int:
    meta = metadata()
    packages = {p["id"]: p for p in meta["packages"]}
    members = list(meta["workspace_members"])
    if not members:
        sys.exit("cargo metadata reported no workspace members")

    resolve = meta.get("resolve")
    if not resolve:
        sys.exit("cargo metadata reported no resolved graph to walk")
    edges = {node["id"]: list(node["dependencies"]) for node in resolve["nodes"]}

    failures: list[str] = []
    agpl: set[str] = set()
    permissive: set[str] = set()
    for member in sorted(members, key=lambda i: packages[i]["name"]):
        package = packages[member]
        name = package["name"]
        licence = package.get("license")
        wanted = AGPL if name.startswith(AGPL_PREFIX) else PERMISSIVE
        if licence == wanted:
            (agpl if wanted == AGPL else permissive).add(member)
            continue
        said = f"is licensed {licence}" if licence else "declares no licence"
        where = Path(package["manifest_path"]).relative_to(ROOT)
        failures.append(
            f"{where}: {name} {said}, and a member "
            + (
                f"named {AGPL_PREFIX}* is the teams plane, licensed {wanted}"
                if wanted == AGPL
                else f"outside the teams plane is licensed {wanted}"
            )
        )

    if not agpl:
        failures.append(
            f"no {AGPL} member found — either the teams plane went, and this "
            f"script goes with it, or its declarations did"
        )

    for member in sorted(permissive, key=lambda i: packages[i]["name"]):
        path = reaches(member, edges, agpl)
        if path:
            names = [packages[i]["name"] for i in path]
            failures.append(
                f"{names[0]} reaches {names[-1]} ({AGPL}) through "
                f"{' -> '.join(names)}: the permissive plane does not link "
                f"the AGPL one, and this edge would put AGPL code wherever "
                f"{names[0]} ships"
            )

    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(
        f"licence planes: {len(agpl)} AGPL member(s), {len(permissive)} on the "
        f"workspace licence, and none of the second reaches the first"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
