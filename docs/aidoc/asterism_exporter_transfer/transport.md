# asterism-exporter-transfer::transport

The port between the exporter and the wire.

One trait with three verbs — reach the directory, put a file in it,
close — and one trait that opens a connection to reach it with.
Everything above this line is the same whichever protocol carries the
bytes: which files go, under what names, in what order, what the
sidecar says and what the attempt record ends up holding.

# Why the split is here and not at the exporter

A protocol needs a server to talk to, and an SFTP server in a unit
test is a second implementation of the thing under test. Behind this
trait the exporter's own decisions are exercised against a far side
held in memory, which is a map of what it was asked to put; the
protocol implementations answer for the protocol and nothing else.

# What a refusal is, and what a failure is

[`TransportError::Refused`] is the answer before any byte moved: a
scheme the profile did not opt into, a host whose key is not the one
named, a credential the server would not take.
[`TransportError::Failed`] is what the far end said about something
that was attempted. The exporter records both, and the difference is
what a reader needs: a refusal means nothing arrived, a failure means
some of it may have.

## Functions

- `read_endpoint` — Reads a profile's endpoint into a [`Target`].

## Types

- `Credentials` — What the far side is asked to accept as proof of who is calling.
- `HostKey` — How the far side's identity is checked.
- `Scheme` — The protocols this adapter speaks.
- `Target` — Where a send is going, as the profile's endpoint describes it.
- `TransportError` — What went wrong.

## Traits

- `Connector` — What opens one.
- `Transport` — An open connection to one destination.

