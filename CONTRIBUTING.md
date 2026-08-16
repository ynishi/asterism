# Contributing

Shared conventions for changes to this repository, for humans and
coding agents alike. The disclosure policy in
[PUBLIC_DEVELOPMENT.md](PUBLIC_DEVELOPMENT.md) outranks this file.

## Issue conventions

### Labels

Four categories. Assign at least one when you open an issue.

| Label | The change |
|---|---|
| `bug` | behaviour contradicts what it promises |
| `enhancement` | behaviour that does not exist yet |
| `refactor` | behaviour unchanged |
| `chore` | production code untouched — CI, tests, tooling, docs |

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

`just check` is the definition of green, and CI is where it runs. Two
of its gates cost what the workspace costs rather than what the change
costs: `rust-test` links every test binary at once — one linker process
each, gigabytes resident each, as many at a time as `jobs` allows — and
`rust-clippy` compiles every target in every crate. That is minutes on
any machine, and on a shared or memory-tight one the test half is
enough to push the box into swap and take down whatever else is running
there.

**Do not run either over the whole workspace locally as a matter of
course.** Reach for `just rust-test` by hand only when CI has reported
something a narrow run cannot reproduce, or when the change really is
workspace-wide and the machine has the room. Never hand-roll
`cargo test --workspace`; `rust-test` is the only sanctioned way to run
it at all, for reasons its own comment gives.

The recipes to actually use:

- `just rust-test-changed` and `just rust-clippy-changed` — each works
  out which workspace members this branch touched, against
  `origin/main` plus anything uncommitted, and runs only those. This
  pair is what `pre-push` runs. `just changed-packages` prints the
  list they share, if you want to see it.
- `just rust-test-pkg <crate>…` — the tests with the crates named by
  hand, for the loop while work is still moving.

These are narrower than the workspace gates, not weaker: they cover
what a change edited, not what depends on it. CI closes that gap on
every push.

**Opening a pull request does not wait on a full local run.** CI runs
`just check` — the workspace suite included — on every push, so the
full result reaches the PR either way. Report what was actually run;
"I did not verify X" is a usable report, a green claim resting on a
recipe nobody ran is not. When a narrow run is what happened, say
which packages it covered.

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
- **Never write a CI skip keyword in a commit message or a pull request
  title.** GitHub reads `[skip ci]`, `[ci skip]`, `[no ci]`,
  `[skip actions]`, `[actions skip]` and a `skip-checks: true` trailer
  anywhere in the message, and it does not care that you were writing
  about them rather than asking for them. A `pull_request` run reads
  the branch's head commit, so a branch whose tip discusses one of
  these gets no CI at all; a pull request title reaches `main`'s merge
  commit, so the same is true after the merge. The failure is silent —
  a skipped workflow leaves its checks at *pending* rather than
  failing, so nothing turns red and nothing is missing from the list.
  Write "the skip keyword" or name the mechanism instead. This
  happened: pull request #53 landed three commits whose prose quoted
  one, and only the accident that none of them was a branch tip kept
  its CI running.

  File contents are not affected — `.github/workflows/check.yml`
  quotes the keyword freely, and this bullet does too. The rule is
  about commit messages and pull request titles, which is where GitHub
  looks.

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

The `pre-push` at the end is the gate: it runs after the last commit,
over the tree that is actually handed over. It is not `check` — it adds
`branch-check`, which `check` never covers, and it substitutes the two
`-changed` gates for the workspace-wide clippy and test runs, so the
gate before a hand-over costs what the change costs rather than what
the workspace costs. Run it there even when a mid-loop run was green. A
green run over a different tree is not a gate.

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
   `pre-push` is `branch-check` plus `check-shared` plus
   `rust-clippy-changed` and `rust-test-changed`, and none of them
   writes to anything remote, so being denied `git push` is no reason
   to skip it. The fetch comes first because they read `origin/main`
   offline: `branch-check`'s ancestry assertions and the two narrow
   gates' idea of which packages the branch touched are only as fresh
   as the last fetch.
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
