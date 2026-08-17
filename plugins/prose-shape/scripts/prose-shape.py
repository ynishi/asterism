#!/usr/bin/env python3
"""Refuse a write that gives a file the wrong prose shape.

Two shapes, decided by where the file goes rather than by what it says:

  - Tracked prose is read in an editor and in `git diff`, where
    re-flowing a paragraph is a whole-paragraph diff. It wraps.
  - A body GitHub renders is folded by the renderer. Hard wrapping it
    buys nothing and survives into the rendered page as nothing at all.

The widths are here and nowhere else on purpose. They used to be in
CONTRIBUTING, which is read start to finish by every agent that opens
the repository, and an agent that reads "wrapped at 72" wraps whatever
it writes next — including the pull request bodies that must not be.
Naming the exceptions in the same paragraph did not stop it: two
consecutive pull request bodies came in hand-wrapped, the second after
the exceptions had been spelled out. A width that is only ever quoted
back at the moment a file breaks it cannot leak into anything else.

Columns are characters, not bytes: an em dash is one column, and
counting it as three calls a 71-column line 73.
"""

import json
import os
import re
import sys

WRAP_AT = 72

# Read in an editor and in `git diff`.
WRAPPED = re.compile(
    r"^(README|CHANGELOG|CONTRIBUTING|PUBLIC_DEVELOPMENT|SECURITY)\.md$"
    r"|^docs/.*\.md$"
)
# Written to be posted: pull request and issue bodies, which CONTRIBUTING
# asks for as files under this directory.
RENDERED = re.compile(r"^workspace/.*\.md$")

FENCE = re.compile(r"^\s*(```|~~~)")
TABLE = re.compile(r"^\s*\|")
# A line that is one long link or path has nowhere to break.
UNBREAKABLE = re.compile(r"^\s*[-*]?\s*\[?[^ ]{60,}")


def body_lines(text):
    """Every line outside a fenced block, with its 1-based number."""
    fenced = False
    for number, line in enumerate(text.split("\n"), start=1):
        if FENCE.match(line):
            fenced = not fenced
            continue
        if not fenced:
            yield number, line


def too_wide(text):
    for number, line in body_lines(text):
        if len(line) <= WRAP_AT or TABLE.match(line) or UNBREAKABLE.match(line):
            continue
        # Only a line that could have been broken is one to complain
        # about: it has a space left of the limit to break at.
        if " " in line[:WRAP_AT]:
            return number, len(line)
    return None


def hand_wrapped(text):
    """Three or more body lines in a row, none reaching the margin.

    That is what a hand-wrapped paragraph looks like and what a
    paragraph written as one line never does.
    """
    run = 0
    for number, line in body_lines(text):
        stripped = line.strip()
        short = 40 <= len(line) <= WRAP_AT + 4
        listish = stripped.startswith(("-", "*", ">", "#", "|")) or re.match(
            r"^\d+\.", stripped
        )
        if short and not listish:
            run += 1
            if run >= 3:
                return number
        else:
            run = 0
    return None


def main():
    try:
        event = json.load(sys.stdin)
    except json.JSONDecodeError:
        return 0

    tool_input = event.get("tool_input") or {}
    path = tool_input.get("file_path") or ""
    if not path:
        return 0

    root = event.get("cwd") or os.getcwd()
    try:
        relative = os.path.relpath(path, root)
    except ValueError:
        return 0
    if relative.startswith(".."):
        return 0

    # Write carries the whole file; Edit carries the replacement.
    whole_file = "content" in tool_input
    text = tool_input.get("content") or tool_input.get("new_string") or ""
    if not text:
        return 0

    if WRAPPED.match(relative):
        wide = too_wide(text)
        if wide:
            number, width = wide
            print(
                f"{relative} is read in an editor and in `git diff`, so its"
                f" prose wraps at {WRAP_AT} columns."
                f" Line {number} of what you are writing is {width}."
                f" Break it. Tables, fenced blocks and unbreakable links are"
                f" exempt, and this check skips them.",
                file=sys.stderr,
            )
            return 2

    if RENDERED.match(relative) and whole_file:
        number = hand_wrapped(text)
        if number is not None:
            print(
                f"{relative} is a body GitHub renders, and the renderer folds"
                f" paragraphs itself. Line {number} sits in a run of"
                f" hand-wrapped lines: write each paragraph as one line and"
                f" let it fold. Lists, tables and fenced blocks keep their own"
                f" line breaks.",
                file=sys.stderr,
            )
            return 2

    return 0


if __name__ == "__main__":
    sys.exit(main())
