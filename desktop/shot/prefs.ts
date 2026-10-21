/**
 * The preferences every capture boots with.
 *
 * Two jobs:
 *
 * 1. **Pin them from the URL.** A preference is read *at module load* by the
 *    code that applies it (`lib/theme.ts` applies the palette before the first
 *    paint, `i18n` picks its language at init), so it has to be in
 *    `localStorage` before the app's module graph is evaluated — hence a
 *    separate script that `shot/index.html` loads before `main.tsx`
 *    (module scripts execute in document order). Setting it in `main.tsx`'s
 *    body would be too late.
 * 2. **Make every capture start from the same state.** The headless browser
 *    keeps one profile for the whole run, so a scenario that switches the color
 *    theme would otherwise leak it into every scenario captured after it — and
 *    a machine whose OS is dark would capture dark screenshots of screens
 *    nobody asked to see dark. Parameters default to what a first run shows.
 *
 * Parameters (all optional):
 *   ?lang=zh|en                  UI language (default zh)
 *   ?theme=light|dark|system     appearance (default light)
 *   ?palette=<palette id>        color theme (default futureos; see
 *                                `lib/theme.ts` THEME_PALETTES)
 */
const params = new URLSearchParams(location.search);

const pins: Record<string, string> = {
  "future.language": params.get("lang") === "en" ? "en" : "zh",
  "future.theme": params.get("theme") ?? "light",
  "future.palette": params.get("palette") ?? "futureos",
};

for (const [key, value] of Object.entries(pins)) {
  localStorage.setItem(key, value);
}

export {};
