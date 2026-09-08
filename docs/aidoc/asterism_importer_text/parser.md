# asterism-importer-text::parser

Text `RawItem` → `Footprint::Doc` parser.

The capability this crate is the door for already existed: hand
`asset_add` a path to a `.md` and `guess_mime` answers `text/plain`,
`MimeType::body_text` accepts that, and the words reach the body
cache and the full-text index. What was missing was a route that
files a written file as a *document* — the media routes take a file
whole and call it an image, a clip, a recording; the tape route
takes one whole and calls it a terminal transcript; nothing took one
and called it something somebody wrote.

It shares `.txt` with that tape route, and the overlap is not an
accident to resolve: a transcript and a note are different things
wearing one extension, and which one a directory holds is the
caller's to say by picking the subcommand.

What this reads out of the bytes is only what a card needs before
the body is read: a heading, an excerpt, and a word count. The body
itself is never sent — the server reads it off the locator, which is
also what keeps a document that changes on disk from having two
answers.

## Types

- `TextParser` — Turns one scanned document into `Footprint::Doc`.

