# asterism-exporter-transfer::ftp

`ftps://` and `ftp://` — one client, two answers about TLS.

`suppaftp` fixes the TLS stream as a type parameter at construction,
so a plain connection cannot be upgraded later and the two are
different types all the way down. [`FtpWire`] is that pair, and every
verb below matches on it.

# What each scheme costs

`ftps://` negotiates TLS on the control connection before the
credential is sent and asks for the data connections to be protected
too, so the files are encrypted as well as the login. `ftp://`
protects neither, which is why nothing reaches this module under that
scheme unless the profile said so — the refusal is in
`crate::check_scheme`, where the message can name the field.

# Roots

The Mozilla set, compiled in. The alternative is the platform's own
store, which would make what a send trusts a property of the machine
it ran on; this workspace already made that choice for its HTTP
client, and this is the same choice rather than a second one.

## Functions

- `open` — Opens an FTP or FTPS session on the host the endpoint names.

