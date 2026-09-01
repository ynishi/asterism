#!/usr/bin/env python3
"""Holds scripts/cross-member-readers.txt to the tree, from both sides.

`changed-packages` selects packages by the member directory containing
each changed path, plus the reader list in that file — a test reading a
file under another member's directory runs only when its own crate is
selected, and the list is what arranges that. #206 is what the absence
of any such arrangement cost: the reader existed, nothing selected it,
and main's full run was the first thing to say so. A list is only
better than none while something holds it to the tree, which is this
script's job. It fails when the two disagree in either direction:

- a cross-member read with no covering line (the silent gap);
- a line no test witnesses any more (dead weight that over-selects).

A read counts wherever the member compiles it: the scan walks every
`.rs` file under each member's directory, so a `#[cfg(test)]` module
under `src/` weighs the same as an integration test under `tests/` —
where a test lives does not change what has to select it.

What counts as a read is what the scan can see, and that boundary is
worth knowing. Three literal shapes are recognized: a
workspace-relative path (`crates/<member>/...`), a parent-relative
path (`../<member>/...`, resolved against the owning member), and —
only in a file that also hops a directory upward with `join("..")` — a
quoted string equal to a member directory's name, which is how a
helper that assembles `../<crate>/tests/fixtures/<file>` names its
target. That last shape is verified at member granularity: the scan
cannot see the assembled tail, so it asks only that some line address
that member for this reader. A path built by any other means is
invisible here. Line comments are stripped so prose mentioning a
member does not count as a read — block comments are not understood,
and none of the scanned sources carries one — and a member's
references to its own files never count. The walk is over the
filesystem rather than the tracked tree, so a stray `.rs` under a
member is scanned too; what that buys is a loud local red, never a
silent gap.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
READERS = ROOT / "scripts" / "cross-member-readers.txt"

WORKSPACE_PATH = re.compile(r"crates/[A-Za-z0-9_./-]+")
PARENT_PATH = re.compile(r"\.\./[A-Za-z0-9_./-]+")
QUOTED = re.compile(r'"([A-Za-z0-9_-]+)"')


def members() -> dict[str, str]:
    """Member directory -> package name, read from the manifests."""
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
            dirs.append(line.strip().strip('",'))
    out: dict[str, str] = {}
    for d in dirs:
        manifest = ROOT / d / "Cargo.toml"
        if not manifest.is_file():
            continue
        name = re.search(
            r'^\[package\][^[]*?^name\s*=\s*"([^"]+)"',
            manifest.read_text(),
            re.M | re.S,
        )
        if name:
            out[d] = name.group(1)
    if not out:
        sys.exit("no workspace members parsed out of Cargo.toml")
    return out


def stripped(text: str) -> str:
    """The source with `//` comments removed, string contents kept.

    Tracked character by character rather than cut at the first `//`,
    because a URL inside a string would otherwise truncate the rest of
    the line holding it.
    """
    out: list[str] = []
    in_string = False
    i = 0
    while i < len(text):
        c = text[i]
        if in_string:
            out.append(c)
            if c == "\\" and i + 1 < len(text):
                out.append(text[i + 1])
                i += 2
                continue
            if c == '"':
                in_string = False
            i += 1
            continue
        if c == '"':
            in_string = True
            out.append(c)
            i += 1
            continue
        if c == "/" and text[i : i + 2] == "//":
            while i < len(text) and text[i] != "\n":
                i += 1
            continue
        out.append(c)
        i += 1
    return "".join(out)


def owning(path: Path, dirs: list[str]) -> str | None:
    rel = path.relative_to(ROOT).as_posix()
    hits = [d for d in dirs if rel.startswith(d + "/")]
    return max(hits, key=len) if hits else None


def referenced(ref: str, dirs: list[str]) -> str | None:
    hits = [d for d in dirs if ref == d or ref.startswith(d + "/")]
    return max(hits, key=len) if hits else None


def refs_in(path: Path, owner_dir: str, dirs: list[str]) -> set[tuple[str, bool]]:
    """(referenced path, member_granularity) pairs, self-references dropped."""
    text = stripped(path.read_text())
    found: set[tuple[str, bool]] = set()
    for m in WORKSPACE_PATH.finditer(text):
        ref = m.group(0).rstrip("./")
        target = referenced(ref, dirs)
        if target and target != owner_dir:
            found.add((ref, False))
    for m in PARENT_PATH.finditer(text):
        resolved = (ROOT / owner_dir / m.group(0)).resolve()
        try:
            ref = resolved.relative_to(ROOT).as_posix()
        except ValueError:
            continue
        target = referenced(ref, dirs)
        if target and target != owner_dir:
            found.add((ref, False))
    if 'join("..")' in text:
        basenames = {d.rsplit("/", 1)[-1]: d for d in dirs}
        for m in QUOTED.finditer(text):
            target = basenames.get(m.group(1))
            if target and target != owner_dir:
                found.add((target + "/", True))
    return found


def covers(prefix: str, ref: str, member_level: bool) -> bool:
    if member_level:
        return prefix.startswith(ref)
    return ref.startswith(prefix) or (ref + "/").startswith(prefix)


def main() -> int:
    table: list[tuple[str, str]] = []
    for line in READERS.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        prefix, _, reader = line.partition("|")
        table.append((prefix, reader))

    dir_to_pkg = members()
    dirs = sorted(dir_to_pkg)

    reads: set[tuple[str, bool, str]] = set()
    for d in dirs:
        for source in sorted((ROOT / d).rglob("*.rs")):
            for ref, member_level in refs_in(source, d, dirs):
                reads.add((ref, member_level, dir_to_pkg[d]))

    failures: list[str] = []
    rel = READERS.relative_to(ROOT)
    for ref, member_level, reader in sorted(reads):
        if not any(
            r == reader and covers(p, ref, member_level) for p, r in table
        ):
            if member_level:
                failures.append(
                    f"a test in {reader} assembles paths under {ref} and "
                    f"no line in {rel} addresses that member for it — add "
                    f"'{ref}|{reader}' (or a longer path under it) in this "
                    f"change"
                )
            else:
                failures.append(
                    f"a test in {reader} reads {ref} and no line in {rel} "
                    f"covers it — add '{ref}|{reader}' (or a prefix of it) "
                    f"in this change"
                )
    for prefix, reader in table:
        if not any(
            r == reader and covers(prefix, ref, member_level)
            for ref, member_level, r in reads
        ):
            failures.append(
                f"'{prefix}|{reader}' is witnessed by no test — the read "
                f"moved or went, so the line goes in the same change"
            )

    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(
        f"cross-member readers: {len(reads)} read(s) in the tree, "
        f"{len(table)} line(s), both directions agree"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
