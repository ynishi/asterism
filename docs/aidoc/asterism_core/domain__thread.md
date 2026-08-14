# asterism-core::domain::thread

`Thread` — the app-level container that collects `Message`s from
both the UI (human) and the HTTP channel (Claude Code / agents).

A Thread is a lightweight anchor + a chronological list of
`Message`s. Author-agnostic: the same row receives human notes
and remote agent writes so the "Context → Action" transition stays
visually contiguous. The anchor tells *what* the Thread hangs off
of; `AppGlobal` is the Home-tab default, other variants attach the
Thread to a `Snapshot`, a query `Group`, or a `Card` (Phase 3+).

# Invariants

- `title` is non-empty (trimmed). Callers pick the label; the
  UI-suggested default for the AppGlobal case is `"Inbox"`.
- `Message::body` is non-empty (trimmed) — a blank submit is a
  caller bug, not a stored row.
- `Message::refs` may be empty. Duplicates and ordering are
  preserved verbatim (chips render in declared order).
- `Thread.message_count` and `Thread.last_message_at` are
  projections the adapter maintains from the `messages` table via
  a trigger — the domain entity carries them for read-side
  convenience but does not compute them.

# Design notes

- **Model unification**: UI and HTTP both hit
  `append_message`. Author identity is a wire-level detail
  (`author_kind` + `author_name`) that the server side accepts
  verbatim from a caller-supplied header; server never *infers*
  the author (spoof-prevention is a later concern; the field is
  just typed data for now).
- **Append-only Messages**: no `edit_message`
  verb. Misfires are corrected with `delete_message` and a fresh
  append.
- **`EntityRef` chips** are wired at the domain
  layer already so the Phase 4 UI does not need a schema
  migration — the `refs_json` column exists from day one.

## Types

- `Author` — Who wrote a `Message`.
- `EntityRef` — A reference chip embedded in a `Message` body (Phase 4 UI
- `Message` — One appended entry in a `Thread`.
- `Role` — Semantic role of a `Message`.
- `Thread` — Thread container.
- `ThreadAnchor` — What a `Thread` is attached to.

