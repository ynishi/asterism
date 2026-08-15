# asterism-exporter-common 0.0.0

# asterism-exporter-common

What a *schema-driven* exporter needs in order to be configured
rather than written: a `{{...}}` substitution over the dispatch
([`template`]) and a path grammar for reading the backend's answer
([`jsonpath`]). `asterism-exporter-http` wants both, any adapter
configured the same way will, and a grammar with two spellings is
worse than either spelling on its own.

## Why not in the SDK

`asterism-dispatch-sdk` is the port. It publishes the `Exporter`
trait, the types that cross it, and the schema artifacts a backend
author reads, and an adapter that hard-codes its backend's protocol
implements that port without ever meeting a template. Shared
*implementation* between adapters is a different thing from the
contract adapters are written against, and it belongs one layer
out: this crate depends on the SDK, the SDK does not know this
crate exists.

## The traits, and what they are for

A concrete adapter does not reach for [`template::render`] and
[`jsonpath::many`] directly — it takes [`TemplateAdapter`] and
[`ResponsePath`] and receives [`CommonExportAdapter`] as the default
implementation of both. That is one line more at the definition
site, and it buys the thing the direct call cannot: the grammar
becomes substitutable per adapter without every adapter's call sites
changing shape.

That is not hypothetical. A profile that resolves its credential
from the environment rather than out of the params blob needs a
placeholder root for it, and the shared engine must not grow one:
everything it can reach comes out of the params blob, so a `secret`
root there would be a root that writes a credential down. The HTTP
adapter's `SecretGrammar` wraps [`CommonExportAdapter`] and
overrides [`TemplateAdapter::render`] to add exactly that root, and
the JSON-leaf and header traversals keep working because they are
default methods written in terms of `render`.

```no_run
use asterism_exporter_common::{CommonExportAdapter, ResponsePath, TemplateAdapter};

struct MyExporter<A = CommonExportAdapter> {
    grammar: A,
}

impl<A: TemplateAdapter + ResponsePath> MyExporter<A> {
    fn status(&self, response: &serde_json::Value) -> Option<String> {
        self.grammar
            .select_first(response, "$.status")
            .and_then(|v| self.grammar.display_string(v))
    }
}
```

## Modules

- [`jsonpath`](jsonpath.md): A JSONPath subset — enough to steer a state machine and pluck out
- [`template`](template.md): `{{...}}` substitution over a dispatch — the other half of what a

