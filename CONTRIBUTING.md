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
      -> reviewer agent on the diff -> commit
      -> git fetch origin -> just pre-push
      -> write the PR body to a file -> hand over push/PR
```

The `just check` in the middle is what you iterate against while the
work is still moving. The `pre-push` at the end is the gate: it runs
after the last commit, over the tree that is actually handed over, and
it is not the same recipe — it adds `branch-check`, which `check` never
covers. Run it there even when the mid-loop run was green. A green run
over a different tree is not a gate.

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

An agent's part ends with everything that does not write to anything
remote, and that includes the gate:

1. `git fetch origin`, then `just pre-push` — the agent runs both.
   `pre-push` is `branch-check` plus `check`, and neither writes to
   anything remote, so being denied `git push` is no reason to skip it.
   The fetch comes first because `branch-check` is offline by design:
   its ancestry assertions are only as fresh as the last fetch.
   Report the result, including any recipe that reported it did not
   check anything — `aidoc-guard` says so out loud and still exits 0.
2. **Write the PR body to a file** under `workspace/`, which is
   gitignored, and say where it is. Not a summary in the chat, not "a
   draft" — a file, so that the command handed over can be
   `--body-file <path>` and the human reads the same bytes that will be
   posted.
3. Hand over the literal commands. These two are the only ones that
   write to anything remote:

   ```bash
   git push -u origin <branch>
   gh pr create --base main --head <branch> \
     --title "<subject>" --body-file <path>
   ```

Naming the artifact is the load-bearing part. Until 2026-08-15 this
section said that agents prepare "the branch, the commits, and the PR
draft" and that "a human runs `just pre-push`, pushes, and opens the
PR". An agent followed it exactly: it wrote no file, and handed over a
command block containing `just pre-push` — putting the gate on the
human instead of running and reporting it, and leaving the PR body to
be invented at `gh pr create` time.

The PR body records what changed, what was verified, and what it
deliberately does not cover — under the same disclosure policy as
everything else.
