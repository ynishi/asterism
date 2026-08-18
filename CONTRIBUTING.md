# Contributing

Shared conventions for changes to this repository, for humans and coding agents
alike. The disclosure policy in [PUBLIC_DEVELOPMENT.md](PUBLIC_DEVELOPMENT.md)
outranks this file.

## Issue conventions

### Labels

Four categories. Assign at least one when you open an issue.

| Label         | The change                                           |
| ------------- | ---------------------------------------------------- |
| `bug`         | behaviour contradicts what it promises               |
| `enhancement` | behaviour that does not exist yet                    |
| `refactor`    | behaviour unchanged                                  |
| `chore`       | production code untouched — CI, tests, tooling, docs |

## Branches

Never work on `main`. One worktree per issue, under the gitignored
`.worktrees/`:

```bash
just worktree-new <type> <slug>   # ci/, fix/, feat/, docs/
```

That is `git fetch origin`, then
`git worktree add .worktrees/<slug> -b <type>/<slug> origin/main`, then
`just branch-check` to verify the base before you build on it. What the recipe
adds to those three is a copy of `target/` into the new worktree, so its first
gate is not a rebuild of the whole dependency graph. Measured on
`asterism-infra`: `cargo check` took 1 min 17 s in a cold worktree against 39 s
in a copied one.

A copy, and deliberately not one shared target directory. Cargo treats path
dependencies with the same name, version and workspace-relative path as the same
crate even across checkouts
([cargo#12516](https://github.com/rust-lang/cargo/issues/12516)), which every
crate here satisfies against every other worktree — so two worktrees pointed at
one directory can report a gate green against the other branch's binaries, with
no error to notice. Copies collide with nothing. The copy is made only where it
is copy-on-write: APFS on macOS, where it costs about three seconds and no disk,
and on Linux btrfs, bcachefs or XFS made with `reflink=1`, where it is the same
operation with no timing taken yet. Not ext4, which is what most distributions
leave on `/`. The recipe asks the filesystem before copying rather than judging
by the result, because neither `cp` answers usefully afterwards: BSD's `-c` does
not fail where clonefile is unavailable — it falls back to a real byte copy "to
ensure the copy still succeeds" (`man cp`), which would cost more than the build
it is meant to save — and GNU's `--reflink=always` does fail, but once per file,
which over a full `target/` is tens of thousands of lines.

Where Linux has no clone the recipe hardlinks instead. Measured on ext4 against
a 111 GB target directory, that is seconds and about 10 GB where a byte copy of
the same tree is 6.3 minutes and 111 GB — and two of those copies do not fit
beside a checkout that already holds one. A hardlink shares the inode, so only
the part of the tree that cargo replaces rather than overwrites is shared: the
artifacts of a megabyte and up under `deps/`, which cargo names by a hash of
their inputs and swaps by unlinking its own copy first. Everything else is
copied, because everything else has a writer that opens the file that is already
there — dep-info, fingerprints, an `OUT_DIR` a re-run build script does not get
cleared, rustdoc's JSON, the lock file two checkouts would otherwise queue on.
`incremental/` is dropped because cargo regenerates it.

That copy is the slow half, so it runs in the background and the recipe returns
in about two seconds: the branch is cut and its sources are there, and nothing
reads `target/` until something compiles. It is staged under `workspace/`, which
is gitignored, so an unfinished copy never makes the tree dirty and never blocks
the `-changed` gates, and cargo cannot see a half-made tree either way.
`workspace/target-staging.log` says when it lands, and a build started before
then gets a cold `target/` of its own, which the staging leaves alone. Off Linux
and without a clone the recipe skips the copy and says so, and the worktree
starts cold, which is where it would have started regardless.

Run it from the main checkout — a worktree cannot cut another one, and the
recipe stops rather than nest one. Remove the worktree once the branch is
merged.

## Verification

`just check` is the definition of green, and CI is where it runs. Two of its
gates cost what the workspace costs rather than what the change costs:
`rust-test` links every test binary at once — one linker process each, gigabytes
resident each, as many at a time as `jobs` allows — and `rust-clippy` compiles
every target in every crate. That is minutes on any machine, and on a shared or
memory-tight one the test half is enough to push the box into swap and take down
whatever else is running there.

**Do not run either over the whole workspace locally as a matter of course.**
Reach for `just rust-test` by hand only when CI has reported something a narrow
run cannot reproduce, or when the change really is workspace-wide and the
machine has the room. Never hand-roll `cargo test --workspace`; `rust-test` is
the only sanctioned way to run it at all, for reasons its own comment gives.

The recipes to actually use:

- `just rust-test-changed` and `just rust-clippy-changed` — each works out which
  workspace members the commits on this branch touched, against `origin/main`,
  and runs only those. This pair is what `pre-push` runs.
  `just changed-packages` prints the list they share, if you want to see it. A
  dirty tree is refused rather than answered: an uncommitted edit belongs to no
  commit, so it maps to no member, and "no member changed" is not a thing to
  hear while a suite goes unrun. Commit, then ask.
- `just rust-test-pkg <crate>…` — the tests with the crates named by hand, for
  the loop while work is still moving. This is the one to reach for before the
  commit lands.

These are narrower than the workspace gates, not weaker: they cover what a
change edited, not what depends on it. `main` closes that gap on every merge.

**Opening a pull request does not wait on a full local run**, and it does not
wait on a full CI run either. A pull request runs `just check-changed`, which is
`check` with the same two substitutions `pre-push` makes — so the hosted answer
covers the crates the branch edited, and a branch that edits no crate links no
test binary. A change no single crate owns (the manifest, the lockfile, the
toolchain, `fixtures/`, `scripts/`) runs the full suite instead, since there is
nothing narrower to run and no later run to defer to. `main` runs `just check`
in full on every push, and that is where a dependent broken without being
touched surfaces: one merge later than if every pull request had linked the
whole workspace to look for it.

That backstop assumes the merge reaches `main` with a run of its own, which is
why the skip-keyword rule below matters more than it used to: a keyword in a
pull request title now costs the only place that regression would be caught, not
a duplicate verdict.

A change touching nothing but prose starts no run at all — the workflow's
`paths-ignore` names those files.

Report what was actually run; "I did not verify X" is a usable report, a green
claim resting on a recipe nobody ran is not. When a narrow run is what happened,
say which packages it covered.

## Preferred commit format

```text
<subject: what changed, one line>

<prose: the problem, why this fix and not the alternative, what it
cost. This is where the reasoning lives.>

Verified: <the recipes actually run, and their outcome>

Refs #<issue>
```

- No AI attribution of any kind — no `Co-Authored-By`, no "Generated with", no
  `Signed-off-by`.
- `cargo fmt` output and clippy fixes go in their own commits, separate from
  behaviour changes. `check-shared` running `rust-fmt-check` is a different
  question: it says the tree is formatted, not that the formatting belongs in
  the commit next to the behaviour it touched.
- Update `CHANGELOG.md` under `## [Unreleased]` as its own commit.
- Never commit `workspace/`, `.worktrees/`, or local agent state. If a commit
  needs `git add -f`, stop: something is filed wrong.
- A pull request title reaches `main`'s merge commit, and GitHub reads a CI skip
  keyword there the same way it reads one in a message: the run is skipped, its
  checks stay _pending_ rather than failing, and nothing looks wrong. Write "the
  skip keyword" instead, or name the mechanism. `commit-msg-check` covers the
  messages and carries the background — including why file contents are not
  affected; nothing can see a title.

## Working with coding agents — the recommended pattern

This repository ships its agent configuration in the open: pointer memory
(`.claude/CLAUDE.md`), guard agents (`.claude/agents/`), permission settings
that deny push/PR to agents outright, and a plugin worth installing:

```text
/plugin marketplace add ynishi/asterism
/plugin install prose-shape@asterism
```

`prose-shape` is a hook. It refuses a write that gives a file the wrong prose
shape — a paragraph hand-wrapped into a body GitHub renders and folds itself, a
line too wide for prose that is read in an editor and in `git diff` — and names
the file and the line as the write is attempted. Nothing about it is installed
by cloning; it is two commands you run once. If you develop here with a coding
agent, the loop that works is:

```text
issue -> just worktree-new -> implement -> just check
      -> reviewer agent on the diff -> commit
      -> git fetch origin -> just pre-push
      -> write the PR body to a file -> hand over push/PR
```

The `pre-push` at the end is the gate: it runs after the last commit, over the
tree that is actually handed over. It is not `check` — it adds `branch-check`,
which `check` never covers, and it substitutes the two `-changed` gates for the
workspace-wide clippy and test runs, so the gate before a hand-over costs what
the change costs rather than what the workspace costs. Run it there even when a
mid-loop run was green. A green run over a different tree is not a gate.

Two agents ship with the repo, and we would appreciate a diff passing both
before a pull request — recommended, not enforced:

- `pub-checker` — applies the disclosure policy to the diff.
- `reviewer` — checks the branch against the issue's acceptance criteria,
  redistribution, gates, and the commit message format. Give it the issue number
  and it takes the rest: the diff against `main`, and its own rounds from
  `workspace/review-<issue>.md`. It stops without an issue, and it reviews a
  branch twice — a branch too large for that is an issue to split.

The same recommendation extends past the PR: releases and publishes (crates,
packages, the repository's own settings) are best performed by a human hand in
agent-assisted interactive development, and whoever runs one — human or
automation — is encouraged to put what ships through `pub-checker` and
`reviewer` first.

## Pull requests

An agent's part ends with everything that does not write to anything remote, and
that includes the gate:

1. `git fetch origin`, then `just pre-push` — the agent runs both. `pre-push` is
   `branch-check` and `commit-msg-check` — the two that answer before anything
   compiles — then `check-shared`, `rust-clippy-changed` and
   `rust-test-changed`. None of them writes to anything remote, so being denied
   `git push` is no reason to skip it. The fetch comes first because they read
   `origin/main` offline: `branch-check`'s ancestry assertions, the range the
   message check walks, and the two narrow gates' idea of which packages the
   branch touched are only as fresh as the last fetch. Report the result,
   including any recipe that reported it did not check anything — `aidoc-guard`
   says so out loud and still exits 0.
2. **Write the PR body to a file** under `workspace/`, which is gitignored, and
   say where it is. Not a summary in the chat, not "a draft" — a file, so that
   the command handed over can be `--body-file <path>` and the human reads the
   same bytes that will be posted. The review record —
   `workspace/review-<issue>.md`, the rounds and what became of each finding —
   goes into that body. A finding declined there is a decision the pull request
   is making, so it travels with it rather than staying in a worktree that is
   about to be deleted.
3. Hand over the literal commands. These two are the only ones that write to
   anything remote:

   ```bash
   git push -u origin <branch>
   gh pr create --base main --head <branch> \
     --title "<subject>" --body-file <path>
   ```

Naming the artifact is the load-bearing part. Until 2026-08-15 this section said
that agents prepare "the branch, the commits, and the PR draft" and that "a
human runs `just pre-push`, pushes, and opens the PR". An agent followed it
exactly: it wrote no file, and handed over a command block containing
`just pre-push` — putting the gate on the human instead of running and reporting
it, and leaving the PR body to be invented at `gh pr create` time.

The PR body records what changed, what was verified, and what it deliberately
does not cover — under the same disclosure policy as everything else.
