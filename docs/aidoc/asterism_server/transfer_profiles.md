# asterism-server::transfer_profiles

The destination profiles a send can be aimed with, listed from disk.

A profile is one JSON file in the directory
[`release_dirs::profile_dir`](crate::release_dirs::profile_dir)
resolves, holding the transfer exporter's params minus the one key
the send writes. Files rather than rows, and a list rather than an
editor: the app reads this directory, validates what is in it and
lets somebody choose, and never writes to it. What an agency's
intake asks for moves on that agency's schedule, so the columns a
sidecar carries live in the file, and nothing in this tree knows any
of them.

# The parser is the sender's

[`asterism_exporter_transfer::read_profile`] is what answers for
each file, which is the same `serde_json::from_value` over the same
struct that `dispatch` runs, plus the refusals the send makes before
a connection opens. A list that blessed a profile the send then
refused would be the failure this listing exists to prevent, so the
two do not get separate opinions. That is also why this module sits
in `asterism-server`: it is the crate that already builds the
exporter registry, so it can name the transfer crate's parser
without the desktop crate taking a dependency on an adapter it does
not otherwise know about.

# A file that does not parse is listed, with its reason

Dropping it would leave somebody editing a file the app has stopped
mentioning, wondering why it never appears. It is listed, carrying
the parser's own sentence, and it cannot be picked.

# A directory that is not there is empty rather than broken

Nothing creates this directory: the first profile is written by
hand, and until then there is nothing to read. "No profiles" and "no
directory" are the same answer to the only question the screen asks,
and the directory travels with the list so the answer says where to
put one. A directory that exists and cannot be read is a different
matter and is reported as itself.

## Functions

- `list` — Every profile in `dir`, by name.
- `read_body` — One profile's text, by the name the listing gave it.

