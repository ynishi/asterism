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
- `just rust-test-one <crate> <cargo args>…` — one crate, and everything after
  it handed to `cargo test` verbatim, so a filter, `--lib`, or `--test <name>`
  all reach it. This is the one to reach for while work is still moving: a whole
  crate is not a small unit here, and `--lib` in particular skips the
  integration binaries that hold the link time.
- `just rust-test-pkg <crate>…` — whole crates, named by hand. Between the two
  above when the change is wide enough that naming tests stops helping.

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

## Documentation

API detail lives in the rustdoc. `cargo doc` is where a reader goes for what a
type is and what a function promises, and `docs/aidoc/` is generated from those
same comments — regenerate it with `just aidoc` after changing a public API or a
doc comment, and commit the diff. `aidoc-guard` fails on drift where it can run,
and says out loud that nothing was checked where it cannot: it needs a nightly
this workspace does not pin, so on most machines it exits 0 having looked at
nothing. When two texts disagree the code wins, and after that the doc comment:
a comment contradicting an issue is a finding about the issue.

Four rules for writing that prose. `doc-reviewer` asks after the fact whether a
passage is true, sited and singular; these are the same properties facing the
person writing it.

**State a rule once, where the thing it constrains is defined.** Everywhere
else, link to that statement and write only what this site adds — why it is an
instance of the rule, and what follows here. A second full statement is the copy
nobody edits the day the rule moves; `00c9ca4` is the audit that went looking
for them under one module, and its message sorts what it found by kind. Writing
a rule out where a reader meets it is what makes these files readable; writing
it out _twice_ is what goes stale.

**Do not write the state of the tree where nothing maintains it.** "Not yet",
"neither has one", "the only caller" are true when written and go false in
silence — a doc saying a port has no transport is refuted the moment somebody
routes it, and nothing tells the author. Keep the rule and cut the clause about
today.

**A number describing a list belongs in the file holding the list, or nowhere.**
The list is the answer and the number is a copy of it. Point at the list
instead.

**A sentence recording that a rule changed stays.** It is a constraint written
in the past tense, and it is what stands between the next reader and undoing the
rule. The test is whether deleting it lets somebody repeat the mistake. If
deleting it costs nothing it is a report on how a change went, and that belongs
in the commit message.

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

What this repository tells an agent is `AGENTS.md`, at the root, in the open and
under the name other coding agents already read. Claude Code reads `CLAUDE.md`
rather than that name, so `.claude/CLAUDE.md` is committed as a symlink back to
it — the instructions sit in one file, and a clone loads them with nothing to
set up.

That symlink is the only thing under `.claude/` this repository tracks.
`.gitignore` takes the rest, because the directory is Claude Code's working
directory on your machine: local settings, local memory, plugin caches, session
state. A diff that reaches anything else there is doing something unusual, and
the reviews say so. On Windows a clone without Developer Mode writes the symlink
out as a text file holding the path; leave it alone — editing it dirties a
tracked file and the `-changed` gates refuse a dirty tree — and put a
`CLAUDE.md` containing `@AGENTS.md` at the repository root instead, which loads
the same file by import and which `.gitignore` already keeps out of the tree.

The plugins are installed rather than cloned, and the block below is the same in
every checkout:

```text
/plugin marketplace add ynishi/asterism
/plugin install prose-shape@asterism
/plugin install review@asterism
```

A `permissions.deny` list is personal and is not committed here: it is advisory
in the checkout that has it and absent from every other, so what holds an agent
to anything is the remote's own settings. Keep yours for the reminder rather
than as the guard.

`prose-shape` is a hook, and it covers the one width nothing else can. A commit
message body is answered for by `commit-msg-check` and the markdown in the tree
by `just md-check`, both over committed files. A pull request or issue body is
never committed — this file asks for it as a file under the gitignored
`workspace/` — so the write is the only moment anything can look at one, and a
hard-wrapped paragraph is exactly what should not reach a renderer that folds.
The hook refuses that write and names the line. Nothing about it is installed by
cloning; it is two commands you run once. If you develop here with a coding
agent, the loop that works is:

```text
issue -> just worktree-new -> implement -> just check
      -> reviewer on the issue, pub-checker + doc-reviewer on the diff
      -> commit
      -> git fetch origin -> just pre-push
      -> write the PR body to a file -> hand over push/PR
```

Three reviews run there and they do not overlap. `reviewer` answers whether the
change does what its issue asked; `pub-checker` answers what may be published;
`doc-reviewer` answers whether the comments beside the code are still true,
which is the only one of the three that owns prose. They are one plugin because
they are one moment: nobody wants two of them and not the third. The prose
findings are advisory: a commit may land with all of them open, and only a claim
quoted beside the code that contradicts it is a fix. A sentence recording that a
rule changed is not a defect to any of them, for the reason
[Documentation](#documentation) gives.

That division has to be visible to the agent doing the work, not only true: the
run that prompted writing it down had `reviewer` producing wording notes for a
prose review that was not installed, and an agent that read the list as work to
do rewrote all nine of them — including ones whose whole content was the record
of what a rule replaced. The three arrive together, so a machine has all of them
or none: if `review` is not installed, the answer is to say the change was not
reviewed rather than to review it in their place.

The `pre-push` at the end is the gate: it runs after the last commit, over the
tree that is actually handed over. It is not `check` — it adds `branch-check`,
which `check` never covers, and it substitutes the two `-changed` gates for the
workspace-wide clippy and test runs, so the gate before a hand-over costs what
the change costs rather than what the workspace costs. Run it there even when a
mid-loop run was green. A green run over a different tree is not a gate.

Three agents come with the `review` plugin, and we would appreciate a diff
passing them before a pull request — recommended, not enforced:

- `pub-checker` — applies the disclosure policy to the diff.
- `doc-reviewer` — reads the prose the change touches, and the prose it makes
  false without touching, against the code beside it. Advisory, as above.
- `reviewer` — checks the branch against the issue's acceptance criteria,
  redistribution, gates, and the commit message format. Give it the issue number
  and it takes the rest: the diff against `main`, and its own rounds from
  `workspace/review-<issue>.md`. It stops without an issue, and it reviews a
  branch twice.

A defect the first round aimed at and the second finds wrong again in another
way says the design is what is wrong rather than the lines, so both agents open
with `DESIGN REVIEW REQUIRED`, name the one thing to settle, and leave it to a
human. Neither offers to move what is left into another issue: splitting is for
work that is long or whose blast radius is unknown, and a defect that could be
fixed today is neither.

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
