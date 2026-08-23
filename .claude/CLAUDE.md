# Asterism — pointers for coding agents

- Architecture and crate layout: [README.md](../README.md)
- Disclosure policy — read before anything ships:
  [PUBLIC_DEVELOPMENT.md](../PUBLIC_DEVELOPMENT.md)
- Issue conventions: [CONTRIBUTING.md](../CONTRIBUTING.md#issue-conventions)
- Branch, commit, and PR conventions (preferred commit format included):
  [CONTRIBUTING.md](../CONTRIBUTING.md)
- Security reports: [SECURITY.md](../SECURITY.md)
- API detail lives in RustDoc (`cargo doc`) and `docs/aidoc/`; code
  documentation outranks stale issue text.
- Green is `just check`, and CI is where it runs. Do not run its two
  workspace-wide gates locally: `just rust-test` links every test binary in the
  workspace at once, and `just rust-clippy` compiles every target in every
  crate. Use `just rust-test-changed` and `just rust-clippy-changed` (the
  packages this branch touched — the pair `pre-push` runs), or
  `just rust-test-pkg <crate>…`. Never hand-roll `cargo test --workspace` or
  `cargo clippy --workspace` — that includes reaching for either as a quick
  check of your own edit.
- Never work on `main`; one worktree per issue under `.worktrees/`, cut with
  `just worktree-new <type> <slug>` from the main checkout — the recipe runs
  `just branch-check` in the new worktree, so there is no second run to make.
- The `-changed` gates answer for the commits on the branch and refuse a dirty
  tree. Commit, then run them; while editing, reach for
  `just rust-test-one <crate> <cargo args>…`, which passes a filter, `--lib`, or
  `--test <name>` straight through. A whole crate is not a small unit here.
- Agents do not push, publish, or open PRs. They do run `git fetch origin` then
  `just pre-push` themselves, write the PR body to a file under `workspace/`,
  and hand over the two literal commands — the ordering is in
  [CONTRIBUTING.md](../CONTRIBUTING.md#pull-requests) and putting
  `just pre-push` in the handed-over block is the mistake it exists to stop.
- Run the `reviewer` and `pub-checker` agents (`.claude/agents/`) before every
  commit — `pub-checker` on the diff, `reviewer` with the issue number, which is
  all it needs.
- Prose against code is a third review, and it is the one that owns comments:
  `doc-reviewer`, from the `doc-review` plugin
  (`/plugin install doc-review@asterism`, same marketplace as `prose-shape`).
  Run it on the diff beside `pub-checker`. It is a plugin, so it may not be
  installed on this machine — then say that the prose was not reviewed, and do
  not let `reviewer` stand in for it. Neither of the other two decides what a
  comment should say, and a sentence recording that a rule changed is not a
  defect to any of them: it is what stops the next reader undoing the rule.
- Do not carry a wrapping width from one artefact to another; three differ, and
  each has its own check to say so. `commit-msg-check` answers for commit
  messages, `just md-fmt` wraps the markdown in the tree and `just md-check`
  fails when it is not wrapped, and the `prose-shape` plugin refuses a
  hard-wrapped pull request or issue body, which nothing over the tree can see
  because those are never committed. Install it once per machine:
  `/plugin marketplace add ynishi/asterism`, then
  `/plugin install prose-shape@asterism`.
