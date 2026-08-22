# asterism-core::domain::disclosure::generator_keys

Keys that only a generator writes, one per family.

Matched by presence rather than by value: what identifies a ComfyUI
export is *that it carries a `workflow`*, not what the workflow says.
A container that happens to carry a keyword of the same name from
somewhere else is a false positive this accepts — it costs a
`trainedAlgorithmicMedia` on a file that is not one, which is the
direction that would matter, so each entry is a keyword no general
tool writes rather than a plausible-sounding word.

Public because family membership is one fact with two readers: the
evidence rule here, and the parameter extraction registry
(`asterism-infra`), which routes a row to a family's judgement by
the same keys. A second list on that side would be a way for "what
counts as ComfyUI" to disagree with itself.

## Constants

- `A1111` — AUTOMATIC1111 and its forks write one text blob under this
- `COMFY` — ComfyUI writes both of these: the API-format graph it executed
- `COMFY_API_GRAPH` — The first of [`COMFY`] under its own name: the API-format graph

