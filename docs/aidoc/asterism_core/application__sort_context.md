# asterism-core::application::sort_context

Assembles the [`SortContext`] the sort evaluator needs, sourcing every
lookup from a repository.

Two callers, one implementation: the Query Group evaluator freezes the
resulting order into `asset_bucket.position`, and
[`AssetService::list`](crate::application::AssetService::list) answers a
caller that named an axis on the wire. They have to agree — a page that
comes back in a different order than the one a Query Group materialised
under the same spec would make the two features disagree about what
`Sort: Group` / `Order: ordered` means.

Lookups and their fidelity:

- **persona order / names** — [`PersonaRepository::list`] sorted by
  `display_order` (then id for stability): the authentic backend
  analogue of the UI sidebar order.
- **modality order** — [`AssetRepository::counts_by_modality`]. **Known
  drift**: this is corpus-frequency order, not the UI's hand-arranged
  sidebar order (which lives only in the browser's `localStorage`,
  `App.svelte` `MODALITIES`). It only affects the `modality` +
  `ordered` axis; every other axis is frequency-independent. A
  persisted backend modality order is the proper fix and is out of
  W1's scope.
- **group names** — [`GroupRepository::list`].

`persona` scopes the modality and group lookups. `None` means "every
persona", which is what an unscoped listing asks for: scoping those two
to one persona while the filter selects across all of them would rank
the axis by a corpus the page does not show.

## Functions

- `build_sort_context` — Builds the lookup context for [`sort_asset_ids`](crate::domain::sort_eval::sort_asset_ids).

