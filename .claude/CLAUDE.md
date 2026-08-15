# Asterism — pointers for coding agents

- Architecture and crate layout: [README.md](../README.md)
- Disclosure policy — read before anything ships: [PUBLIC_DEVELOPMENT.md](../PUBLIC_DEVELOPMENT.md)
- Branch, commit, and PR conventions (preferred commit format included): [CONTRIBUTING.md](../CONTRIBUTING.md)
- Security reports: [SECURITY.md](../SECURITY.md)
- API detail lives in RustDoc (`cargo doc`) and `docs/aidoc/`; code documentation outranks stale issue text.
- Green is `just check`; the full Rust suite runs only via `just rust-test` (never hand-rolled `cargo test --workspace`).
- Never work on `main`; one worktree per issue under `.worktrees/`, and run `just branch-check` before implementing.
- Agents do not push, publish, or open PRs. They do run `git fetch origin` then `just pre-push` themselves, write the PR body to a file under `workspace/`, and hand over the two literal commands — the ordering is in [CONTRIBUTING.md](../CONTRIBUTING.md#pull-requests) and putting `just pre-push` in the handed-over block is the mistake it exists to stop.
- Run the `reviewer` and `pub-checker` agents (`.claude/agents/`) on the diff before every commit.
