// "Show me this asset" — asked from anywhere, answered by the App.
//
// The detail pane's subject is `openAssetId` in `App.svelte`, and five
// gestures already funnel through the `openDetail(id)` beside it: a
// grid card, a provenance chip, a session drill-in, a constellation
// burst, a Quick Look escalation. All five are inside App, so setting
// the state directly was never a problem for them.
//
// The forge is not inside App. It is mounted there and reads its own
// catalog, taking nothing from whoever mounts it — the arrangement
// `SharedLinesPanel` has — and an entry on a line names an asset, so
// "look at this one properly" is a question it can ask and cannot
// answer.
//
// So this is a request rather than a second owner of the answer. App
// keeps `openAssetId` and keeps deciding what opening means (it closes
// a Quick Look first, pushes the interaction stack, records the event);
// this only carries the id to it. The alternative was to move that
// state out here, which would have meant every one of those five
// gestures changing to reach it — a refactor of the pane's bridge to
// serve a sixth caller.
//
// `assetPageCatalog.invalidations` is the same shape for the same
// reason: a signal a component consumes, rather than state two places
// write.

class DetailRequest {
  /// The asset somebody wants opened, until the App takes it.
  ///
  /// Null between requests, so asking for the same asset twice in a row
  /// is two requests rather than one no-op — the pane can be closed in
  /// between, and the second ask has to reopen it.
  asset = $state<string | null>(null);

  /// Asks for it.
  open(assetId: string): void {
    this.asset = assetId;
  }

  /// Takes it, leaving nothing. Called by whoever answers, before
  /// answering: an effect that reads this and opens a pane would
  /// otherwise re-open on its next pass.
  take(): string | null {
    const asset = this.asset;
    this.asset = null;
    return asset;
  }
}

export const detailRequest = new DetailRequest();
