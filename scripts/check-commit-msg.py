#!/usr/bin/env python3
"""Check commit messages: body width, and CI skip keywords.

The body wraps at 72 columns and the message names no keyword GitHub
reads as "skip this run".

72 is not a taste. `git log` indents the message four spaces, so on an
80-column terminal a 72-column body leaves four columns of margin on
each side, and `git format-patch` output survives a couple of levels of
quoting. The convention is Tim Pope's (2008); the kernel runs the same
arithmetic without the right margin and uses 75.

Python rather than a shell line, because the count has to be in
characters and the shell tools disagree about that. macOS `awk`'s
`length()` counts bytes whatever `LC_ALL` says: it reads an em dash as
three columns and calls a 71-column line 73. That is a wrong answer in
the direction that invents work, and it did invent some before this
file existed. `len()` over text decoded as UTF-8 has one meaning.

The subject line is exempt. CONTRIBUTING.md asks for "one line" and
sets no width for it.
"""

import argparse
import re
import subprocess
import sys

LIMIT = 72

# GitHub reads these anywhere in a commit message and skips the run. It
# does not care that the message was discussing them rather than asking
# for them, and the failure is silent: a skipped workflow leaves its
# checks pending rather than failing, so nothing turns red and nothing
# is missing from the list. This happened — pull request #53 landed
# three commits whose prose quoted one, and only the accident that none
# of them was a branch tip kept CI running.
#
# A `pull_request` run reads the branch's head commit, so a branch whose
# tip discusses one of these gets no CI at all; a pull request title
# reaches main's merge commit, so the same is true after the merge.
# Nothing here can see a title, so that half stays a human rule: write
# "the skip keyword", or name the mechanism.
#
# File contents are not affected, which is why this reads messages and
# not a diff. `.github/workflows/check.yml` quotes the keyword freely,
# CONTRIBUTING.md discusses it, and the pattern below has to spell the
# keywords out to match them at all. Commit messages and pull request
# titles are the two places GitHub looks.
CI_SKIP = re.compile(
    r"\[(?:skip ci|ci skip|no ci|skip actions|actions skip)\]"
    r"|^skip-checks:[ \t]*true[ \t]*$",
    re.IGNORECASE | re.MULTILINE,
)

# The one author a range is not allowed to fail on, because nobody it
# could report to is able to act. The aidoc job in
# `.github/workflows/check.yml` commits `Regenerate docs/aidoc
# [skip ci]` and pushes it, and it does that for pull requests from
# this repository and not only for `main` — so a branch that has been
# through CI once carries that commit, and `pre-push` would block on it
# with no move available. Rewriting a commit that is already pushed is
# not one.
#
# The keyword is doing its job there rather than by accident: it sits
# on the head commit of a push the job itself just made, which is the
# duplicate run it exists to suppress. Quoting one in prose is what
# this file is for, and the bot writes no prose.
#
# Matched on the address rather than the name because that is what the
# workflow sets and what `git log` will report. Anyone can claim it —
# this is a gate against a mistake, not against a person.
BOT = "41898282+github-actions[bot]@users.noreply.github.com"


def offenders(message):
    """Yield (line number, width, text) for body lines past the limit."""
    for number, line in enumerate(message.splitlines(), start=1):
        if number == 1:
            continue
        if len(line) > LIMIT:
            yield number, len(line), line


def check(label, message):
    """Report a message's problems. True when it has none."""
    clean = True
    for number, width, line in offenders(message):
        print(f"  {label} line {number} is {width} columns: {line}", file=sys.stderr)
        clean = False
    for hit in CI_SKIP.finditer(message):
        print(f"  {label} carries a CI skip keyword: {hit.group(0)}", file=sys.stderr)
        clean = False
    return clean


def git(*args):
    """Run git, or exit with git's own complaint rather than a stack."""
    try:
        result = subprocess.run(
            ["git", *args], capture_output=True, text=True, check=True
        )
    except subprocess.CalledProcessError as failure:
        # `capture_output` plus `check` would otherwise swallow the one
        # useful sentence and hand back a traceback. The reachable case
        # is an unresolvable `origin/main` — a shallow CI checkout, or a
        # clone that has never fetched — which is exactly what
        # `changed-packages` refuses over, in two lines, with the fix
        # named. The last gate in the same run should not do worse.
        sys.stderr.write(failure.stderr)
        raise SystemExit(
            f"git {' '.join(args)} failed, so there is nothing to check. "
            "If the range names origin/main, run 'git fetch origin' "
            "(in CI, check out with fetch-depth: 0)."
        )
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", nargs="*", help="commit message files to check")
    parser.add_argument(
        "--range",
        dest="revisions",
        help="also check every commit in a git range, e.g. origin/main..HEAD",
    )
    args = parser.parse_args()

    if not args.files and not args.revisions:
        parser.error("give a file, a --range, or both")

    # `--range HEAD` is accepted by `rev-list` and walks the whole
    # history, which is a slow way to report on commits the caller was
    # not asking about.
    if args.revisions and ".." not in args.revisions:
        parser.error(
            f"--range wants a range: '{args.revisions}' names one commit, "
            "and rev-list would walk everything behind it. "
            "Use e.g. origin/main..HEAD"
        )

    clean = True
    for path in args.files:
        with open(path, encoding="utf-8") as handle:
            clean &= check(path, handle.read())

    # `--no-merges`, for the same reason as `BOT`: a merge commit's
    # message is generated, not written. GitHub's is "Merge pull request
    # #N from …" and then the pull request title, and the author it
    # records is a person who typed neither. Over `origin/main~20..`
    # that flagged an 80-column line nobody could act on. A topic branch
    # rarely carries one, so this is mostly about the wider ranges the
    # recipe advertises.
    revisions = (
        git("rev-list", "--no-merges", args.revisions).split()
        if args.revisions
        else []
    )
    merges = (
        len(git("rev-list", "--merges", args.revisions).split())
        if args.revisions
        else 0
    )

    skipped = 0
    for revision in revisions:
        author, _, message = git(
            "log", "-1", "--format=%ae%n%B", revision
        ).partition("\n")
        if author.strip() == BOT:
            skipped += 1
            continue
        clean &= check(revision[:8], message)

    # Said out loud. An exemption nobody sees reads as coverage.
    if skipped or merges:
        parts = []
        if skipped:
            parts.append(f"{skipped} authored by the CI bot")
        if merges:
            parts.append(f"{merges} generated by a merge")
        print(f"not checked: {', '.join(parts)}")

    if not clean:
        print(
            f"commit messages wrap their body at {LIMIT} columns and name no "
            "CI skip keyword",
            file=sys.stderr,
        )
        return 1

    print(f"commit messages wrap at {LIMIT} columns, no CI skip keyword")
    return 0


if __name__ == "__main__":
    sys.exit(main())
