import type { BundledLanguage, BundledTheme, HighlighterGeneric } from "shiki";
import { useCallback, useEffect, useSyncExternalStore } from "react";
import { createHighlighter } from "shiki";

type CodeHighlighter = HighlighterGeneric<BundledLanguage, BundledTheme>;

interface HighlightedToken {
  content: string;
  color: string;
  fontStyle?: number;
}

interface HighlightedLine {
  tokens: HighlightedToken[];
}

interface HighlightResult {
  lines: HighlightedLine[];
  bgColor: string;
  fgColor: string;
}

const THEME: BundledTheme = "github-light";

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

function getHighlighter(): Promise<CodeHighlighter> {
  if (cachedHighlighter) {
    return Promise.resolve(cachedHighlighter);
  }

  if (!highlighterPromise) {
    // Start with no grammars — each language's grammar is loaded on first use
    // (see ensureLanguageLoaded) instead of paying to parse all of them upfront
    // when the first code block appears.
    highlighterPromise = createHighlighter({
      themes: [THEME],
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

function normalizeLanguage(language: string | undefined): BundledLanguage | null {
  if (!language) {
    return null;
  }

  const normalized = language.toLowerCase().trim();

  const languageMap: Record<string, BundledLanguage> = {
    "ts": "typescript",
    "tsx": "tsx",
    "js": "javascript",
    "jsx": "jsx",
    "javascript": "javascript",
    "typescript": "typescript",
    "json": "json",
    "html": "html",
    "css": "css",
    "md": "markdown",
    "markdown": "markdown",
    "yml": "yaml",
    "yaml": "yaml",
    "toml": "toml",
    "rs": "rust",
    "rust": "rust",
    "py": "python",
    "python": "python",
    "go": "go",
    "golang": "go",
    "java": "java",
    "c": "c",
    "cpp": "cpp",
    "c++": "cpp",
    "cs": "csharp",
    "csharp": "csharp",
    "c#": "csharp",
    "rb": "ruby",
    "ruby": "ruby",
    "php": "php",
    "swift": "swift",
    "kt": "kotlin",
    "kotlin": "kotlin",
    "scala": "scala",
    "sh": "shellscript",
    "bash": "bash",
    "shell": "shellscript",
    "zsh": "shellscript",
    "fish": "shellscript",
    "sql": "sql",
    "graphql": "graphql",
    "xml": "xml",
    "diff": "diff",
  };

  return languageMap[normalized] ?? null;
}

export function useCodeHighlighter() {
  // `version` bumps when the highlighter becomes ready or a grammar loads.
  // useSyncExternalStore re-reads the snapshot immediately after subscribing, so
  // a load that finishes between this block's first render and its subscription
  // still triggers a re-render — no missed notification.
  const version = useSyncExternalStore(subscribe, getSnapshot);

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
          theme: THEME,
        });

        const theme = highlighter.getTheme(THEME);
        const bgColor = typeof theme.bg === "string" ? theme.bg : "#ffffff";
        const fgColor = typeof theme.fg === "string" ? theme.fg : "#000000";

        const lines: HighlightedLine[] = tokens.tokens.map(line => ({
          tokens: line.map(token => ({
            content: token.content,
            color: typeof token.color === "string" ? token.color : fgColor,
            fontStyle: typeof token.fontStyle === "number" ? token.fontStyle : undefined,
          })),
        }));

        return { lines, bgColor, fgColor };
      }
      catch {
        return null;
      }
    },
    // `version` isn't read in the body but is an intentional dep: bumping it when
    // the highlighter/grammar loads gives `highlight` a new identity so memoized
    // consumers re-run against the now-ready highlighter or language.
    // eslint-disable-next-line react/exhaustive-deps
    [version],
  );

  return { highlight, isLoaded: cachedHighlighter !== null };
}
