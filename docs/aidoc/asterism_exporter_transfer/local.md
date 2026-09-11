# asterism-exporter-transfer::local

`file://` — a directory on this machine as a destination.

Why it exists is on [`Scheme::File`](crate::transport::Scheme::File):
everything above the transport trait needs a far side a test can read
back, and an SFTP server inside a test answers for the server rather
than for this adapter.

It is a destination a profile may name, not a stub. What it does is
what the others do — reach the directory, put the files, put the
sidecar — with the filesystem as the wire, so a send to it lands the
same bytes under the same names in the same order.

## Functions

- `open` — Opens the directory the endpoint names.

