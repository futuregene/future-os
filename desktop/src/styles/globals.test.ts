import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { THEME_PALETTES } from "../lib/theme";

/**
 * Structural checks on the stylesheet that no component test can make.
 *
 * The palettes are plain CSS blocks keyed by `data-palette`; nothing in the
 * type system connects them to `THEME_PALETTES`, so a palette could lose a
 * token (the app silently renders that surface in the previous theme's color)
 * or exist only in one file (the picker offers a theme that does nothing) with
 * every TypeScript check green. These assertions close that gap.
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

const lightTheme = colorTokens(blockAt("@theme {").body);
const darkTheme = colorTokens(blockAt(":root.dark {").body);

/**
 * Tokens a palette must not restate: a search hit is a bright mark with dark
 * text in every theme, so it keeps the shared values on purpose.
 */
const PALETTE_EXEMPT = ["--color-search-match", "--color-search-current", "--color-search-ink"];

const expectedTokens = [...lightTheme.keys()].filter(name => !PALETTE_EXEMPT.includes(name)).sort();

describe("globals.css palettes", () => {
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

  it.each(THEME_PALETTES)("gives the %s palette distinct surfaces from the default", (palette) => {
    if (palette === "futureos")
      return;
    const light = colorTokens(blockAt(`[data-palette="${palette}"] {`).body);
    const dark = colorTokens(blockAt(`:root.dark[data-palette="${palette}"] {`).body);
    // A palette that copied the default wholesale is a theme that does nothing.
    expect(light.get("--color-canvas")).not.toBe(lightTheme.get("--color-canvas"));
    expect(dark.get("--color-canvas")).not.toBe(darkTheme.get("--color-canvas"));
    expect(light.get("--color-accent")).not.toBe(lightTheme.get("--color-accent"));
  });

  it("keeps the default palette's preview block identical to @theme and :root.dark", () => {
    const light = colorTokens(blockAt(`[data-palette="futureos"] {`).body);
    const dark = colorTokens(blockAt(`:root.dark[data-palette="futureos"] {`).body);

    const shared = (a: Map<string, string>, b: Map<string, string>) =>
      [...a].filter(([name]) => !PALETTE_EXEMPT.includes(name) && a.get(name) !== b.get(name));

    expect(shared(light, lightTheme), "futureos light mirror drifted from @theme").toEqual([]);
    expect(shared(dark, darkTheme), "futureos dark mirror drifted from :root.dark").toEqual([]);
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
});
