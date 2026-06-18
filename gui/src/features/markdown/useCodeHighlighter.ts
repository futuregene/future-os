import type { BundledLanguage, BundledTheme, HighlighterGeneric } from "shiki";
import { useCallback, useEffect, useRef, useState } from "react";
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

const SUPPORTED_LANGUAGES: BundledLanguage[] = [
  "typescript",
  "tsx",
  "javascript",
  "jsx",
  "json",
  "html",
  "css",
  "markdown",
  "yaml",
  "toml",
  "rust",
  "python",
  "go",
  "java",
  "c",
  "cpp",
  "csharp",
  "ruby",
  "php",
  "swift",
  "kotlin",
  "scala",
  "shellscript",
  "bash",
  "sql",
  "graphql",
  "xml",
  "diff",
];

const THEME: BundledTheme = "github-light";

let highlighterPromise: Promise<CodeHighlighter> | null = null;
let cachedHighlighter: CodeHighlighter | null = null;

function getHighlighter(): Promise<CodeHighlighter> {
  if (cachedHighlighter) {
    return Promise.resolve(cachedHighlighter);
  }

  if (!highlighterPromise) {
    highlighterPromise = createHighlighter({
      themes: [THEME],
      langs: SUPPORTED_LANGUAGES,
    }).then((highlighter) => {
      cachedHighlighter = highlighter;
      return highlighter;
    }) as Promise<CodeHighlighter>;
  }

  return highlighterPromise;
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
  const [highlighter, setHighlighter] = useState<CodeHighlighter | null>(cachedHighlighter);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;

    if (!highlighter) {
      getHighlighter().then((loaded) => {
        if (mountedRef.current) {
          setHighlighter(loaded);
        }
      });
    }

    return () => {
      mountedRef.current = false;
    };
  }, [highlighter]);

  const highlight = useCallback(
    (code: string, language: string | undefined): HighlightResult | null => {
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
    [highlighter],
  );

  return { highlight, isLoaded: highlighter !== null };
}
