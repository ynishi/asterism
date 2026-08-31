// The name a person reads off a card.
//
// Shared rather than local because the places that use it have to
// agree: the grid labels a visual card with it in clean mode when there
// is no cover text, the forge offers it as the default name for an
// entry added from a selection, and the detail pane offers it as the
// default name for an asset handed to a team. Somebody selecting three
// images and putting them on a line should see the names that were
// under them.
//
// Agreement is the point rather than reuse. What the forge does with
// the result is not what the grid does: the string is sent as an
// operation's name, lands on the line, and is what the model's
// uniqueness rule compares — two live entries under one name is a
// refused landing. So a second implementation here would not be a
// duplicated label, it would be a line whose entries are named one way
// and shown another. Several screens cut a locator down inline for a
// label, each in its own spelling; those are labels nothing compares.

/**
 * Basename without its extension, from a source locator.
 *
 * A leading dot is kept — `.env` is a name and not an extension — which
 * is what `dot > 0` says.
 */
export function baseName(locator: string | null | undefined): string {
  if (!locator) return "";
  const last = locator.split("/").pop() ?? locator;
  const dot = last.lastIndexOf(".");
  return dot > 0 ? last.slice(0, dot) : last;
}
