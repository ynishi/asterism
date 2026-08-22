# asterism-core::application::forge::thread_service

Saying something about work, and correcting it.

```text
  open      resolves an anchor, writes the thread and its first message
  say       writes a message
  amend     writes a correction
  rename    writes the thread's title

  get / anchored / about    read
```

Nothing here touches either log. A conversation about a pass does
not change what the pass said, and a remark on a change point does
not move the line — which is why this is a third face rather than
more verbs on the two that write records.

# An anchor is resolved, not accepted

[`Anchor`] is built from the thing itself rather than from its id,
so that a thread hanging off something nobody wrote is not a value
anybody can make. A caller with an id has not got the thing, so
this service reads it: [`Anchored`] names what to look for, and
every verb that takes one loads the pursuit or the line before a
thread exists.

That is the whole of what this service decides, and it decides it
by asking. The refusals are the model's — an entry a pass did not
touch is [`Anchor::entry`]'s refusal, not one written here.

# What this service is allowed to decide

Nothing else. It resolves, calls the model, and writes back what
came out.

## Types

- `Anchored` — What a caller says a thread hangs off, in ids.
- `ThreadService` — Conversation use-case service.

