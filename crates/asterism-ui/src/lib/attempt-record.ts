// What a transfer's attempt record says, read for a screen.
//
// `DispatchDto.attempt_json` is the exporter's own record of the call
// it made, written under the exporter's grammar and handed back by the
// core without being looked into. The transfer adapter writes one row
// per file — what it was called on the far side, how many bytes went,
// and what the host answered — plus a row for the sidecar beside them.
// Nothing rendered it before this.
//
// # Why a module rather than a function in the drawer
//
// Two reasons. The rows are the same rows for every dispatch rather
// than only for a send, so nothing about them belongs to the screen
// that happens to read them first. And a parser is the half worth
// testing without a DOM: what it has to survive is a record written by
// a build that is not this one.
//
// # Every field is optional, and that is the contract
//
// The payload's shape belongs to the exporter. A record from an older
// build, from another adapter, or from a run that was refused before it
// reached the files all arrive here, and none of them is a defect to
// throw on — so every read is defensive and an unreadable record is
// `null`, which a screen shows as "nothing recorded" rather than as an
// error about JSON.
//
// # A partial run is labelled by its failures
//
// `put` and `refused` are counted separately and neither is rounded
// into the other: the whole point of a per-file record is that "9 put ·
// 1 refused" is not "done". `refusedBefore` is the other shape the
// adapter writes — a refusal that happened before any file moved, where
// the reason is one sentence about the call rather than a row per file.

/// One file, as the record holds it.
export interface AttemptFile {
  /// What it was called on the far side.
  name: string;
  /// Where the copy was on this machine.
  source: string;
  /// `"sent"` or `"failed"`, as the adapter words it.
  outcome: string;
  /// How many bytes went, when any did.
  bytes: number | null;
  /// What the host said, when it refused.
  answer: string | null;
}

/// The sidecar that goes beside the files.
export interface AttemptSidecar {
  name: string;
  rows: number | null;
  outcome: string;
  answer: string | null;
}

/// One transfer call, read.
export interface AttemptRecord {
  /// Where the bytes were going, as the profile spelled it.
  endpoint: string | null;
  /// The scheme that carried them.
  scheme: string | null;
  /// The directory on the far side.
  directory: string | null;
  /// One row per file the run tried to put.
  files: AttemptFile[];
  /// The sidecar's own row, when the run got that far.
  sidecar: AttemptSidecar | null;
  /// Why nothing was sent, when the call was refused before any file
  /// moved. Null when the run reached the files, whatever became of
  /// them.
  refusedBefore: string | null;
  /// How many files the host took.
  put: number;
  /// How many it refused.
  refused: number;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function count(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readFile(row: unknown): AttemptFile | null {
  if (typeof row !== "object" || row === null) return null;
  const cell = row as Record<string, unknown>;
  const outcome = text(cell.outcome);
  if (outcome === null) return null;
  return {
    name: text(cell.name) ?? "",
    source: text(cell.source) ?? "",
    outcome,
    bytes: count(cell.bytes),
    answer: text(cell.answer),
  };
}

function readSidecar(value: unknown): AttemptSidecar | null {
  if (typeof value !== "object" || value === null) return null;
  const cell = value as Record<string, unknown>;
  const outcome = text(cell.outcome);
  if (outcome === null) return null;
  return {
    name: text(cell.name) ?? "",
    rows: count(cell.rows),
    outcome,
    answer: text(cell.answer),
  };
}

/**
 * Reads a dispatch's attempt record, or `null` when there is nothing
 * readable to show.
 *
 * `null` covers all of: no record yet (the run has not reported), a
 * record that is not JSON, and one that is JSON but not an object.
 * They are one answer on screen — there is nothing to render — and
 * telling them apart would be telling somebody about this build's
 * parser rather than about their send.
 */
export function readAttempt(attemptJson: string | null): AttemptRecord | null {
  if (attemptJson === null || attemptJson.trim() === "") return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(attemptJson);
  } catch {
    return null;
  }
  // An array is an object to `typeof` and is not a record. Spelled out
  // because the first version of this checked only `typeof`, and a
  // record that arrived as a list came back as a summary of nothing
  // rather than as "nothing recorded".
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return null;
  }
  const payload = parsed as Record<string, unknown>;
  const rows = Array.isArray(payload.files) ? payload.files : [];
  const files = rows
    .map(readFile)
    .filter((row): row is AttemptFile => row !== null);
  return {
    endpoint: text(payload.endpoint),
    scheme: text(payload.scheme),
    directory: text(payload.directory),
    files,
    sidecar: readSidecar(payload.sidecar),
    refusedBefore: text(payload.refused),
    put: files.filter((row) => row.outcome === "sent").length,
    refused: files.filter((row) => row.outcome !== "sent").length,
  };
}

/**
 * How a send's outcome reads in one line.
 *
 * A partial run is named by its failures rather than rounded to done,
 * which is the convention every file-transfer tool that got this right
 * follows and the one that reports a single modal for the batch got
 * wrong.
 */
export function attemptSummary(record: AttemptRecord | null): string {
  if (record === null) return "nothing recorded";
  if (record.refusedBefore !== null) return `refused · ${record.refusedBefore}`;
  if (record.files.length === 0) return "no file rows";
  if (record.refused === 0) {
    return `${record.put} put`;
  }
  return `${record.put} put · ${record.refused} refused`;
}
