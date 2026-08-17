# Asterism — pointers for coding agents

- Architecture and crate layout: [README.md](../README.md)
- Disclosure policy — read before anything ships: [PUBLIC_DEVELOPMENT.md](../PUBLIC_DEVELOPMENT.md)
- Issue conventions: [CONTRIBUTING.md](../CONTRIBUTING.md#issue-conventions)
- Branch, commit, and PR conventions (preferred commit format included): [CONTRIBUTING.md](../CONTRIBUTING.md)
- Security reports: [SECURITY.md](../SECURITY.md)
- API detail lives in RustDoc (`cargo doc`) and `docs/aidoc/`; code documentation outranks stale issue text.
- Green is `just check`, and CI is where it runs. Do not run its two workspace-wide gates locally: `just rust-test` links every test binary in the workspace at once, and `just rust-clippy` compiles every target in every crate. Use `just rust-test-changed` and `just rust-clippy-changed` (the packages this branch touched — the pair `pre-push` runs), or `just rust-test-pkg <crate>…`. Never hand-roll `cargo test --workspace` or `cargo clippy --workspace`.
- Never work on `main`; one worktree per issue under `.worktrees/`, cut with `just worktree-new <type> <slug>` from the main checkout — the recipe runs `just branch-check` in the new worktree, so there is no second run to make.
- The `-changed` gates answer for the commits on the branch and refuse a dirty tree. Commit, then run them; while editing, reach for `just rust-test-pkg <crate>`.
- Agents do not push, publish, or open PRs. They do run `git fetch origin` then `just pre-push` themselves, write the PR body to a file under `workspace/`, and hand over the two literal commands — the ordering is in [CONTRIBUTING.md](../CONTRIBUTING.md#pull-requests) and putting `just pre-push` in the handed-over block is the mistake it exists to stop.
- Run the `reviewer` and `pub-checker` agents (`.claude/agents/`) before every commit — `pub-checker` on the diff, `reviewer` with the issue number, which is all it needs.
- Recommended, once per machine: `/plugin marketplace add ynishi/asterism` then `/plugin install prose-shape@asterism`. Its hook refuses a write that gives a file the wrong prose shape and says which line, at the moment of the write. What the shapes are is in the check and deliberately not repeated here — an agent that reads a width applies it to everything it writes next, which is the failure this replaces.
