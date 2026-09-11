// What a transfer's attempt record says, and what it must survive.
//
// The payload's shape belongs to the exporter and this reads it for a
// screen, so the thing worth pinning is not the happy path — it is that
// a record from a build that is not this one, or from a run refused
// before it reached the files, arrives as something renderable rather
// than as a thrown error inside a drawer.
//
// The fixtures here are the shape `asterism-exporter-transfer` writes,
// and `forge_send_e2e` asserts the same field names off a real run —
// which is what keeps these two honest about the same record.
import { describe, expect, it } from "vitest";
import { attemptSummary, readAttempt } from "./attempt-record";

/// A run that reached the files: two put, the sidecar beside them.
const SENT = JSON.stringify({
  endpoint: "file:///tmp/outbox",
  scheme: "file",
  directory: "/tmp/outbox",
  account: null,
  files: [
    { name: "a.png", source: "/rel/a.png", bytes: 7, outcome: "sent", answer: null },
    { name: "b.png", source: "/rel/b.png", bytes: 9, outcome: "sent", answer: null },
  ],
  sidecar: { name: "metadata.csv", rows: 2, outcome: "sent", answer: null },
});

/// The same run with one file refused by the host.
const PARTIAL = JSON.stringify({
  endpoint: "ftps://host.example/in",
  scheme: "ftps",
  directory: "/in",
  files: [
    { name: "a.png", source: "/rel/a.png", bytes: 7, outcome: "sent", answer: null },
    {
      name: "b.png",
      source: "/rel/b.png",
      bytes: null,
      outcome: "failed",
      answer: "550 quota exceeded",
    },
  ],
  sidecar: { name: "metadata.csv", rows: 2, outcome: "sent", answer: null },
});

/// Refused before a connection opened: no rows at all, one sentence.
const REFUSED = JSON.stringify({
  endpoint: "ftp://host.example/in",
  account: null,
  refused:
    "ftp:// sends the credential and the files in the clear; a profile that means it says allow_insecure: true",
  files: [],
});

describe("reading a transfer's attempt record", () => {
  it("counts what the host took and what it refused, separately", () => {
    const record = readAttempt(PARTIAL);
    expect(record).not.toBeNull();
    expect(record!.put).toBe(1);
    expect(record!.refused).toBe(1);
    expect(record!.files[1].answer).toBe("550 quota exceeded");
    expect(record!.scheme).toBe("ftps");
  });

  /// **A partial run is never rounded to done.** This is the whole
  /// reason the record is per file: one modal for the batch is the
  /// shape that loses which file the reader has to go and look at.
  it("labels a partial run by its refusals and a clean one by its count", () => {
    expect(attemptSummary(readAttempt(PARTIAL))).toBe("1 put · 1 refused");
    expect(attemptSummary(readAttempt(SENT))).toBe("2 put");
  });

  /// A refusal before any file moved is the other shape the adapter
  /// writes, and it has no rows to count — so the summary carries the
  /// sentence instead of saying "0 put", which reads as a run that
  /// tried.
  it("carries a refusal that happened before any file moved", () => {
    const record = readAttempt(REFUSED);
    expect(record!.files).toEqual([]);
    expect(record!.refusedBefore).toContain("allow_insecure");
    expect(attemptSummary(record)).toContain("refused");
  });

  /// Everything that is not a readable record is one answer on screen,
  /// because there is nothing to render for any of them and telling
  /// them apart would be telling somebody about this build's parser.
  it("answers null for anything there is nothing to show for", () => {
    expect(readAttempt(null)).toBeNull();
    expect(readAttempt("")).toBeNull();
    expect(readAttempt("not json at all")).toBeNull();
    expect(readAttempt("[1, 2, 3]")).toBeNull();
    expect(readAttempt('"a string"')).toBeNull();
    expect(attemptSummary(null)).toBe("nothing recorded");
  });

  /// A record from a build that is not this one. Every field is
  /// optional on the way in, so a missing `sidecar`, a `files` that is
  /// not an array and a row with no `outcome` are dropped rather than
  /// thrown on — a drawer that threw here would show nothing at all
  /// about a send that did happen.
  it("survives a record written by another build", () => {
    const record = readAttempt(
      JSON.stringify({
        files: [
          { outcome: "sent" },
          { name: "no outcome" },
          "not an object",
          null,
        ],
        somethingNew: { nested: true },
      }),
    );
    expect(record).not.toBeNull();
    expect(record!.files).toHaveLength(1);
    expect(record!.files[0].name).toBe("");
    expect(record!.files[0].bytes).toBeNull();
    expect(record!.sidecar).toBeNull();
    expect(record!.endpoint).toBeNull();
    expect(attemptSummary(record)).toBe("1 put");

    const notAList = readAttempt(JSON.stringify({ files: "nope" }));
    expect(notAList!.files).toEqual([]);
    expect(attemptSummary(notAList)).toBe("no file rows");
  });
});
