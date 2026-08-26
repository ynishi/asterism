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
- Never work on `main`; one worktree per issue under `.worktrees/`, cut with
  `just worktree-new <type> <slug>` from the main checkout — the recipe runs
  `just branch-check` in the new worktree, so there is no second run to make.
- The `-changed` gates answer for the commits on the branch and refuse a dirty
  tree. Commit, then run them; while editing, reach for
  `just rust-test-one <crate> <cargo args>…`, which passes a filter, `--lib`, or
  `--test <name>` straight through. A whole crate is not a small unit here.
- The three reviews are the `review` plugin, so a checkout is not how they reach
  this machine: `/plugin install review@asterism`, same marketplace as
  `prose-shape`. When it is not installed none of the three exist — then say the
  change was not reviewed, rather than reviewing it by hand in their place.
- Do not carry a wrapping width from one artefact to another; three differ, and
  each has its own check to say so. `commit-msg-check` answers for commit
  messages, `just md-fmt` wraps the markdown in the tree and `just md-check`
  fails when it is not wrapped, and the `prose-shape` plugin refuses a
  hard-wrapped pull request or issue body, which nothing over the tree can see
  because those are never committed. Install it once per machine:
  `/plugin marketplace add ynishi/asterism`, then
  `/plugin install prose-shape@asterism`.
