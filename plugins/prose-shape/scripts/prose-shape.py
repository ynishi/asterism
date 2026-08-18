#!/usr/bin/env python3
"""Refuse a write that hard-wraps a body GitHub is going to render.

Three widths apply to this repository, and two of them are checked by
something that runs over the tree:

  - A commit message body wraps at 72, because `git log` indents it four
    spaces inside an 80-column terminal. `just commit-msg-check`.
  - Markdown in the tree wraps at 80. `just md-check`.
  - A pull request or issue body is not wrapped at all. GitHub folds
    paragraphs itself, and a hard-wrapped one arrives carrying breaks
    nobody chose.

The third is what this hook is for, and the reason it is a hook rather
than a gate is that there is no tree to gate: CONTRIBUTING asks for
those bodies as files under `workspace/`, which is gitignored, so they
are never committed and no run ever sees them. The only moment anything
can look at one is the moment it is written.

It has happened twice — two consecutive pull request bodies arrived
wrapped at 72, the second after CONTRIBUTING had spelled out that
GitHub-rendered prose takes no wrapping. Guidance that has to be
remembered and scoped by its reader loses to a check that runs.
"""

import json
import os
import re
import sys

# Written to be posted: the pull request and issue bodies CONTRIBUTING
# asks for as files here.
RENDERED = re.compile(r"^workspace/.*\.md$")

FENCE = re.compile(r"^\s*(```|~~~)")
# A run this long says the writer was wrapping rather than writing.
RUN = 3
# Lines this short are a heading or a fragment; this wide, nothing was
# wrapping to a margin.
NARROW, WIDE = 40, 76


def body_lines(text):
    """Every line outside a fenced block, with its 1-based number."""
    fenced = False
    for number, line in enumerate(text.split("\n"), start=1):
        if FENCE.match(line):
            fenced = not fenced
            continue
        if not fenced:
            yield number, line


def hand_wrapped(text):
    """Three or more body lines in a row, none reaching the margin.

    That is what a hand-wrapped paragraph looks like, and what a
    paragraph written as one line never does. Lists, quotes, headings
    and table rows carry their own line breaks and are not counted.
    """
    run = 0
    for number, line in body_lines(text):
        stripped = line.strip()
        listish = stripped.startswith(("-", "*", ">", "#", "|")) or re.match(
            r"^\d+\.", stripped
        )
        if NARROW <= len(line) <= WIDE and not listish:
            run += 1
            if run >= RUN:
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
    # `Write` carries the whole file. An `Edit` carries a fragment, and a
    # fragment of a paragraph is not enough to tell wrapping from a
    # short paragraph, so only whole files are judged.
    content = tool_input.get("content")
    if not path or not content:
        return 0

    root = event.get("cwd") or os.getcwd()
    try:
        relative = os.path.relpath(path, root)
    except ValueError:
        return 0
    if not RENDERED.match(relative):
        return 0

    number = hand_wrapped(content)
    if number is None:
        return 0

    print(
        f"{relative} is a body GitHub renders, and the renderer folds"
        f" paragraphs itself. Line {number} sits in a run of hand-wrapped"
        f" lines: write each paragraph as one line and let it fold. Lists,"
        f" tables and fenced blocks keep their own line breaks.",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    sys.exit(main())
