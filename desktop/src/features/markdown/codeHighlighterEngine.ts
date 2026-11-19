import type { BundledLanguage, BundledTheme, HighlighterGeneric } from "shiki";
import { bundledLanguagesInfo, createHighlighter } from "shiki";
import { MAX_HIGHLIGHTABLE_CODE_LENGTH } from "./codeHighlightLimits";

/**
 * The Shiki half of code highlighting: everything that touches the library —
 * its grammars, its themes and the tokenizer — so that `useCodeHighlighter`
 * (which every message with a fenced block imports) stays a thin, Shiki-free
 * consumer. This module is reached through `import("../codeHighlighterEngine")`
 * the first time a code block mounts, which keeps ~110 kB of tokenizer out of
 * the entry chunk the app parses on every launch.
 */
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

type CodeHighlighter = HighlighterGeneric<BundledLanguage, BundledTheme>;

export interface CodeHighlighterEngine {
  /**
   * Tokenizes synchronously. Returns null when the instance or the requested
   * grammar is not ready yet, having started the grammar load; the caller
   * re-runs once the load reports back through `onGrammarLoaded`.
   */
  highlight: (code: string, language: string | undefined, theme: BundledTheme) => HighlightResult | null;
}

export interface CodeHighlighterEngineOptions {
  /** Every theme a palette can select, loaded with the instance so a theme flip never leaves a block behind. */
  themes: BundledTheme[];
  /** Grammar loads finish asynchronously; this is how blocks learn to re-tokenize. */
  onGrammarLoaded: () => void;
}

// Use Shiki's registry rather than a partial whitelist: e.g. PowerShell,
// Dockerfile and Vue should highlight too. Aliases share one grammar/cache key.
const languageNames = new Map<string, BundledLanguage>();
for (const language of bundledLanguagesInfo) {
  const id = language.id as BundledLanguage;
  languageNames.set(id, id);
  for (const alias of language.aliases ?? [])
    languageNames.set(alias, id);
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

export async function createCodeHighlighterEngine({
  themes,
  onGrammarLoaded,
}: CodeHighlighterEngineOptions): Promise<CodeHighlighterEngine> {
  // Start with no grammars — each language's grammar is loaded on first use
  // instead of paying to parse all of them upfront when the first code block
  // appears.
  const highlighter = await createHighlighter({ themes, langs: [] }) as CodeHighlighter;

  // Languages whose grammar is currently being fetched, so concurrent code
  // blocks asking for the same language don't each kick off a load.
  const inFlightLanguages = new Set<BundledLanguage>();

  function ensureLanguageLoaded(lang: BundledLanguage) {
    if (highlighter.getLoadedLanguages().includes(lang) || inFlightLanguages.has(lang))
      return;
    inFlightLanguages.add(lang);
    highlighter
      .loadLanguage(lang)
      .then(() => {
        inFlightLanguages.delete(lang);
        onGrammarLoaded();
      })
      .catch(() => {
        // Grammar failed to load — leave it unloaded so the block stays plain text.
        inFlightLanguages.delete(lang);
      });
  }

  // Cross-instance highlight cache: `CodeBlock` memoizes its own tokenization,
  // but a thread switch unmounts every block and re-tokenizes from scratch.
  // Caching by (language, code) makes a re-mount of a previously-rendered block
  // a map lookup instead of a full Shiki tokenize. Results are plain
  // read-only token arrays, so sharing is safe. LRU-bounded; huge blocks are
  // skipped (tokenizing them once is cheaper than pinning the cache).
  const highlightCache = new Map<string, HighlightResult>();
  const HIGHLIGHT_CACHE_MAX = 400;

  return {
    highlight(code, language, theme) {
      const normalizedLang = normalizeLanguage(language);
      if (!normalizedLang)
        return null;

      const cacheKey = `${theme}\u0000${normalizedLang}\u0000${code}`;
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
          // text; the store change re-arms the caller once the grammar lands.
          ensureLanguageLoaded(normalizedLang);
          return null;
        }

        const tokens = highlighter.codeToTokens(code, {
          lang: normalizedLang,
          theme,
        });

        const resolvedTheme = highlighter.getTheme(theme);
        const fgColor = typeof resolvedTheme.fg === "string" ? resolvedTheme.fg : "#000000";

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
  };
}
