// @vitest-environment jsdom
import { act, createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";

/**
 * `useCodeHighlighter` keeps its whole state at module scope — the highlighter
 * promise, the cached instance, the in-flight grammar set, the store version
 * and the highlight LRU. That state is one-per-process by design (one Shiki
 * instance for the app), so a test that asserts *how* it gets warm ("unloaded
 * before the first mount resolves") is only meaningful on a fresh module. Each
 * test therefore re-imports the hook through `vi.resetModules()`, the same way
 * `lib/fontSize.test.ts` and `lib/doneBell.context.test.ts` do, instead of
 * inheriting whatever an earlier test in the file happened to leave warm.
 *
 * `../../lib/theme` is re-imported in the same graph: the hook reads the theme
 * module's state, so a test that mutates it has to call the *same* instance.
 */
beforeEach(() => {
  vi.resetModules();
});

// A fully controllable fake Shiki highlighter.
const loadedLanguages: string[] = [];
let loadLanguageImpl: (lang: string) => Promise<void> = async (lang) => {
  loadedLanguages.push(lang);
};
let codeToTokensImpl: (code: string) => { tokens: Array<Array<Record<string, unknown>>> } = code => ({
  tokens: [[{ content: code, color: "#111", fontStyle: 1 }, { content: "!", fontStyle: undefined, color: undefined }]],
});

function defaultCodeToTokens(code: string) {
  return {
    tokens: [[{ content: code, color: "#111", fontStyle: 1 }, { content: "!", fontStyle: undefined, color: undefined }]],
  };
}

const fakeHighlighter = {
  getLoadedLanguages: () => [...loadedLanguages],
  loadLanguage: vi.fn((lang: string) => loadLanguageImpl(lang)),
  // The options the hook passes (`theme`) are what the palette tests assert on.
  codeToTokens: vi.fn((code: string, _options?: { lang: string; theme: string }) => codeToTokensImpl(code)),
  getTheme: () => ({ bg: 0, fg: null }),
};

const createHighlighterMock = vi.fn(() => Promise.resolve(fakeHighlighter));

vi.mock("shiki", async importOriginal => ({
  ...await importOriginal<typeof import("shiki")>(),
  createHighlighter: () => createHighlighterMock(),
}));

type ThemeModule = typeof import("../../lib/theme");
type HookModule = typeof import("./useCodeHighlighter");

/**
 * A fresh highlighter module plus the theme module from the same registry, so
 * `setThemeMode` / `setThemePalette` land on the instance the hook reads.
 */
async function loadFreshGraph(): Promise<{ useCodeHighlighter: HookModule["useCodeHighlighter"]; theme: ThemeModule }> {
  const [hook, theme] = await Promise.all([
    import("./useCodeHighlighter"),
    import("../../lib/theme"),
  ]);
  return { useCodeHighlighter: hook.useCodeHighlighter, theme };
}

beforeEach(() => {
  fakeHighlighter.loadLanguage.mockClear();
  fakeHighlighter.codeToTokens.mockClear();
  createHighlighterMock.mockClear();
  // No grammar is preloaded unless the test says so, and nothing is "already
  // loaded" from an earlier test's fake.
  loadedLanguages.length = 0;
  loadLanguageImpl = async (lang) => {
    loadedLanguages.push(lang);
  };
  codeToTokensImpl = defaultCodeToTokens;
  // The theme module reads both axes from `localStorage` when it initialises,
  // so the fresh instance only starts at the documented default (system mode
  // → light in jsdom, `futureos` palette) if storage is clear first.
  localStorage.clear();
});

describe("useCodeHighlighter", () => {
  it("reports unloaded before the highlighter resolves and loaded after", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    const h = renderHook(() => useCodeHighlighter());
    expect(h.current.isLoaded).toBe(false);
    expect(h.current.highlight("code", "ts")).toBeNull();
    await flushAsync();
    expect(h.current.isLoaded).toBe(true);
    h.unmount();
  });

  it("returns null for missing or unmapped languages", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    expect(h.current.highlight("code", undefined)).toBeNull();
    expect(h.current.highlight("code", "not-a-lang")).toBeNull();
    h.unmount();
  });

  it("kicks off a lazy grammar load and highlights once loaded", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    // Not loaded yet → null, but the grammar load starts.
    expect(h.current.highlight("const a = 1", "TypeScript")).toBeNull();
    expect(fakeHighlighter.loadLanguage).toHaveBeenCalledWith("typescript");
    await flushAsync();
    const result = h.current.highlight("const a = 1", "ts");
    expect(result).not.toBeNull();
    expect(result!.lines[0]!.tokens[0]).toEqual({ content: "const a = 1", color: "#111", fontStyle: 1 });
    // Missing color/fontStyle fall back to the theme foreground / undefined.
    expect(result!.lines[0]!.tokens[1]).toEqual({ content: "!", color: "#000000", fontStyle: undefined });
    // Non-string theme foregrounds fall back to the default.
    expect(result!.fgColor).toBe("#000000");
    h.unmount();
  });

  it.each([
    [" TSX ", "tsx"],
    ["ps1", "powershell"],
    ["dockerfile", "docker"],
    ["vue", "vue"],
    ["c++", "cpp"],
    ["shell", "shellscript"],
  ])("loads the registered grammar for %s", async (alias, canonical) => {
    const { useCodeHighlighter } = await loadFreshGraph();
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    expect(h.current.highlight("registry test", alias)).toBeNull();
    expect(fakeHighlighter.loadLanguage).toHaveBeenCalledWith(canonical);
    await flushAsync();
    expect(h.current.highlight("registry test", alias)).not.toBeNull();
    const calls = fakeHighlighter.codeToTokens.mock.calls.length;
    expect(h.current.highlight("registry test", canonical)).not.toBeNull();
    expect(fakeHighlighter.codeToTokens.mock.calls.length).toBe(calls);
    h.unmount();
  });

  it("dedupes concurrent grammar loads for the same language", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    let resolveLoad!: () => void;
    loadLanguageImpl = () => new Promise<void>((resolve) => {
      resolveLoad = resolve;
    });
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    expect(h.current.highlight("a", "rust")).toBeNull();
    expect(h.current.highlight("b", "rust")).toBeNull();
    expect(fakeHighlighter.loadLanguage).toHaveBeenCalledTimes(1);
    resolveLoad();
    await flushAsync();
    loadLanguageImpl = async (lang) => {
      loadedLanguages.push(lang);
    };
    h.unmount();
  });

  it("stays unloaded (plain text) when the grammar fails to load", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    loadLanguageImpl = () => Promise.reject(new Error("no grammar"));
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    expect(h.current.highlight("a", "go")).toBeNull();
    await flushAsync();
    // In-flight marker cleared: a second attempt re-issues the load.
    expect(h.current.highlight("a", "go")).toBeNull();
    expect(fakeHighlighter.loadLanguage).toHaveBeenCalledTimes(2);
    loadLanguageImpl = async (lang) => {
      loadedLanguages.push(lang);
    };
    h.unmount();
  });

  it("uses the syntax palette of the active app theme", async () => {
    const { useCodeHighlighter, theme } = await loadFreshGraph();
    loadedLanguages.push("json");
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    h.current.highlight("{}", "json");
    expect(fakeHighlighter.codeToTokens).toHaveBeenLastCalledWith("{}", { lang: "json", theme: "github-light" });

    act(() => theme.setThemeMode("dark"));
    h.current.highlight("{}", "json");
    // A different theme is a different cache entry, not a stale reuse.
    expect(fakeHighlighter.codeToTokens).toHaveBeenLastCalledWith("{}", { lang: "json", theme: "github-dark" });

    act(() => theme.setThemeMode("light"));
    h.unmount();
  });

  it("uses the syntax palette of the active color theme, in both renditions", async () => {
    const { useCodeHighlighter, theme } = await loadFreshGraph();
    loadedLanguages.push("json");
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();

    // Distinct sources per step: a repeated (theme, language, code) triple is
    // served from the highlight cache without tokenizing again, which would
    // leave the last call pointing at the previous theme.
    const themeFor = (code: string) => {
      expect(h.current.highlight(code, "json")).not.toBeNull();
      return fakeHighlighter.codeToTokens.mock.lastCall?.[1]?.theme;
    };

    // A theme that ships its own editor colors uses those, not a tinted
    // GitHub theme: a Nord code block belongs in Nord's syntax colors.
    act(() => theme.setThemePalette("nord"));
    expect(themeFor("{}")).toBe("nord");

    // Palettes with a light and a dark editor theme pick by rendition.
    act(() => theme.setThemePalette("catppuccin"));
    act(() => theme.setThemeMode("dark"));
    expect(themeFor("[]")).toBe("catppuccin-mocha");

    act(() => theme.setThemeMode("light"));
    expect(themeFor("1")).toBe("catppuccin-latte");

    // Back to the default palette: the GitHub pair, not the last theme's colors.
    act(() => theme.setThemePalette("futureos"));
    expect(themeFor("2")).toBe("github-light");

    act(() => theme.setThemeMode("dark"));
    expect(themeFor("3")).toBe("github-dark");

    act(() => {
      theme.setThemeMode("light");
      theme.setThemePalette("futureos");
    });
    h.unmount();
  });

  it("serves repeat highlights from the LRU cache", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    loadedLanguages.push("java");
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    expect(h.current.highlight("class A {}", "java")).not.toBeNull();
    const calls = fakeHighlighter.codeToTokens.mock.calls.length;
    expect(h.current.highlight("class A {}", "java")).not.toBeNull();
    expect(fakeHighlighter.codeToTokens.mock.calls.length).toBe(calls);
    h.unmount();
  });

  it("skips caching huge blocks", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    loadedLanguages.push("python");
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    const huge = "x".repeat(100_001);
    expect(h.current.highlight(huge, "python")).not.toBeNull();
    const calls = fakeHighlighter.codeToTokens.mock.calls.length;
    expect(h.current.highlight(huge, "python")).not.toBeNull();
    expect(fakeHighlighter.codeToTokens.mock.calls.length).toBe(calls + 1);
    h.unmount();
  });

  it("returns null when tokenization throws", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    loadedLanguages.push("sql");
    codeToTokensImpl = () => {
      throw new Error("bad grammar");
    };
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    expect(h.current.highlight("select 1", "sql")).toBeNull();
    codeToTokensImpl = defaultCodeToTokens;
    h.unmount();
  });

  it("evicts the oldest cache entry past the cap", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    loadedLanguages.push("xml");
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    for (let i = 0; i < 401; i += 1) {
      h.current.highlight(`<a>${i}</a>`, "xml");
    }
    // The first entry was evicted: re-highlighting it tokenizes again.
    const calls = fakeHighlighter.codeToTokens.mock.calls.length;
    h.current.highlight("<a>0</a>", "xml");
    expect(fakeHighlighter.codeToTokens.mock.calls.length).toBe(calls + 1);
    h.unmount();
  });

  it("reuses the cached highlighter on later mounts", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    // Warm the module the way the app does: one mount, one createHighlighter.
    const first = renderHook(() => useCodeHighlighter());
    await flushAsync();
    expect(first.current.isLoaded).toBe(true);
    first.unmount();
    expect(createHighlighterMock).toHaveBeenCalledTimes(1);

    // A second mount takes the cached fast path instead of building another.
    const second = renderHook(() => useCodeHighlighter());
    expect(second.current.isLoaded).toBe(true);
    expect(createHighlighterMock).toHaveBeenCalledTimes(1);
    second.unmount();
  });

  it("renders without a subscriber under SSR (getServerSnapshot path)", async () => {
    const { useCodeHighlighter } = await loadFreshGraph();
    function Probe() {
      const { isLoaded } = useCodeHighlighter();
      return createElement("span", null, `loaded=${isLoaded}`);
    }
    // The server snapshot is the cold-start version: SSR never awaits the
    // async highlighter, so it renders the plain-text fallback.
    expect(renderToStaticMarkup(createElement(Probe))).toBe("<span>loaded=false</span>");
  });
});
