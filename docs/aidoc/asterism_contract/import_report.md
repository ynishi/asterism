# asterism-contract::import_report

What an importer run reports back to whatever started it.

A machine-readable summary, written to a path the caller names, and
the reason it exists rather than being scraped off stderr: the
progress lines are for a person and are free to change, while this
is a contract between two binaries. A supervisor parsing "done —
ok=3 err=0" would break the day somebody improved the wording, and
would break silently, reporting zero.

It carries the **class** of whatever ended the run and not only its
message. The class is what a caller acts on — three slices of this
port went into drawing it — and it is the part a scheduler needs in
order to know whether running again is worth anything.

## Types

- `ImportReport` — One run's outcome, as the importer saw it.
- `ReportedFailure` — The failure that ended a run, flattened for the wire.

