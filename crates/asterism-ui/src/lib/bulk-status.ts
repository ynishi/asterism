// The status line for an operation over a set that can partly fail.
//
// One function, and it owns every branch, because the alternative was
// tried twice and failed twice in opposite directions. First the caller
// supplied a `say(n)` that dropped the count when `n === 1` — correct
// standing alone ("moved to trash"), fatal inside the partial phrasing
// ("moved to trash of 5 — the rest was refused"). Then the caller
// supplied the zero phrasing as a constant, which lost the singular arm
// and produced "none of the 1 were moved to trash" for the commonest
// case there is: one card, refused, no selection.
//
// Both times the defect was the caller deciding something about number
// that only this function can see. So the caller passes the verb and
// nothing else about counting, and every string below is built here.
//
// The verb is a past participle: "moved", "restored", "deleted". What
// follows it is split in two, and the split is the third lesson: a
// phrase that reads correctly after "moved 3 of 5" does not necessarily
// read correctly after "it was not". "to trash" attaches to the verb
// and survives — "it was not moved to trash" says what happened. But
// "deleted forever" under negation becomes "it was not deleted
// forever", which English scopes over the adverb: *it was deleted, just
// not permanently*. About the one irreversible action in the app, in
// the direction that tells someone their asset is gone when it is still
// in the trash.
//
// So `into` is kept everywhere and `qualifier` is dropped where the
// sentence turns negative. The caller says which is which; it does not
// say where either one goes.

export interface BulkVerb {
  /** Past participle: `"moved"`, `"restored"`, `"deleted"`. */
  verb: string;
  /**
   * A prepositional phrase — `"to trash"`, `"to the group"`. Attaches
   * to the verb, so it is kept in every branch including the negative
   * ones.
   */
  into?: string;
  /**
   * An adverbial — `"forever"`. Dropped from the negative branches,
   * where it would read as qualifying the action rather than denying
   * it.
   */
  qualifier?: string;
}

/**
 * @param done  How many actually happened.
 * @param asked How many the gesture was for.
 *
 * `done` is never greater than `asked`, and `asked` is never zero:
 * every call site either returns early on an empty set or builds its
 * ids from a gesture, which always names at least one.
 *
 * @example
 * summariseBulk(1, 1, { verb: "moved", into: "to trash" })       // "moved to trash"
 * summariseBulk(5, 5, { verb: "moved", into: "to trash" })       // "moved 5 to trash"
 * summariseBulk(3, 5, { verb: "moved", into: "to trash" })       // "moved 3 of 5 to trash — the rest was refused"
 * summariseBulk(0, 5, { verb: "moved", into: "to trash" })       // "none of the 5 were moved to trash"
 * summariseBulk(0, 1, { verb: "moved", into: "to trash" })       // "it was not moved to trash"
 * summariseBulk(5, 5, { verb: "deleted", qualifier: "forever" }) // "deleted 5 forever"
 * summariseBulk(0, 5, { verb: "deleted", qualifier: "forever" }) // "none of the 5 were deleted"
 */
export function summariseBulk(
  done: number,
  asked: number,
  say: BulkVerb,
): string {
  const into = say.into ? ` ${say.into}` : "";
  const qualifier = say.qualifier ? ` ${say.qualifier}` : "";
  const tail = `${into}${qualifier}`;
  if (done === asked) {
    return asked === 1 ? `${say.verb}${tail}` : `${say.verb} ${asked}${tail}`;
  }
  if (done === 0) {
    // The count survives here because only the most recent refusal is on
    // screen: without it, five refused rows and one refused row read the
    // same. The qualifier does not — see the note above `BulkVerb`.
    return asked === 1
      ? `it was not ${say.verb}${into}`
      : `none of the ${asked} were ${say.verb}${into}`;
  }
  return `${say.verb} ${done} of ${asked}${tail} — the rest was refused`;
}
