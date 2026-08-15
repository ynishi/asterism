# Asterism — pointers for coding agents

- Architecture and crate layout: [README.md](../README.md)
- Disclosure policy — read before anything ships: [PUBLIC_DEVELOPMENT.md](../PUBLIC_DEVELOPMENT.md)
- Branch, commit, and PR conventions (preferred commit format included): [CONTRIBUTING.md](../CONTRIBUTING.md)
- Security reports: [SECURITY.md](../SECURITY.md)
- API detail lives in RustDoc (`cargo doc`) and `docs/aidoc/`; code documentation outranks stale issue text.
- Green is `just check`; the full Rust suite runs only via `just rust-test` (never hand-rolled `cargo test --workspace`).
- Never work on `main`; one worktree per issue under `.worktrees/`, and run `just branch-check` before implementing.
- Agents do not push, publish, or open PRs — prepare the branch and hand over the literal commands.
- Run the `reviewer` and `pub-checker` agents (`.claude/agents/`) on the diff before every commit.
