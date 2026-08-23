# asterism-model-lab 0.0.0

# asterism-model-lab — provider-side model preparation (#112)

The preparation half of the model split. The app *uses* a model
package (`asterism-vision` reads it, digest-verified); this tool is
how a package comes to exist and how it earns its place: download
the towers from their official source, pin their digests into the
manifest, verify the package exactly the way the app will read it,
and qualify it against the fixture set. (A `registry` verb once
authored a distribution entry here; #132 retired that flow — the
encoder ships with the app, and what travels is the trained head.)

Deliberately a separate binary in the `asterism-import` category —
the actor is the provider, not the user, and nothing in the app's
dependency graph reaches back here. The dependency runs the other
way: this tool links `asterism-vision` so that `verify` and
`qualify` are the app's own reading of the package, not a second
implementation of it.

## Charter, and what v1 leaves out

`convert` (ONNX export / quantization for a model whose publisher
ships none) and `train` belong to this tool's charter and are not
implemented: the one supported model has official ONNX exports, and
#112 scopes training out. The recipe table below is where a model
that needs conversion would declare it.

