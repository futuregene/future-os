import type { BundledLanguage, BundledTheme, HighlighterGeneric } from "shiki";
import type { ResolvedTheme, ThemePalette } from "../../lib/theme";
import { useCallback, useEffect, useSyncExternalStore } from "react";
import { bundledLanguagesInfo, createHighlighter } from "shiki";
import { useResolvedTheme, useThemePalette } from "../../lib/theme";

type CodeHighlighter = HighlighterGeneric<BundledLanguage, BundledTheme>;

export interface HighlightedToken {
  content: string;
  color: string;
  fontStyle?: number;
}

export interface HighlightedLine {
  tokens: HighlightedToken[];
}

export interface HighlightResult {
  lines: HighlightedLine[];
  fgColor: string;
}

/**
 * Shiki theme per palette and rendition.
 *
 * Code block colors are the one place the app hands syntax highlighting to a
 * third party, so each palette reuses the highlighting theme its authors ship
 * instead of a hand-rolled derivative — a Nord code block next to Nord chrome
 * should look like code in Nord, not like GitHub Dark tinted cold. The two
 * GitHub themes stay the default palette's, because FutureOS's light/dark
 * palette was drawn from them.
 *
 * Every palette needs both renditions: a palette with only one would leave code
 * blocks on the wrong background half the time.
 */
const THEMES: Record<ThemePalette, Record<ResolvedTheme, BundledTheme>> = {
  "futureos": { light: "github-light", dark: "github-dark" },
  "nord": { light: "nord", dark: "nord" },
  "solarized": { light: "solarized-light", dark: "solarized-dark" },
  "gruvbox": { light: "gruvbox-light-medium", dark: "gruvbox-dark-medium" },
  "rose-pine": { light: "rose-pine-dawn", dark: "rose-pine-moon" },
  "catppuccin": { light: "catppuccin-latte", dark: "catppuccin-mocha" },
};

/** Every bundled theme, deduplicated: the highlighter loads them all up front. */
const BUNDLED_THEMES: BundledTheme[] = [
  ...new Set(Object.values(THEMES).flatMap(perRendition => Object.values(perRendition))),
];

let highlighterPromise: Promise<CodeHighlighter> | null = null;
let cachedHighlighter: CodeHighlighter | null = null;
// Languages whose grammar is currently being fetched, so concurrent code blocks
// asking for the same language don't each kick off a load.
const inFlightLanguages = new Set<BundledLanguage>();

// External store backing `useSyncExternalStore`: `storeVersion` bumps whenever
// the shared highlighter becomes ready or a grammar finishes loading. Subscribing
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

function getHighlighter(): Promise<CodeHighlighter> {
  /* v8 ignore start -- the only caller (the mount effect) guards on
     !cachedHighlighter, so this fast path is unreachable defense */
  if (cachedHighlighter) {
    return Promise.resolve(cachedHighlighter);
  }
  /* v8 ignore stop */

  if (!highlighterPromise) {
    // Start with no grammars — each language's grammar is loaded on first use
    // (see ensureLanguageLoaded) instead of paying to parse all of them upfront
    // when the first code block appears.
    highlighterPromise = createHighlighter({
      themes: BUNDLED_THEMES,
      langs: [],
    }).then((highlighter) => {
      cachedHighlighter = highlighter;
      emitChange();
      return highlighter;
    }) as Promise<CodeHighlighter>;
  }

  return highlighterPromise;
}

function ensureLanguageLoaded(highlighter: CodeHighlighter, lang: BundledLanguage) {
  if (highlighter.getLoadedLanguages().includes(lang) || inFlightLanguages.has(lang)) {
    return;
  }
  inFlightLanguages.add(lang);
  highlighter
    .loadLanguage(lang)
    .then(() => {
      inFlightLanguages.delete(lang);
      emitChange();
    })
    .catch(() => {
      // Grammar failed to load — leave it unloaded so the block stays plain text.
      inFlightLanguages.delete(lang);
    });
}

// Use Shiki's registry rather than a partial whitelist: e.g. PowerShell,
// Dockerfile and Vue should highlight too. Aliases share one grammar/cache key.
const languageNames = new Map<string, BundledLanguage>();
for (const language of bundledLanguagesInfo) {
  const id = language.id as BundledLanguage;
  languageNames.set(id, id);
  for (const alias of language.aliases ?? []) languageNames.set(alias, id);
}
for (const [alias, language] of Object.entries({
  "c++": "cpp",
  "c#": "csharp",
  "golang": "go",
  "shell": "shellscript",
  "zsh": "shellscript",
} satisfies Record<string, BundledLanguage>)) languageNames.set(alias, language);

function normalizeLanguage(language: string | undefined): BundledLanguage | null {
  return languageNames.get(language?.toLowerCase().trim() ?? "") ?? null;
}

// Cross-instance highlight cache: `CodeBlock` memoizes its own tokenization,
// but a thread switch unmounts every block and re-tokenizes from scratch.
// Caching by (language, code) makes a re-mount of a previously-rendered block
// a map lookup instead of a full Shiki tokenize. Results are plain
// read-only token arrays, so sharing is safe. LRU-bounded; huge blocks are
// skipped (tokenizing them once is cheaper than pinning the cache).
const highlightCache = new Map<string, HighlightResult>();
const HIGHLIGHT_CACHE_MAX = 400;

/**
 * The largest source worth tokenizing: past this, `highlight` still works but
 * the result is not cached. That single bound is why full-screen previews gate
 * on it too — above it a 200KB source costs ~200ms of tokenizing plus tens of
 * thousands of spans, and the preview shows plain monospace instead.
 */
export const MAX_HIGHLIGHTABLE_CODE_LENGTH = 100_000;

export function useCodeHighlighter() {
  // `version` bumps when the highlighter becomes ready or a grammar loads.
  // useSyncExternalStore re-reads the snapshot immediately after subscribing, so
  // a load that finishes between this block's first render and its subscription
  // still triggers a re-render — no missed notification.
  const version = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
  // Components re-render when the app theme flips, and the token colors below
  // come from the matching Shiki theme.
  const resolvedTheme = useResolvedTheme();
  const palette = useThemePalette();

  useEffect(() => {
    // Kick off the shared highlighter lazily on first mount; getHighlighter is
    // idempotent and emits a store change once it resolves.
    if (!cachedHighlighter) {
      void getHighlighter();
    }
  }, []);

  const highlight = useCallback(
    (code: string, language: string | undefined): HighlightResult | null => {
      const highlighter = cachedHighlighter;
      if (!highlighter) {
        return null;
      }

      const normalizedLang = normalizeLanguage(language);
      if (!normalizedLang) {
        return null;
      }

      const shikiTheme = THEMES[palette][resolvedTheme];
      const cacheKey = `${shikiTheme}\u0000${normalizedLang}\u0000${code}`;
      const cached = highlightCache.get(cacheKey);
      if (cached) {
        // LRU touch.
        highlightCache.delete(cacheKey);
        highlightCache.set(cacheKey, cached);
        return cached;
      }

      try {
        const loadedLanguages = highlighter.getLoadedLanguages();
        if (!loadedLanguages.includes(normalizedLang)) {
          // Not loaded yet — start the on-demand load and fall back to plain
          // text; the store change re-arms `highlight` once the grammar lands.
          ensureLanguageLoaded(highlighter, normalizedLang);
          return null;
        }

        const tokens = highlighter.codeToTokens(code, {
          lang: normalizedLang,
          theme: shikiTheme,
        });

        const theme = highlighter.getTheme(shikiTheme);
        const fgColor = typeof theme.fg === "string" ? theme.fg : "#000000";

        const lines: HighlightedLine[] = tokens.tokens.map(line => ({
          tokens: line.map(token => ({
            content: token.content,
            color: typeof token.color === "string" ? token.color : fgColor,
            fontStyle: typeof token.fontStyle === "number" ? token.fontStyle : undefined,
          })),
        }));

        const result: HighlightResult = { lines, fgColor };
        if (code.length <= MAX_HIGHLIGHTABLE_CODE_LENGTH) {
          if (highlightCache.size >= HIGHLIGHT_CACHE_MAX) {
            const oldest = highlightCache.keys().next().value;
            if (oldest !== undefined)
              highlightCache.delete(oldest);
          }
          highlightCache.set(cacheKey, result);
        }
        return result;
      }
      catch {
        return null;
      }
    },
    // `version` isn't read in the body but is an intentional dep: bumping it when
    // the highlighter/grammar loads gives `highlight` a new identity so memoized
    // consumers re-run against the now-ready highlighter or language.
    // eslint-disable-next-line react/exhaustive-deps
    [version, palette, resolvedTheme],
  );

  return { highlight, isLoaded: cachedHighlighter !== null };
}
