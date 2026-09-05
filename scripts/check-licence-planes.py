#!/usr/bin/env python3
"""Holds the two licence planes apart, in the tree rather than in prose.

Four crates carry `AGPL-3.0-or-later` and every other member inherits the
workspace's `MIT OR Apache-2.0`. README's licence section states what that
arrangement rests on: the direction an `asterism-*` crate depending on a
`teams-*` crate would open stays empty. Until this script that was a claim
nothing checked. `crates/asterism-ui/src-tauri/tests/boundary.rs` is the one
mechanical boundary test in the tree and its subject is the wire crate's
vocabulary, so a line reading `teams-core = { path = "../../teams-core" }` in
the UI manifest passed it, passed clippy, passed every recipe `check`
composes, and put AGPL code inside a notarized app.

Two assertions, and the second is the one worth having:

- **The planes are declared.** A member whose package name begins `teams-`
  says `license = "AGPL-3.0-or-later"`; every other member says
  `license.workspace = true`. Without this the second assertion can be
  defeated by deleting a line rather than by adding one — a `teams-core`
  that stopped declaring AGPL would simply read as permissive here.
- **The forbidden direction is empty.** No member on the permissive plane
  reaches an AGPL member through the lockfile's dependency graph.

The closure is read out of `Cargo.lock` rather than out of the manifests,
because the edge that matters is not always written down where it is made:
`asterism-ui` names `asterism-teams-client` and nothing else, and what makes
that safe is everything `asterism-teams-client` does *not* reach. A
manifest-only check would answer for one hop and call it a boundary.

`Cargo.lock` merges normal, build and dev dependencies into one list, so a
dev-dependency on an AGPL crate counts here too. That is deliberate and it is
stricter than distribution requires — a test binary is not shipped — but the
arrangement being guarded is that the permissive plane does not link the AGPL
one at all, and a rule with an exception for test binaries is a rule with a
place to put the next exception.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AGPL = "AGPL-3.0-or-later"
AGPL_PREFIX = "teams-"

PACKAGE_NAME = re.compile(r'^\[package\][^[]*?^name\s*=\s*"([^"]+)"', re.M | re.S)
LICENCE = re.compile(r"^license\s*=\s*\"([^\"]+)\"", re.M)
INHERITED = re.compile(r"^license\.workspace\s*=\s*true", re.M)


def members() -> dict[str, str]:
    """Package name -> manifest path, relative, read from the root manifest."""
    lines = (ROOT / "Cargo.toml").read_text().splitlines()
    dirs: list[str] = []
    inside = False
    for line in lines:
        if line.startswith("members"):
            inside = True
            continue
        if inside and line.startswith("]"):
            break
        if inside:
            entry = line.strip()
            if entry.startswith("#") or not entry:
                continue
            dirs.append(entry.strip('",'))
    out: dict[str, str] = {}
    for d in dirs:
        manifest = ROOT / d / "Cargo.toml"
        if not manifest.is_file():
            continue
        name = PACKAGE_NAME.search(manifest.read_text())
        if name:
            out[name.group(1)] = f"{d}/Cargo.toml"
    if not out:
        sys.exit("no workspace members parsed out of Cargo.toml")
    return out


def graph() -> dict[str, set[str]]:
    """Package name -> the names it depends on, out of the lockfile.

    Versions are dropped: two copies of one crate at different versions
    are the same node for this question, and no workspace member has a
    second version of itself to confuse with.
    """
    out: dict[str, set[str]] = {}
    name: str | None = None
    deps: set[str] = set()
    in_deps = False
    for line in (ROOT / "Cargo.lock").read_text().splitlines():
        if line.startswith("[[package]]"):
            if name:
                out.setdefault(name, set()).update(deps)
            name, deps, in_deps = None, set(), False
            continue
        if line.startswith("name = "):
            name = line.split('"')[1]
            continue
        if line.startswith("dependencies = ["):
            in_deps = True
            continue
        if in_deps:
            if line.startswith("]"):
                in_deps = False
                continue
            deps.add(line.strip().strip('",').split(" ")[0])
    if name:
        out.setdefault(name, set()).update(deps)
    return out


def reaches(start: str, edges: dict[str, set[str]], targets: set[str]) -> list[str]:
    """The first path from `start` to any target, or an empty list."""
    seen = {start}
    queue: list[list[str]] = [[start]]
    while queue:
        path = queue.pop(0)
        for dep in sorted(edges.get(path[-1], set())):
            if dep in targets:
                return path + [dep]
            if dep not in seen:
                seen.add(dep)
                queue.append(path + [dep])
    return []


def main() -> int:
    manifests = members()
    failures: list[str] = []

    agpl: set[str] = set()
    permissive: set[str] = set()
    for name, manifest in sorted(manifests.items()):
        text = (ROOT / manifest).read_text()
        declared = LICENCE.search(text)
        inherits = INHERITED.search(text) is not None
        if name.startswith(AGPL_PREFIX):
            if declared and declared.group(1) == AGPL:
                agpl.add(name)
            else:
                said = declared.group(1) if declared else "the workspace licence"
                failures.append(
                    f"{manifest}: a {AGPL_PREFIX}* crate says {said}, and the "
                    f"teams plane is licensed {AGPL} — declare it at the field "
                    f"or move the crate off the prefix"
                )
        elif inherits and not declared:
            permissive.add(name)
        else:
            said = declared.group(1) if declared else "no licence at all"
            failures.append(
                f"{manifest}: says {said}; a member outside the teams plane "
                f"takes license.workspace = true, so that the workspace "
                f"manifest stays the one place the permissive terms are stated"
            )

    if not agpl:
        failures.append(
            f"no {AGPL} member found — either the teams plane went, and this "
            f"script goes with it, or its declarations did"
        )

    edges = graph()
    for name in sorted(permissive):
        path = reaches(name, edges, agpl)
        if path:
            failures.append(
                f"{name} reaches {path[-1]} ({AGPL}) through "
                f"{' -> '.join(path)}: the permissive plane does not link the "
                f"AGPL one, and this edge would put AGPL code wherever "
                f"{name} ships"
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
