import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  applyPaletteSection,
  CLOSE_MARKER,
  extractPaletteSection,
  OPEN_MARKER,
  renderPaletteSection,
} from "../../scripts/gen-theme-palettes.mjs";
import { THEME_PALETTES } from "../lib/theme";
import palettes from "../theme/palettes.json";

/**
 * Structural checks on the stylesheet that no component test can make.
 *
 * The palettes are CSS blocks keyed by `data-palette`; nothing in the type
 * system connects them to the theme data, so a palette could lose a token (the
 * app silently renders that surface in the previous theme's color) or the
 * generated block could go stale after a JSON edit — with every TypeScript check
 * green. These assertions close that gap.
 *
 * The default palette is a *mirror* of `@theme` / `:root.dark`: the picker has
 * to be able to preview "FutureOS" while another theme is active, and a card
 * previews a palette by carrying its `data-palette`. If the mirror drifts, the
 * card lies about what the default looks like.
 */

const css = readFileSync(resolve(__dirname, "globals.css"), "utf8");

/** The declarations of the first block whose selector line matches `selector`. */
function blockAt(selector: string): { body: string; start: number } {
  const index = css.indexOf(selector);
  expect(index, `no rule for ${selector}`).toBeGreaterThanOrEqual(0);
  const open = css.indexOf("{", index);
  const close = css.indexOf("}", open);
  return { body: css.slice(open + 1, close), start: index };
}

function declarations(body: string): Map<string, string> {
  const out = new Map<string, string>();
  // Strip comments first: a `;`-split would otherwise glue a comment onto the
  // declaration that follows it and lose that declaration's name.
  const stripped = body.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const declaration of stripped.split(";")) {
    const colon = declaration.indexOf(":");
    if (colon < 0)
      continue;
    const name = declaration.slice(0, colon).trim();
    if (!/^--[\w-]+$/.test(name))
      continue;
    out.set(name, declaration.slice(colon + 1).trim().replace(/\s+/g, " "));
  }
  return out;
}

function colorTokens(body: string): Map<string, string> {
  const all = declarations(body);
  return new Map([...all].filter(([name]) => name.startsWith("--color-")));
}

/** Every declaration in a block, custom properties and standard ones alike. */
function properties(body: string): Map<string, string> {
  const out = new Map<string, string>();
  const stripped = body.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const declaration of stripped.split(";")) {
    const colon = declaration.indexOf(":");
    if (colon < 0)
      continue;
    const name = declaration.slice(0, colon).trim().replace(/\s+/g, " ");
    if (!/^-{0,2}[a-z][\w-]*$/.test(name))
      continue;
    out.set(name, declaration.slice(colon + 1).trim().replace(/\s+/g, " "));
  }
  return out;
}

const lightTheme = colorTokens(blockAt("@theme {").body);
const darkTheme = colorTokens(blockAt(":root.dark {").body);

/**
 * Tokens a palette must not restate: a search hit is a bright mark with dark
 * text in every theme, so it keeps the shared values on purpose.
 */
const PALETTE_EXEMPT = ["--color-search-match", "--color-search-current", "--color-search-ink"];

const expectedTokens = [...lightTheme.keys()].filter(name => !PALETTE_EXEMPT.includes(name)).sort();

describe("globals.css palettes", () => {
  it("matches the generated section, so the stylesheet is never stale", () => {
    // `renderPaletteSection` is the same function the generator writes with.
    expect(extractPaletteSection(css)).toBe(renderPaletteSection(palettes as never));
    expect(applyPaletteSection(css, extractPaletteSection(css))).toBe(css);
  });

  it("declares the same color tokens in the light and dark base palettes", () => {
    expect([...darkTheme.keys()].sort()).toEqual([...lightTheme.keys()].sort());
  });

  it.each(THEME_PALETTES)("styles the %s palette in both renditions, with every token", (palette) => {
    const light = colorTokens(blockAt(`[data-palette="${palette}"] {`).body);
    const dark = colorTokens(blockAt(`:root.dark[data-palette="${palette}"] {`).body);

    // A missing token falls back to whatever the previous palette set, which
    // looks like a rendering bug rather than an unstyled surface.
    expect([...light.keys()].sort(), `${palette} light`).toEqual(expectedTokens);
    expect([...dark.keys()].sort(), `${palette} dark`).toEqual(expectedTokens);
  });

  it("keeps the default palette's preview block identical to @theme and :root.dark", () => {
    const light = colorTokens(blockAt(`[data-palette="futureos"] {`).body);
    const dark = colorTokens(blockAt(`:root.dark[data-palette="futureos"] {`).body);

    const drifted = (a: Map<string, string>, b: Map<string, string>) =>
      [...a].filter(([name]) => !PALETTE_EXEMPT.includes(name) && a.get(name) !== b.get(name));

    expect(drifted(light, lightTheme), "futureos light mirror drifted from @theme").toEqual([]);
    expect(drifted(dark, darkTheme), "futureos dark mirror drifted from :root.dark").toEqual([]);
  });

  it("orders the palette blocks after :root.dark", () => {
    // `[data-palette="x"]` and `:root` have the same specificity, so which one
    // wins is decided by source order alone.
    const base = blockAt(":root.dark {").start;
    for (const palette of THEME_PALETTES) {
      expect(blockAt(`[data-palette="${palette}"] {`).start, palette).toBeGreaterThan(base);
      expect(blockAt(`:root.dark[data-palette="${palette}"] {`).start, palette).toBeGreaterThan(base);
    }
  });

  it("matches a nested preview as well as the root", () => {
    // The picker paints each card with its own `data-palette`, so every dark
    // rule needs the descendant form: the root selector alone would leave a
    // card in a dark window showing its light colors.
    for (const palette of THEME_PALETTES) {
      expect(css).toContain(`.dark [data-palette="${palette}"]`);
    }
  });

  it("keeps both markers, so the generator can find its section", () => {
    expect(css.split(OPEN_MARKER).length).toBe(2);
    expect(css.split(CLOSE_MARKER).length).toBe(2);
    expect(css.indexOf(OPEN_MARKER)).toBeLessThan(css.indexOf(CLOSE_MARKER));
  });
});

/**
 * The base rendition: everything that is not a color.
 *
 * None of it is visible to a component test — a stylesheet that loses its
 * `:root` block still type-checks, still renders every class, and still passes
 * the palette suites, while the whole app quietly changes: glyphs painted
 * subpixel-antialiased (every label reads heavier), the reader's text size
 * setting doing nothing, flat panels in a light window, and animation classes
 * whose keyframes no longer exist. These assertions pin each one.
 */
describe("globals.css base rendition", () => {
  // Read lazily: a missing `:root` block is the regression under test, so each
  // assertion has to fail on its own rather than at collection time.
  const baseOf = () => properties(blockAt(":root {").body);
  const darkOf = () => properties(blockAt(":root.dark {").body);

  it("declares the reader's text size on the type tokens Tailwind generates", () => {
    // `lib/fontSize.ts` only writes `--app-font-scale`; if no token reads it the
    // setting is dead. Covers the sub-`text-xs` labels (`@theme` declares their
    // design-time size) as well as Tailwind's own steps.
    const base = baseOf();
    expect(base.get("--app-font-scale")).toBe("1");
    for (const size of ["3xs", "2xs", "13", "xs", "sm", "base", "lg", "xl", "2xl", "3xl", "4xl"]) {
      const value = base.get(`--text-${size}`);
      expect(value, `--text-${size}`).toBeTruthy();
      expect(value, `--text-${size}`).toContain("var(--app-font-scale)");
    }
  });

  it("keeps the font stack and the antialiasing it was designed with", () => {
    // The stack falls back to Tailwind's `ui-sans-serif, system-ui` without it;
    // the smoothing flags are what keep `text-sm font-medium` from reading bold.
    const base = baseOf();
    expect(base.get("font-family")).toContain("PingFang SC");
    expect(base.get("font-synthesis")).toBe("none");
    expect(base.get("text-rendering")).toBe("optimizeLegibility");
    expect(base.get("-webkit-font-smoothing")).toBe("antialiased");
    expect(base.get("-moz-osx-font-smoothing")).toBe("grayscale");
  });

  it("declares light shadows, so a light window's panels are not flat", () => {
    // The dark values live in `:root.dark`; a light window needs its own, and a
    // copy of the dark ones would be visible as a black haze.
    const base = baseOf();
    const dark = darkOf();
    for (const name of ["--shadow-panel", "--shadow-dialog", "--shadow-sidebar-divider", "--shadow-sidebar-floating"]) {
      expect(base.get(name), name).toBeTruthy();
      expect(base.get(name), name).not.toBe(dark.get(name));
    }
  });

  it("orders the base block after :root.dark, before the palettes", () => {
    // `:root.dark` wins on specificity, so this block may follow it; the palette
    // blocks must stay last so a chosen palette still overrides both.
    const root = blockAt(":root {");
    expect(root.start).toBeGreaterThan(blockAt(":root.dark {").start);
    expect(root.start).toBeLessThan(blockAt(`[data-palette="${THEME_PALETTES[0]}"] {`).start);
  });

  it("defines every hand-written animation its components ask for", () => {
    // `animate-spin` and friends are Tailwind's; these two are ours, and a
    // component keeps the class whether or not the rule exists.
    // `AgentThread`/`RemoteConversationView` use the pop-in, and
    // `ActivityRail`'s Skills entry the pulse.
    for (const animation of ["pop-in", "skill-entry-pulse"]) {
      expect(css, animation).toContain(`@keyframes ${animation} {`);
      expect(css, animation).toContain(`.animate-${animation} {`);
    }
  });

  it("keeps the rail toolbar's container query and the search highlights", () => {
    // `ActivityRailSelectionToolbar` shrinks its actions through this container;
    // `ThreadSearch` registers the two highlight names with the CSS Custom
    // Highlight API, which paints them only if these rules exist.
    expect(css).toContain("container: activity-rail-selection-toolbar / inline-size");
    expect(css).toContain("@container activity-rail-selection-toolbar (max-width: 279px)");
    expect(css).toContain(".activity-rail-selection-action-label");
    expect(css).toContain("::highlight(thread-search-match)");
    expect(css).toContain("::highlight(thread-search-current)");
  });
});
