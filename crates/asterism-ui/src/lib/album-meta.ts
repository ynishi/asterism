// AlbumMeta — reading the statements somebody made about an asset out
// of the bag they ride in.
//
// The server keeps them at `extra._trace.meta.<key>`, one slot per name
// (`asterism_core::domain::album_meta`). Everything here is a pure read
// over an already-fetched asset: the panel does not fetch statements,
// because they arrive with the detail.
//
// Defensive by construction. The bag is a JSON column that four other
// writers share, and this runs on every detail render — an entry in a
// shape this does not expect must produce one missing row, never a
// panel that fails to draw.

/// One recorded statement, flattened for display.
export type AlbumMetaStatement = {
  /// The name it was filed under.
  key: string;
  /// What was said.
  value: string;
  /// Channel it arrived on: `pushed` (with the ingest payload),
  /// `embedded` (dug out of the artefact), `manual` (declared after the
  /// fact). Absent when the entry does not carry one.
  source: string | null;
  /// Agent it came through, when one was stated.
  operator: string | null;
  /// When it was recorded.
  declaredAtMs: number | null;
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

/// Reads every statement out of a parsed `extra` bag, ordered by name.
///
/// Sorted rather than left in insertion order: the object comes back
/// from SQLite as it was serialised, so two renders of the same row
/// would otherwise be free to disagree about the order. A stable order
/// is also what makes "did this list change" answerable by eye.
///
/// An entry with no `value` is dropped. The server does not write one —
/// a statement is the value — so an entry without it is the shape of a
/// bag somebody edited by hand, and rendering an empty row would put a
/// name on screen that nothing is being said under.
export function readAlbumMeta(
  extra: Record<string, unknown>,
): AlbumMetaStatement[] {
  const trace = asRecord(extra["_trace"]);
  const meta = trace ? asRecord(trace["meta"]) : null;
  if (!meta) return [];

  const out: AlbumMetaStatement[] = [];
  for (const [key, raw] of Object.entries(meta)) {
    const entry = asRecord(raw);
    if (!entry) continue;
    const value = asString(entry["value"]);
    if (value === null) continue;
    const declaredAt = entry["declared_at_ms"];
    out.push({
      key,
      value,
      source: asString(entry["source"]),
      operator: asString(entry["operator"]),
      declaredAtMs: typeof declaredAt === "number" ? declaredAt : null,
    });
  }
  out.sort((a, b) => a.key.localeCompare(b.key));
  return out;
}

/// The shape the server accepts for a name, checked here so the panel
/// can say what is wrong before spending a round trip on it.
///
/// Deliberately the same rule as `album_meta::parse_key`, and the
/// duplication is the point of the message: the server refuses these
/// too, so this is an earlier answer to the same question rather than a
/// second policy. Returns `null` when the key is fine.
export function albumMetaKeyProblem(raw: string): string | null {
  const key = raw.trim();
  if (key.length === 0) return "a name is required";
  if (key.length > 64) return "a name is at most 64 characters";
  if (!/^[a-z0-9_-]+$/.test(key)) {
    return "lowercase letters, digits, _ and - only";
  }
  return null;
}
