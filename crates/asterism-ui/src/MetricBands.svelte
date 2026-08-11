<script lang="ts">
  // MetricBands — the "Length / Size / Pixels" sidebar section.
  //
  // Three numeric bands over facts of the material: playback length
  // (`asset.duration_ms`), stored size (`asset.file_size_bytes`) and
  // resolution (`width_px * height_px`). All compose with every other
  // facet rather than replacing one, and none is format-specific —
  // audio carries a length the same way video does.
  //
  // State (0-prop, catalog-driven):
  //   - activeFilter.durationMinSec / .durationMaxSec
  //   - activeFilter.sizeMinMb / .sizeMaxMb
  //   - activeFilter.pixelsMinMp / .pixelsMaxMp
  //
  // Units: this section shows **seconds**, **MB** and **MP**, which is
  // what the store holds. The wire takes milliseconds, bytes and a raw
  // pixel count, and `activeFilter.metricBands()` is the only place that
  // conversion happens — this component never multiplies. Writing the
  // display number straight to the store is the whole of its job; the
  // App-side reload `$effect` picks the change up.
  //
  // The resolution row asks for **pixels**, not for a width or a "1080p"
  // preset, and the label says so. The stored dimensions are coded — the
  // byte stream's own, before any orientation is applied — so an upright
  // phone photo sits in the row as a landscape pair. Their product is
  // unchanged by that rotation, which makes it the one resolution
  // question the data can actually answer; a width band or a preset
  // would put every portrait capture in the wrong bucket.
  //
  // Two things about the semantics are worth knowing at this callsite:
  //
  //   - Naming either end of a band **excludes rows whose column is
  //     NULL**: a still image has no length, and a row whose bytes were
  //     never recorded has no size, so neither belongs anywhere inside a
  //     band (`ListAssetsQuery::duration_min_ms`). The hint under the
  //     Length inputs says so, because "why did my images vanish" is the
  //     first question this control raises.
  //   - `min > max` is a validation error on the wire, not an empty
  //     page. It is deliberately not pre-checked here: an empty grid
  //     reads as "nothing in the library is that long", which is a claim
  //     about the library rather than about the request. The error
  //     reaches the status line and the last good page stays up.
  //
  // Committed on `change` (blur / Enter), not on every keystroke: typing
  // `120` into an empty max passes through `1` and `12`, and a min of
  // `60` would make each of those an inverted band and a round trip that
  // could only fail.
  import { activeFilter } from "./lib/stores/filter.svelte";

  // "" (the cleared input) is an open end, not a zero. `valueAsNumber`
  // is `NaN` there, and a `NaN` band would serialise as `null` anyway —
  // being explicit keeps the store's `null` the only representation of
  // "this end is open".
  function read(e: Event): number | null {
    const el = e.currentTarget as HTMLInputElement;
    if (el.value.trim() === "") return null;
    const n = el.valueAsNumber;
    return Number.isFinite(n) ? n : null;
  }

  function clearAll() {
    activeFilter.durationMinSec = null;
    activeFilter.durationMaxSec = null;
    activeFilter.sizeMinMb = null;
    activeFilter.sizeMaxMb = null;
    activeFilter.pixelsMinMp = null;
    activeFilter.pixelsMaxMp = null;
  }
</script>

<h2>Length / Size / Pixels</h2>
<div class="bands">
  <div class="band">
    <span class="band-label">Length</span>
    <input
      type="number"
      min="0"
      step="1"
      inputmode="numeric"
      placeholder="min"
      aria-label="Minimum playback length in seconds"
      value={activeFilter.durationMinSec ?? ""}
      onchange={(e) => (activeFilter.durationMinSec = read(e))}
    />
    <span class="band-sep">–</span>
    <input
      type="number"
      min="0"
      step="1"
      inputmode="numeric"
      placeholder="max"
      aria-label="Maximum playback length in seconds"
      value={activeFilter.durationMaxSec ?? ""}
      onchange={(e) => (activeFilter.durationMaxSec = read(e))}
    />
    <span class="band-unit">s</span>
  </div>
  <div class="band">
    <span class="band-label">Size</span>
    <input
      type="number"
      min="0"
      step="1"
      inputmode="numeric"
      placeholder="min"
      aria-label="Minimum stored size in MB"
      value={activeFilter.sizeMinMb ?? ""}
      onchange={(e) => (activeFilter.sizeMinMb = read(e))}
    />
    <span class="band-sep">–</span>
    <input
      type="number"
      min="0"
      step="1"
      inputmode="numeric"
      placeholder="max"
      aria-label="Maximum stored size in MB"
      value={activeFilter.sizeMaxMb ?? ""}
      onchange={(e) => (activeFilter.sizeMaxMb = read(e))}
    />
    <span class="band-unit">MB</span>
  </div>
  <div class="band">
    <span class="band-label">Pixels</span>
    <input
      type="number"
      min="0"
      step="1"
      inputmode="numeric"
      placeholder="min"
      aria-label="Minimum total pixel count in megapixels"
      value={activeFilter.pixelsMinMp ?? ""}
      onchange={(e) => (activeFilter.pixelsMinMp = read(e))}
    />
    <span class="band-sep">–</span>
    <input
      type="number"
      min="0"
      step="1"
      inputmode="numeric"
      placeholder="max"
      aria-label="Maximum total pixel count in megapixels"
      value={activeFilter.pixelsMaxMp ?? ""}
      onchange={(e) => (activeFilter.pixelsMaxMp = read(e))}
    />
    <span class="band-unit">MP</span>
  </div>
  {#if activeFilter.hasMetricBand()}
    <p class="band-note">
      Rows with no recorded length / size / dimensions are out while a band is
      set.
    </p>
    <button class="band-clear" onclick={clearAll}>× clear</button>
  {/if}
</div>

<style>
  /* Mirrors the `.sidebar h2` cascade the other sections duplicate
     (FormatList / ModalityList). Kept in sync until the whole sidebar
     graduates out of App (wave 9). */
  h2 {
    font-size: 0.75rem;
    color: #888;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin: 1rem 0 0.25rem;
  }

  .bands {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .band {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.1rem 0.3rem;
  }

  .band-label {
    font-size: 0.8rem;
    color: #555;
    width: 3.2rem;
    flex: none;
  }

  .band input {
    width: 3.4rem;
    min-width: 0;
    font-family: inherit;
    font-size: 0.8rem;
    color: #333;
    padding: 0.1rem 0.25rem;
    border: 1px solid #ddd;
    border-radius: 4px;
    background: #fff;
    font-variant-numeric: tabular-nums;
  }
  .band input:focus {
    outline: none;
    border-color: #8a86ff;
  }

  .band-sep,
  .band-unit {
    font-size: 0.75rem;
    color: #999;
  }
  .band-unit {
    width: 1.6rem;
  }

  .band-note {
    margin: 0 0.3rem;
    font-size: 0.7rem;
    line-height: 1.3;
    color: #9c9a89;
  }

  .band-clear {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 0.1rem 0.3rem;
    font-family: inherit;
    font-size: 0.8rem;
    color: #555;
    cursor: pointer;
    border-radius: 4px;
  }
  .band-clear:hover {
    background: #efefe9;
  }
</style>
