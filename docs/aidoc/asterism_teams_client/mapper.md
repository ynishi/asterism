# asterism-teams-client::mapper

The one mapper — what may travel, and the declaration that decides
it (#148 decisions 13 and 14).

## Why there is exactly one

Decision 14 puts the local Asset and the projection body behind a
single mapper, so this module is the only place in the workspace
that knows both. Decision 13 then puts the *declaration* at that
same seam, because the body is fed from labels, groups, personas,
series, comments, marks and whatever the local model gains next —
and a declaration attached to any one of those would cover one
input rather than the set.

## An input nobody declared does not travel

That is the property, and it is worth being precise about how it is
held rather than merely intended.

**Nothing here serialises an `Asset`.** There is no
`serde_json::to_value(asset)` and no `#[derive(Serialize)]` on
anything local. [`projection_body`] builds its output by walking
[`DeclaredInput::ALL`] and asking each declaration for its value,
so a field added to `Asset` next year produces no key here: it is
not that it would be filtered out, it is that nothing would go
looking for it.

**A declaration cannot be half-added, and the one edit the
compiler will not force is worth naming.** Adding an input is four
edits in this file: the variant, [`DeclaredInput::key`],
[`DeclaredInput::take`], and [`DeclaredInput::ALL`]. The middle two
match exhaustively, so a variant without them does not compile.
`ALL` is the one that fails quietly — a variant left out of it
simply never travels — and quiet in that direction is the safe one:
the failure decision 13 names is *forgetting to untick something*,
and a design whose slip is "it stayed home" cannot fail that way.

**The test says so too.** `every_key_in_a_body_was_declared` builds
a subject with everything populated and asserts the body's key set
is the declared set plus the version tag, which catches a key
written by hand into the assembly rather than through a
declaration.

## What is declared today, and what was left out

[`DeclaredInput::ALL`] is the answer, and it is eight lines of code
away; what follows is why each is there and, more usefully, why the
near misses are not.

The near misses are left undeclared for one reason — decision 4's
argument that what can be re-derived stays home reads on a
description as well as on a thumbnail:

- **`cover`** is produced by the CoverGen job from a
  modality-specific template. The receiving side can generate its
  own, and one member's template output is not a description the
  team should be handed as though a person wrote it.
- **`register_note`** is the same shape: an annotation about tone
  that a job fills.
- **`labels`** and **`keywords`** are mixed provenance — some a
  person applied in the grid, some an importer wrote as a
  `journal_kind:` prefix, and the Asset does not carry which is
  which. An input whose provenance is not decidable is one that
  cannot be declared honestly, so it is not declared.

Any of those becomes a declaration the day the local model can say
who wrote it. That is a decision, taken here, in the open.

## Functions

- `projection_body` — Builds the body a projection travels in, or nothing when no
- `read_projection_body` — Reads a projection body — the half of decision 14 that says *the

## Types

- `DeclaredInput` — One input that has been declared shareable (#148 decision 13).
- `LocalSubject` — Everything the mapper is allowed to look at.
- `ProjectionView` — A projection body, as the mapper for its version understands it.
- `PromotedMark` — One mark a person wrote, as it travels.
- `ReadMark` — One mark as a body carried it.

## Constants

- `VERSION_KEY` — The key the body's own version rides under.

