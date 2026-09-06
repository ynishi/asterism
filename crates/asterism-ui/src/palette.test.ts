// Enforcement for the three rules stated at the top of `app.css`.
// The rules and the reasoning live there, where the palette is
// defined; this file is only the half a machine can repeat, and it
// carries no rule of its own except where one is marked as such.
//
// It exists because nothing else here can see a colour. `svelte-check`
// has no opinion about them and no component test reads a computed
// style, so #240's first pass shipped five classes of defect that four
// review passes then found by hand — and one of them, a palette name
// shadowing a component's injection hook, was invisible even to the
// first version of this file.
//
// The claims are structural rather than aesthetic. Whether
// `--accent-fill` is the right violet is a judgement nothing here can
// hold; whether a rule puts unreadable ink on it is arithmetic, and
// that is the half worth automating.
//
// The sources arrive through Vite's raw glob rather than `node:fs`:
// this crate's tsconfig carries no node types, and the bundler already
// knows where these files are.
import { describe, expect, it } from "vitest";

const MODULES = import.meta.glob(["./*.svelte", "./app.css"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const SOURCES = new Map(
  Object.entries(MODULES).map(([path, text]) => [path.replace("./", ""), text]),
);
const COMPONENTS = [...SOURCES.keys()].filter((f) => f.endsWith(".svelte")).sort();
const read = (name: string) => SOURCES.get(name) ?? "";
const APP_CSS = read("app.css");

// The names a component owns rather than the palette — `app.css` rule
// 1. Adding one here is how a component declares a hook; the two
// assertions below then hold the palette off it, in both directions.
const NOT_OURS = new Set([
  // Set from markup, as data rather than as design.
  "--persona-wallpaper",
  "--divider-color",
  "--depth",
  // Injection hooks: a host may declare these to retheme a component,
  // and until one does, the fallback beside them is the live value.
  // `--accent` and `--danger` are the two the palette took by accident;
  // it now calls those `--accent-fill` and `--danger-fill`.
  "--accent",
  "--danger",
  "--asterism-surface",
  "--asterism-text",
  "--panel-bg",
  "--panel-fg",
  "--hairline",
]);

// Not a palette name: geometry that happens to live in the same block.
const NON_COLOUR = /^--drawer-/;

const COMMENT = /\/\*[\s\S]*?\*\//g;
const LITERAL = /#[0-9a-fA-F]{3,8}\b|rgba?\([^)]*\)|hsla?\([^)]*\)/g;

// Matches a name whether or not it carries a fallback. The first
// version of this required a `)` straight after the name, so it saw
// `var(--ink)` and never `var(--danger, …)` — which meant every hook in
// the tree was invisible to every claim in this file.
const VAR_REF = /var\(\s*(--[a-z-]+)\s*(?=[,)])/g;
const STATE_SRC =
  "(:hover|:focus-visible|:focus-within|:focus|:active|:checked|\\.active\\b|\\.selected\\b|\\.is-active\\b|\\[aria-selected=\"true\"\\]|\\[aria-pressed=\"true\"\\])";
// Two objects on purpose: a `g` regex carries `lastIndex` between
// calls, so a shared one used for both `.test()` and `.replace()`
// answers a different question every other time it is asked.
const HAS_STATE = new RegExp(STATE_SRC);
const STRIP_STATE = new RegExp(STATE_SRC, "g");
const STRIP_NOT = /:not\([^)]*\)/g;

/** Every `<style>` block in a component, with comments blanked out. */
function styleOf(source: string): string {
  const blocks: string[] = [];
  for (const m of source.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)) {
    blocks.push(m[1].replace(COMMENT, " "));
  }
  return blocks.join("\n");
}

type Rule = { selector: string; decls: Map<string, string> };

function rulesOf(style: string): Rule[] {
  const out: Rule[] = [];
  for (const m of style.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const decls = new Map<string, string>();
    for (const dec of m[2].split(";")) {
      const at = dec.indexOf(":");
      if (at === -1) continue;
      decls.set(dec.slice(0, at).trim(), dec.slice(at + 1).trim().replace(/\s+/g, " "));
    }
    if (decls.size > 0) out.push({ selector: m[1].trim().replace(/\s+/g, " "), decls });
  }
  return out;
}

const definedTokens = new Map<string, string>();
for (const m of APP_CSS.matchAll(/^\s*(--[a-z-]+)\s*:\s*([^;]+);/gm)) {
  definedTokens.set(m[1], m[2].trim());
}
const colourTokens = [...definedTokens.keys()].filter((t) => !NON_COLOUR.test(t));

describe("the palette is the only place a colour is named", () => {
  it("leaves no colour literal in any component style block", () => {
    const found: string[] = [];
    for (const file of COMPONENTS) {
      for (const m of styleOf(read(file)).matchAll(LITERAL)) found.push(`${file}: ${m[0]}`);
    }
    expect(found).toEqual([]);
  });

  it("references no name app.css does not define", () => {
    const missing = new Set<string>();
    for (const file of COMPONENTS) {
      for (const m of styleOf(read(file)).matchAll(VAR_REF)) {
        if (!definedTokens.has(m[1]) && !NOT_OURS.has(m[1])) missing.add(`${file}: ${m[1]}`);
      }
    }
    expect([...missing]).toEqual([]);
  });

  // The hook contract, both directions. Either half alone is passable
  // while the app is broken: a hook the palette has claimed still
  // resolves, it just resolves to the wrong thing and silently.
  it("defines none of the names something else supplies", () => {
    expect([...NOT_OURS].filter((n) => definedTokens.has(n))).toEqual([]);
  });

  it("never reads one of its own names through a fallback", () => {
    const shadowed: string[] = [];
    for (const file of COMPONENTS) {
      for (const m of styleOf(read(file)).matchAll(/var\(\s*(--[a-z-]+)\s*,/g)) {
        if (definedTokens.has(m[1])) {
          shadowed.push(`${file}: var(${m[1]}, …) — the fallback is dead`);
        }
      }
    }
    expect(shadowed).toEqual([]);
  });

  // A name nothing reads is a value nobody maintains, and it is the
  // reason `app.css` says to add a role when it is used rather than to
  // complete a set.
  it("defines no name the tree never reads", () => {
    const used = new Set<string>();
    for (const file of COMPONENTS) {
      for (const m of styleOf(read(file)).matchAll(VAR_REF)) used.add(m[1]);
    }
    // A token may also be read by another token's value, or from script.
    for (const value of definedTokens.values()) {
      for (const m of value.matchAll(VAR_REF)) used.add(m[1]);
    }
    for (const file of COMPONENTS) {
      const script = read(file).replace(/<style[^>]*>[\s\S]*?<\/style>/g, "");
      for (const m of script.matchAll(/["'](--[a-z-]+)["']/g)) used.add(m[1]);
    }
    expect(colourTokens.filter((t) => !used.has(t))).toEqual([]);
  });
});

// Two neighbouring tints collapsing onto one name is what made ten
// hover rules no-ops in #240's first pass: the rule still exists, the
// state still matches, and nothing on screen moves.
describe("a state rule changes something", () => {
  it("never resolves every declaration to its own base rule", () => {
    const dead: string[] = [];
    for (const file of COMPONENTS) {
      const rules = rulesOf(styleOf(read(file)));
      const base = new Map<string, Map<string, string>>();
      for (const { selector, decls } of rules) {
        if (HAS_STATE.test(selector)) continue;
        for (const one of selector.split(",")) {
          const key = one.trim().replace(/\s+/g, " ");
          const into = base.get(key) ?? new Map<string, string>();
          for (const [p, v] of decls) into.set(p, v);
          base.set(key, into);
        }
      }
      for (const { selector, decls } of rules) {
        if (!HAS_STATE.test(selector)) continue;
        for (const one of selector.split(",")) {
          const self = one.trim().replace(/\s+/g, " ");
          const stripped = self.replace(STRIP_STATE, "").trim().replace(/\s+/g, " ");
          // `.btn:hover:not(:disabled)` strips to `.btn:not(:disabled)`,
          // which is nobody's base rule — so the lookup missed and six
          // buttons repeated their own fill unnoticed. Try the exact
          // stripped form first, since a base rule may genuinely carry
          // a `:not()`, and only then drop it.
          const under =
            base.get(stripped) ??
            base.get(stripped.replace(STRIP_NOT, "").trim().replace(/\s+/g, " "));
          if (!under) continue;
          const moves = [...decls].some(([p, v]) => under.get(p) !== v);
          if (!moves) dead.push(`${file}: ${self}`);
        }
      }
    }
    expect(dead).toEqual([]);
  });
});

type Rgba = [number, number, number, number];

function parseColour(value: string): Rgba | null {
  const v = value.trim();
  if (v.startsWith("#")) {
    const h = v.slice(1);
    const wide = h.length === 6 || h.length === 8;
    if (h.length !== 3 && !wide) return null;
    const at = (i: number) =>
      wide ? parseInt(h.slice(i * 2, i * 2 + 2), 16) : parseInt(h[i] + h[i], 16);
    return [at(0), at(1), at(2), h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1];
  }
  const fn = v.match(/^rgba?\(([^)]*)\)$/);
  if (!fn) return null;
  const parts = fn[1].split(/[,\s/]+/).filter(Boolean).map(Number);
  if (parts.length < 3 || parts.some(Number.isNaN)) return null;
  return [parts[0], parts[1], parts[2], parts.length > 3 ? parts[3] : 1];
}

const over = (fg: Rgba, bg: Rgba): Rgba => [
  fg[0] * fg[3] + bg[0] * (1 - fg[3]),
  fg[1] * fg[3] + bg[1] * (1 - fg[3]),
  fg[2] * fg[3] + bg[2] * (1 - fg[3]),
  1,
];

function luminance([r, g, b]: Rgba): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a: Rgba, b: Rgba): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

function resolve(token: string, ground: Rgba): Rgba | null {
  const value = definedTokens.get(token);
  if (!value) return null;
  const c = parseColour(value);
  if (!c) return null;
  return c[3] < 1 ? over(c, ground) : c;
}

/**
 * The name a declaration actually paints with.
 *
 * `var(--a, var(--b))` renders `--a` when `--a` is declared and `--b`
 * only when it is not — [MDN: "If the custom property is registered …
 * the fallback is not used in that case"]. Scoring the fallback is how
 * four labels were read as passing while rendering the fill colour at
 * 3.2:1, so walk the names left to right and take the first the palette
 * defines, which is the one the browser will take.
 */
function livePaint(value: string): string | null {
  for (const m of value.matchAll(VAR_REF)) {
    if (definedTokens.has(m[1])) return m[1];
  }
  return null;
}

// `app.css` states the first two: `--ink-faint` is held to 3:1 and
// every other ink to 4.5:1. The third is this file's own — WCAG's
// large-text exemption, which `app.css` has no reason to mention
// because it is a property of the type size rather than of a colour.
const FAINT_BAR = 3;
const BODY_BAR = 4.5;
const LARGE_BAR = 3;

describe("no rule puts unreadable ink on its own ground", () => {
  // Coverage is reported rather than assumed. This check only fires
  // when one rule sets both `color` and a background, so every ink on
  // a ground an ancestor paints — most of the app — goes unscored, and
  // a shrinking numerator would otherwise read as an improving suite.
  const coverage = { scored: 0, inkOnInheritedGround: 0 };

  it("meets the bar app.css states, in every rule that sets both", () => {
    const page = parseColour(definedTokens.get("--surface") ?? "") as Rgba;
    const failures: string[] = [];
    for (const file of COMPONENTS) {
      for (const { selector, decls } of rulesOf(styleOf(read(file)))) {
        const ink = decls.get("color");
        const ground = decls.get("background") ?? decls.get("background-color");
        if (ink && !ground) coverage.inkOnInheritedGround += 1;
        if (!ink || !ground) continue;
        const inkToken = livePaint(ink);
        const groundToken = livePaint(ground);
        if (!inkToken || !groundToken) continue;
        const g = resolve(groundToken, page);
        if (!g) continue;
        const t = resolve(inkToken, g);
        if (!t) continue;
        coverage.scored += 1;

        const rem = decls.get("font-size")?.match(/([\d.]+)rem/);
        const px = rem ? Number(rem[1]) * 16 : 14;
        const weight = Number(decls.get("font-weight") ?? "400");
        const large = px >= 24 || (px >= 18.66 && weight >= 700);
        const bar = inkToken === "--ink-faint" ? FAINT_BAR : large ? LARGE_BAR : BODY_BAR;

        const ratio = contrast(t, g);
        if (ratio < bar) {
          failures.push(
            `${file}: ${selector} — ${inkToken} on ${groundToken} is ${ratio.toFixed(2)}:1, needs ${bar}`,
          );
        }
      }
    }
    expect(failures).toEqual([]);
  });

  it("says how much of the app that actually covered", () => {
    // Not a threshold — a number that has to be looked at. It fell from
    // nothing to this when the check was written, and the honest claim
    // is "the pairs one rule states", not "the app".
    expect(coverage.scored).toBeGreaterThan(0);
    expect(coverage.inkOnInheritedGround).toBeGreaterThan(0);
  });
});
