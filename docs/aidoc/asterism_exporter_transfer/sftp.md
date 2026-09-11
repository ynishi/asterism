# asterism-exporter-transfer::sftp

`sftp://` — SSH's file transfer subsystem.

Two crates carry it: `russh` speaks the transport and the
authentication, `russh-sftp` is the subsystem on one of its channels.

# The host is checked before anything else happens

`russh` asks its handler about the host's key while the connection is
being established, before a credential is sent and long before a
file is. [`HostCheck`] answers that question and nothing else: it
compares the fingerprint the profile named, or looks the host up in
the `known_hosts` file the profile named, and records what was
offered so the refusal can say both halves.

Why the answer is recorded rather than returned as an error: the
handler's error type has to be one `russh` can build from its own,
and a mismatch is this adapter's word rather than the library's. So
the handler says no, the connection fails, and the reason is read
back out of the slot the handler wrote it into. Without that, every
refusal would arrive as `russh`'s generic rejection and a reader
could not tell a wrong key from a closed port.

## Functions

- `open` — Opens an SFTP session on the host the endpoint names.

