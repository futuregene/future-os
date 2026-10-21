#!/usr/bin/env node
/**
 * Render the color themes into `src/styles/globals.css`.
 *
 * `src/theme/palettes.json` is the source of truth; this writes the
 * `[data-palette="…"]` blocks between the two markers in the stylesheet:
 *
 *   /* <theme-palettes> *\/            ← generated below this line
 *   ...
 *   /* </theme-palettes> *\/
 *
 * Run it after editing the JSON:
 *
 *   cd desktop && node scripts/gen-theme-palettes.mjs
 *
 * `src/styles/globals.test.ts` fails, with the same instruction, when the
 * stylesheet and the JSON disagree — so a forgotten run is caught rather than
 * shipped.
 */
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
export const STYLESHEET = path.join(here, "..", "src", "styles", "globals.css");
export const PALETTES_JSON = path.join(here, "..", "src", "theme", "palettes.json");

export const OPEN_MARKER = "/* <theme-palettes> */";
export const CLOSE_MARKER = "/* </theme-palettes> */";

const BANNER_WIDTH = 76;

/**
 * One theme's two rules: the light rendition (which also matches a nested
 * preview element, so the Settings picker can paint a card per theme) and the
 * dark one (which has to win over `:root.dark` for the root and over the light
 * rule for a preview inside a dark window).
 */
function renderTheme(theme) {
  const title = `/* ── ${theme.id.toUpperCase()} — ${theme.origin} `;
  const banner = title + "─".repeat(Math.max(3, BANNER_WIDTH - title.length));
  const lines = [
    banner,
    "   Generated from `src/theme/palettes.json`; edit there and re-run",
    "   `node scripts/gen-theme-palettes.mjs`. */",
    `[data-palette="${theme.id}"] {`,
    ...Object.entries(theme.light).map(([token, value]) => `  --color-${token}: ${value};`),
    "}",
    "",
    `.dark [data-palette="${theme.id}"],`,
    `:root.dark[data-palette="${theme.id}"] {`,
    ...Object.entries(theme.dark).map(([token, value]) => `  --color-${token}: ${value};`),
    "}",
  ];
  return lines.join("\n");
}

/** The full generated section, markers included. */
export function renderPaletteSection(palettes) {
  const blocks = palettes.themes.map(renderTheme).join("\n\n");
  return `${OPEN_MARKER}\n${blocks}\n${CLOSE_MARKER}`;
}

/** The generated section as it currently sits in `css`, markers included. */
export function extractPaletteSection(css) {
  const start = css.indexOf(OPEN_MARKER);
  const end = css.indexOf(CLOSE_MARKER);
  if (start < 0 || end < 0 || end < start)
    throw new Error(`globals.css is missing the ${OPEN_MARKER} / ${CLOSE_MARKER} markers`);
  return css.slice(start, end + CLOSE_MARKER.length);
}

/** Replace (or insert) the generated section. Exported for the test to reuse. */
export function applyPaletteSection(css, section) {
  const start = css.indexOf(OPEN_MARKER);
  const end = css.indexOf(CLOSE_MARKER);
  if (start < 0 || end < 0 || end < start)
    throw new Error(`globals.css is missing the ${OPEN_MARKER} / ${CLOSE_MARKER} markers`);
  return css.slice(0, start) + section + css.slice(end + CLOSE_MARKER.length);
}

function main() {
  const palettes = JSON.parse(readFileSync(PALETTES_JSON, "utf8"));
  const css = readFileSync(STYLESHEET, "utf8");
  const next = applyPaletteSection(css, renderPaletteSection(palettes));
  if (next === css) {
    console.log(`theme palettes: ${palettes.themes.length} themes already current`);
    return;
  }
  writeFileSync(STYLESHEET, next);
  console.log(`theme palettes: wrote ${palettes.themes.length} themes into ${path.relative(process.cwd(), STYLESHEET)}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  main();
