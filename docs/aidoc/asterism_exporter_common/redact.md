# asterism-exporter-common::redact

Taking a credential back out of what an adapter wrote down.

An exporter records the call it made — the request as sent, the
backend's answer, the text of an error — and every one of those
reaches the dispatch row, which is handed back on every read of the
dispatch. A credential that travelled in any of them would be
readable by anything that can list dispatches, so the recorded copy
passes through here first.

# What a scrub can and cannot reach

It removes a value the adapter was *told* is a credential. A token a
profile built out of its own params and interpolated into a URL or a
body is not one of those: the adapter never learned it was a secret,
and no amount of searching here would find it. The way out of that is
for the profile to name the credential — which is what the adapters'
own `secret_ref`-style blocks are for, and what their docs say.

# Why an empty value is left alone

Replacing the empty substring would rewrite every string it is asked
about. A variable set to the empty string is a profile pointing at
nothing rather than a secret to hide, and an unset one never reaches
here — an adapter fails at resolution.

## Types

- `Redaction` — The credentials one call was made with, so they can be taken back out

## Constants

- `REDACTED` — What a redacted value is replaced by wherever an adapter writes down

