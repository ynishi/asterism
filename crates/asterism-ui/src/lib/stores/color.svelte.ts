// Color catalog — sidebar-facing state for the COLOR facet.
//
// Colour is a *derived fact of the image* (the dominant-colour palette
// quantised into a closed swatch set, `asterism_core::domain::color`),
// not a user classification — so like FORMAT and unlike the Modality
// master there is no CRUD, no hidden flag, no ordering table: the facet
// is exactly what `list_color_asset_counts` reports for the current
// persona / trash scope.
//
// The backend returns swatches in wheel order and omits the ones
// nothing carries, so this store never sorts and never fills gaps: a
// colour missing from the list means the corpus has none of it.
//
// Reload wiring follows the catalog rule: App-side `$effect`s own
// the reload chain; this catalog
// never decides when to reload itself.

import type { AssetCountEntryDto } from "../../bindings";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

// Display labels + the ink each swatch is drawn in. The hex here is a
// *representative* of the bucket, not a value any asset holds — the
// bucket covers a band of hues, and this is the middle of it.
const COLOR_SWATCHES: Record<string, { label: string; hex: string }> = {
  red: { label: "Red", hex: "#e03131" },
  orange: { label: "Orange", hex: "#f08c00" },
  yellow: { label: "Yellow", hex: "#f2d600" },
  green: { label: "Green", hex: "#37b24d" },
  cyan: { label: "Cyan", hex: "#22b8cf" },
  blue: { label: "Blue", hex: "#1c7ed6" },
  purple: { label: "Purple", hex: "#7048e8" },
  pink: { label: "Pink", hex: "#e64980" },
  brown: { label: "Brown", hex: "#8b5e34" },
  white: { label: "White", hex: "#f8f9fa" },
  gray: { label: "Gray", hex: "#868e96" },
  black: { label: "Black", hex: "#212529" },
};

class ColorCatalog {
  // `trash` follows the grid — same tuple contract as
  // `formatCatalog.counts`.
  counts = new Resource(
    ([personaId, trash]: [string | null, string]) =>
      api<AssetCountEntryDto[]>("list_color_asset_counts", { personaId, trash }),
    [] as AssetCountEntryDto[],
    "colorCatalog.counts",
  );

  // Deliberately no `totalCount`: an asset carries up to five swatches,
  // so summing the counts would count most photographs several times.
  // The "all" row shows no number rather than a wrong one.

  async loadCounts(
    personaId: string | null,
    trash: "live" | "trashed" = "live",
  ): Promise<void> {
    await this.counts.load([personaId, trash]);
  }

  /** Human label for a swatch slug. */
  labelOf(bucket: string): string {
    return COLOR_SWATCHES[bucket]?.label ?? bucket;
  }

  /** Ink for a swatch slug. Unknown slugs render transparent so an
   *  unexpected value is visibly blank instead of silently coloured. */
  hexOf(bucket: string): string {
    return COLOR_SWATCHES[bucket]?.hex ?? "transparent";
  }
}

export const colorCatalog = new ColorCatalog();
