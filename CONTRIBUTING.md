# Contributing

Shared conventions for changes to this repository, for humans and
coding agents alike. The disclosure policy in
[PUBLIC_DEVELOPMENT.md](PUBLIC_DEVELOPMENT.md) outranks this file.

## Branches

Never work on `main`. One worktree per issue, under the gitignored
`.worktrees/`:

```bash
git fetch origin
git worktree add .worktrees/<slug> -b <type>/<slug> origin/main   # ci/, fix/, feat/, docs/
just branch-check   # verifies the base before you build on it
```

Remove the worktree once the branch is merged.

## Verification

`just check` is the definition of green. The full Rust suite runs only
through `just rust-test` — never a hand-rolled `cargo test --workspace`.
Report what was actually run; "I did not verify X" is a usable report,
a green claim resting on a recipe nobody ran is not.

## Preferred commit format

```text
<subject: what changed, one line>

<prose: the problem, why this fix and not the alternative, what it
cost. Wrapped at 72. This is where the reasoning lives.>

Verified: <the recipes actually run, and their outcome>

Refs #<issue>
```

- No AI attribution of any kind — no `Co-Authored-By`, no "Generated
  with", no `Signed-off-by`.
- `cargo fmt` output and clippy fixes go in their own commits, separate
  from behaviour changes.
- Update `CHANGELOG.md` under `## [Unreleased]` as its own commit.
- Never commit `workspace/`, `.worktrees/`, or local agent state. If a
  commit needs `git add -f`, stop: something is filed wrong.

## Working with coding agents — the recommended pattern

This repository ships its agent configuration in the open: pointer
memory (`.claude/CLAUDE.md`), guard agents (`.claude/agents/`), and
permission settings that deny push/PR to agents outright. If you
develop here with a coding agent, the loop that works is:

```text
issue -> worktree (just branch-check) -> implement -> just check
      -> reviewer agent on the diff -> commit -> hand over push/PR
```

Two agents ship with the repo, and we would appreciate a diff passing
both before a pull request — recommended, not enforced:

- `pub-checker` — applies the disclosure policy to the diff.
- `reviewer` — checks the diff against the issue's acceptance criteria,
  redistribution, gates, and the commit message format.

The same recommendation extends past the PR: releases and publishes
(crates, packages, the repository's own settings) are best performed
by a human hand in agent-assisted interactive development, and whoever
runs one — human or automation — is encouraged to put what ships
through `pub-checker` and `reviewer` first.

## Pull requests

Agents prepare the branch, the commits, and the PR draft, then hand
over the literal commands; a human runs `just pre-push`, pushes, and
opens the PR. The PR body records what changed, what was verified, and
what it deliberately does not cover — under the same disclosure policy
as everything else.
