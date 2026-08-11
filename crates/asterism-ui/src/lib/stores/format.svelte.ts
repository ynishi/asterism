// Format catalog — sidebar-facing state for the FORMAT facet
// (asset-model v4, design-v4-material-asset-card.md).
//
// Format is a *fact of the material* (the primary material's mime
// top-level type: image / video / audio / text …), not a user
// classification — so unlike the Modality master there is no CRUD, no
// hidden flag, no ordering table: the facet is exactly what
// `list_format_asset_counts` reports for the current persona / trash
// scope, nothing more.
//
// Reload wiring follows the catalog rule: App-side `$effect`s own
// the reload chain; this catalog
// never decides when to reload itself.

import type { AssetCountEntryDto } from "../../bindings";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

// Display labels for the well-known mime top-level types. An
// unexpected type (e.g. `application`) falls back to the raw token.
const FORMAT_LABELS: Record<string, string> = {
  image: "Image",
  video: "Video",
  audio: "Audio",
  text: "Text",
};

class FormatCatalog {
  // `trash` follows the grid — same tuple contract as
  // `modalityCatalog.counts`.
  counts = new Resource(
    ([personaId, trash]: [string | null, string]) =>
      api<AssetCountEntryDto[]>("list_format_asset_counts", { personaId, trash }),
    [] as AssetCountEntryDto[],
    "formatCatalog.counts",
  );

  // Deliberately no `totalCount` (same reasoning as modalityCatalog):
  // a row whose material carries no mime has no format bucket yet is
  // still in the grid, so the section's own sum is not the grid size.
  // The "● all" row reads `personaCatalog.scopedTotal`.

  async loadCounts(
    personaId: string | null,
    trash: "live" | "trashed" = "live",
  ): Promise<void> {
    await this.counts.load([personaId, trash]);
  }

  /** Human label for a format token. */
  labelOf(format: string): string {
    return FORMAT_LABELS[format] ?? format;
  }
}

export const formatCatalog = new FormatCatalog();
