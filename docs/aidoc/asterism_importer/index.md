# asterism-importer 0.0.0

Unified CLI for all built-in Asterism import adapters.

One subcommand per source — Claude Code sessions, character cards,
agent-harvest envelopes, arbitrary SQLite queries, persona-journal
entries, tapes, and image / video / audio files. Every subcommand
runs the same importer-SDK pipeline: walk the source, parse it into
typed footprints, and push them in batches to a running
`asterism-server` over HTTP (`--server`, default local). All imports
are persona-scoped (`--persona-id`) and support `--dry-run`, which
validates and reports without writing anything.

