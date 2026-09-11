# asterism-exporter-transfer 0.1.1

# asterism-exporter-transfer

One adapter for the one channel every stock agency sanctions for a
batch: the files on the agency's host over SFTP, FTPS or FTP, with a
CSV sidecar beside them. The scheme in the profile's endpoint chooses
which — one crate for all of them, the way
[`asterism_exporter_http`] is one crate for hosted and self-hosted
job APIs, because a host, a credential and a directory layout are the
whole of what differs. [`Scheme`] is the list.

## What it sends, and what it does not

**The bytes are a release's stamped copies, named by path.** They
reach this adapter in [`RESERVED_KEY`]`.files`, which the send writes
and a profile may not — `asterism_core::application::send_service` is
where that is argued and where the refusal lives. Nothing here reads
`input.source_locator`: the inputs are the library rows the copies
were made from, and they are what the sidecar's columns are rendered
against, not where the bytes come from.

A profile that names no file list is refused. There is no second
meaning for it — an adapter that fell back to the inputs would send
the library's own unstamped originals.

## Params schema

`CreateDispatchCommand.params_json` deserialises into
[`TransferDispatchParams`]:

```json
{
  "endpoint": "sftp://stock.example.com:22/incoming/2026-09",
  "auth": {
    "user":               "contributor",
    "secret_ref":         "AGENCY_SFTP_PASSWORD",
    "key_ref":            "AGENCY_SFTP_KEY",
    "key_passphrase_ref": "AGENCY_SFTP_KEY_PASSPHRASE"
  },
  "host_key": { "fingerprint": "SHA256:0000000000000000000000000000000000000000000" },
  "allow_insecure": false,
  "remote_name_template": "{{item.name}}",
  "sidecar": {
    "filename": "metadata.csv",
    "columns": [
      { "header": "Filename", "template": "{{item.remote_name}}" },
      { "header": "Title",    "template": "{{item.card.title?}}" }
    ]
  }
}
```

- `endpoint` — `<scheme>://<host>[:<port>][/<dir>]`. The scheme
  chooses the protocol, and the schemes are `sftp`, `ftps`, `ftp` and
  `file`; [`Scheme`] is where each says what it costs, and
  [`transport::read_endpoint`] has the grammar and the reason an
  endpoint carries no account.
- `auth` — the account, and the *names* of the environment variables
  the credential is read from. Absent means the server takes an
  anonymous login, which is the FTP shape and not much else.
- `host_key` — what the far side's key has to be. Required for
  `sftp://`. The other schemes authenticate their host through TLS or
  not at all, so a well-formed `host_key` beside one of them is
  ignored — but a malformed one is refused whichever scheme it sits
  with, because it is read before the scheme is consulted and naming
  neither or both of its two forms is a profile that has not decided.
- `allow_insecure` — permission to speak `ftp://`, where the
  credential and the bytes cross the network in the clear.
- `remote_name_template` — what each file is called on the far side.
  Absent means the copy's own basename.
- `sidecar` — the CSV that goes beside the files. Its columns are the
  agency's, and the tree carries no agency's column set: the
  disclosure keyword an agency reads (Freepik's `_ai_generated`) is a
  column somebody's profile chose, exactly like every other one.

`schema/transfer_params.example.json` is the runnable version of this
shape, and the tests at the bottom of this file are what keep it
honest.

### Templates

The `{{...}}` grammar is the shared one, documented where it is
defined: [`asterism_exporter_common::template`]. What this adapter
binds `{{item}}` to is [`FileRow::item`] — one file's row plus the
card of the input it came from — and it binds it in the same shape
for `remote_name_template` and for every sidecar column.

## Where a credential lives

In an environment variable, named by the profile and resolved per
call, for the reason `asterism_exporter_http`'s crate doc gives at
length: params are persisted unedited and handed back on every read
of the dispatch, so a value reachable by `{{params.…}}` is readable
by anything that can list dispatches. `key_ref` is the same rule one
step along — it names a variable holding the key's *location*, so
neither the key nor the path to it is on a row. The path is scrubbed
alongside the password and the passphrase rather than trusted to stay
out of a message: [`Credentials::secrets`] is that list.

## Two refusals that happen before anything is sent

**`ftp://` needs `allow_insecure`.** The default answer to a
credential over cleartext is no, and a profile that means it says so
in one field.

**An SFTP host is the host the profile names.** The fingerprint or
the `known_hosts` entry is checked as the connection opens, and a key
that does not match ends the dispatch with nothing put. There is no
prompt: a dispatch runs in a worker with nobody in front of it, and
"accept and remember" is a decision that would be made by the
absence of anyone to make it.

Both are recorded on the attempt before the error is returned, so a
reader of the dispatch sees which refusal it was rather than a
message alone — and so is every other answer given between reading
the params and the first put attempted, down to a blob that did not
parse. [`refuse`] is the one arm those leave through. On either side
of that span the shape is different and deliberately so: an action
this adapter does not take is the SDK's own variant and is answered
before anything is read, and a put that failed is one row among the
per-file ones below.

## The call is recorded per file

[`AttemptRecord`] carries what was sent, under what name, and what
the server answered for each file and for the sidecar — with the
credential redacted, and the environment variable *names* kept so a
reader can tell which profile was in play. A put that failed part way
through leaves a record of every file either way: the run failed with
the first error, and what actually landed is a question only the
record can answer.

The redaction is applied once per exit rather than per message,
wherever a record or an error leaves this crate, and it looks for
everything [`Credentials::secrets`] names —
[`Redaction`](asterism_exporter_common::Redaction) is what it is for.

## Lifecycle

The whole transfer happens inside [`dispatch`](TransferExporter::dispatch),
the way `asterism-exporter-file`'s writes do: `poll` answers
[`DispatchState::Done`] at once, and `harvest` returns no
[`Derived`](asterism_dispatch_sdk::Derived). Nothing was made. A copy
on somebody else's host is the same content that left, and minting an
asset for it would put a second row in the library for every file
sent.

## Modules

- [`ftp`](ftp.md): `ftps://` and `ftp://` — one client, two answers about TLS.
- [`local`](local.md): `file://` — a directory on this machine as a destination.
- [`sftp`](sftp.md): `sftp://` — SSH's file transfer subsystem.
- [`transport`](transport.md): The port between the exporter and the wire.

