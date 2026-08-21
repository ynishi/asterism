# asterism-infra::probes::json

JSON's reading of the content axis — the first digest on that axis
that re-renders, and the parameter list that makes it a definition.

A JSON object is an unordered collection of name/value pairs
(RFC 8259), so two documents differing only in member order are one
document, and a digest that selects bytes cannot say so. The axis
doctrine ([`content_hash`](asterism_core::domain::content_hash))
prices the other route: a re-rendering digest must write its
canonical form out in full, because the rule for numbers and the
rule for duplicate keys are the parts that decide the answers.
These are the parameters, and the golden vectors below are the
parameters made checkable:

1. **A document is exactly one JSON text, UTF-8.** Anything else —
   invalid bytes, a syntax error, trailing content after the value —
   refuses to [`ContentRegion::EmptySpan`]. JSON has no signature
   apart from parsing whole, so every disagreement between the claim
   and the bytes lands there; `Unsupported` stays what the port's
   gate answers for formats this probe never claimed.
2. **Inter-token whitespace is dropped.** The canonical form is
   compact: no space after `,` or `:`.
3. **Object members are sorted by member name, decoded.** Names are
   compared as the UTF-8 byte sequences their tokens decode to, so
   `"b"` sorts as `b` — an order a reordering serialiser cannot
   disturb, which is the property the axis exists for.
4. **A duplicate member name, at any depth, refuses the document.**
   Decoded before comparing, so a name spelled by Unicode escape
   collides with its plain spelling. The
   spec forbids the duplicate and practice resolves it silently,
   each serialiser its own way — a digest that picked a winner would
   call two documents the same on the strength of the loser.
5. **Every scalar token is copied from the source verbatim** —
   numbers, strings (member names included), `true`/`false`/`null`.
   `1.50` stays `1.50`, `-0.0` stays `-0.0`, and `1` never becomes
   `1.0`'s equal. This is the parameter that separates the reading
   from RFC 8785, whose ECMAScript number rendering collides
   integers above 2^53 and erases `-0.0` and `1.0` against `0` and
   `1`. On a duplicate-detection axis the two error directions are
   not symmetric — a false positive is folded by resolution and
   destroys, a false negative costs a row — so the smaller claim
   wins. The price is paid in the same coin: two spellings of one
   name — plain, and by Unicode escape — decode identically and
   digest differently, a false negative accepted on the same
   grounds.
6. **Array order is kept.** An array is a sequence; reordering one
   changes the document.

The meta axis is not claimed. A JSON document has no container
metadata — no bytes riding alongside that are *about* the value
rather than part of it — so there is nothing for that axis to read,
the same shape JPEG had while its meta reading did not exist yet.

`.jsonl` is deliberately not this format: it is records in a
container, not one value per file, and it keeps `text/plain`
([`guess_mime`](asterism_core::domain::material::guess_mime)).

# Why the walk is not `serde_json::Value`

The workspace already sorts keys once — `series::canonical_value`,
for the series key — and it walks a parsed `Value`, so every scalar
is re-rendered on the way out and `1.50` comes back `1.5`. The right
trade for a key over what a document *refers to*, and exactly the
loss parameter 5 refuses for what a document *is*. So this walk
validates with the parser and then scans the validated text itself,
copying tokens instead of parsing them.

## Types

- `JsonProbe` — The probe. Stateless — the reading is a function of the bytes.

