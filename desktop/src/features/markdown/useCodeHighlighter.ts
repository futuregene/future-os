import type { BundledTheme } from "shiki";
import type { CodeHighlighterEngine, HighlightResult } from "./codeHighlighterEngine";
import { useCallback, useEffect, useSyncExternalStore } from "react";
import { THEME_PALETTES, useResolvedTheme, useThemePalette } from "../../lib/theme";
import { paletteShiki } from "../../theme/palettes";

export type { HighlightedLine, HighlightedToken, HighlightResult } from "./codeHighlighterEngine";
export { MAX_HIGHLIGHTABLE_CODE_LENGTH } from "./codeHighlightLimits";

/**
 * Code block colors come from the theme's own data (`src/theme/palettes.json`),
 * so a palette supplies its syntax-highlighting theme the same way it supplies
 * its surfaces.
 *
 * Each palette reuses the editor theme its upstream project ships (`nord`,
 * `solarized-light` / `solarized-dark`, `gruvbox-light-medium` /
 * `gruvbox-dark-medium`, `rose-pine-dawn` / `rose-pine-moon`,
 * `catppuccin-latte` / `catppuccin-mocha`, `tokyo-night`, `dracula`,
 * `one-dark-pro` / `one-light`, `everforest-dark` / `everforest-light`,
 * `kanagawa-wave` / `kanagawa-lotus`, `ayu-dark` / `ayu-light`, `vesper`): a
 * Nord code block next to Nord chrome should look like code in Nord, not like
 * GitHub Dark tinted cold. Where the project ships only one rendition, the
 * palette names the nearest available editor theme for the other (Dracula →
 * Rosé Pine Dawn, Nightfox → Nord, Vesper/Tokyo Night → One Light); the
 * mapping is visible in the JSON and listed in `COLOR.md`.
 *
 * Every bundled theme is loaded with the instance (a few KB of JSON each) so
 * switching the app theme never leaves code blocks on the old palette.
 *
 * The instance itself, its grammars and the tokenizer live in
 * `codeHighlighterEngine`, which is imported on the first code block to mount:
 * this module is in every message's import graph, so it must not carry Shiki.
 * It only names the themes and delegates.
 */
const BUNDLED_THEMES: BundledTheme[] = [
  ...new Set(THEME_PALETTES.flatMap(palette => [
    paletteShiki(palette, "light"),
    paletteShiki(palette, "dark"),
  ])),
];

let enginePromise: Promise<CodeHighlighterEngine> | null = null;
let cachedEngine: CodeHighlighterEngine | null = null;

// External store backing `useSyncExternalStore`: `storeVersion` bumps whenever
// the shared engine becomes ready or a grammar finishes loading. Subscribing
// components re-read the snapshot right after subscribing, so a load that
// completes between a block's first render and its subscription is never missed
// (the bug the old one-shot broadcast had).
let storeVersion = 0;
const storeListeners = new Set<() => void>();

function emitChange() {
  storeVersion++;
  for (const listener of storeListeners) {
    listener();
  }
}

function subscribe(listener: () => void): () => void {
  storeListeners.add(listener);
  return () => {
    storeListeners.delete(listener);
  };
}

function getSnapshot(): number {
  return storeVersion;
}

// Server-rendered output never has the async highlighter available, so it must
// render the un-highlighted fallback. Return a stable version (0) so
// useSyncExternalStore doesn't throw under renderToStaticMarkup / SSR.
function getServerSnapshot(): number {
  return 0;
}

function getEngine(): Promise<CodeHighlighterEngine> {
  /* v8 ignore start -- the only caller (the mount effect) guards on
     !cachedEngine, so this fast path is unreachable defense */
  if (cachedEngine) {
    return Promise.resolve(cachedEngine);
  }
  /* v8 ignore stop */

  if (!enginePromise) {
    // The chunk, then the instance: both start at the first code block, so a
    // launch that never shows source never pays for the tokenizer.
    enginePromise = import("./codeHighlighterEngine")
      .then(({ createCodeHighlighterEngine }) =>
        createCodeHighlighterEngine({ themes: BUNDLED_THEMES, onGrammarLoaded: emitChange }),
      )
      .then((engine) => {
        cachedEngine = engine;
        emitChange();
        return engine;
      })
      .catch((error: unknown) => {
        // A failed chunk fetch (or a broken grammar set) must not wedge the
        // hook on a rejected promise: forget it so the next mount retries, and
        // leave every block on its plain-text fallback meanwhile.
        enginePromise = null;
        throw error;
      });
  }

  return enginePromise;
}

export function useCodeHighlighter() {
  // `version` bumps when the engine becomes ready or a grammar loads.
  // useSyncExternalStore re-reads the snapshot immediately after subscribing, so
  // a load that finishes between this block's first render and its subscription
  // still triggers a re-render — no missed notification.
  const version = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
  // Components re-render when the app theme flips, and the token colors below
  // come from the matching Shiki theme.
  const resolvedTheme = useResolvedTheme();
  const palette = useThemePalette();

  useEffect(() => {
    // Kick off the shared engine lazily on first mount; getEngine is idempotent
    // and emits a store change once it resolves.
    if (!cachedEngine) {
      void getEngine().catch(() => {
        // Reported through the fallback: every block keeps rendering plain text.
      });
    }
  }, []);

  const highlight = useCallback(
    (code: string, language: string | undefined): HighlightResult | null => {
      const engine = cachedEngine;
      if (!engine) {
        return null;
      }
      return engine.highlight(code, language, paletteShiki(palette, resolvedTheme));
    },
    // `version` isn't read in the body but is an intentional dep: bumping it when
    // the engine/grammar loads gives `highlight` a new identity so memoized
    // consumers re-run against the now-ready engine or language.
    // eslint-disable-next-line react/exhaustive-deps
    [version, palette, resolvedTheme],
  );

  return { highlight, isLoaded: cachedEngine !== null };
}
