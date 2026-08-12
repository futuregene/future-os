// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { act } from "react";
import { flushAsync, renderHook } from "../../test/renderHook";

// A fully controllable fake Shiki highlighter.
const loadedLanguages: string[] = [];
let loadLanguageImpl: (lang: string) => Promise<void> = async (lang) => {
  loadedLanguages.push(lang);
};
let codeToTokensImpl: (code: string) => { tokens: Array<Array<Record<string, unknown>>> } = code => ({
  tokens: [[{ content: code, color: "#111", fontStyle: 1 }, { content: "!", fontStyle: undefined, color: undefined }]],
});

const fakeHighlighter = {
  getLoadedLanguages: () => [...loadedLanguages],
  loadLanguage: vi.fn((lang: string) => loadLanguageImpl(lang)),
  codeToTokens: vi.fn((code: string) => codeToTokensImpl(code)),
  getTheme: () => ({ bg: 0, fg: null }),
};

vi.mock("shiki", () => ({
  createHighlighter: () => Promise.resolve(fakeHighlighter),
}));

import { useCodeHighlighter } from "./useCodeHighlighter";

beforeEach(() => {
  fakeHighlighter.loadLanguage.mockClear();
  fakeHighlighter.codeToTokens.mockClear();
});

describe("useCodeHighlighter", () => {
  it("reports unloaded before the highlighter resolves and loaded after", async () => {
    const h = renderHook(() => useCodeHighlighter());
    expect(h.current.isLoaded).toBe(false);
    expect(h.current.highlight("code", "ts")).toBeNull();
    await flushAsync();
    expect(h.current.isLoaded).toBe(true);
    h.unmount();
  });

  it("returns null for missing or unmapped languages", async () => {
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    expect(h.current.highlight("code")).toBeNull();
    expect(h.current.highlight("code", "not-a-lang")).toBeNull();
    h.unmount();
  });

  it("kicks off a lazy grammar load and highlights once loaded", async () => {
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    // Not loaded yet → null, but the grammar load starts.
    expect(h.current.highlight("const a = 1", "TypeScript")).toBeNull();
    expect(fakeHighlighter.loadLanguage).toHaveBeenCalledWith("typescript");
    await flushAsync();
    const result = h.current.highlight("const a = 1", "ts");
    expect(result).not.toBeNull();
    expect(result!.lines[0].tokens[0]).toEqual({ content: "const a = 1", color: "#111", fontStyle: 1 });
    // Missing color/fontStyle fall back to the theme foreground / undefined.
    expect(result!.lines[0].tokens[1]).toEqual({ content: "!", color: "#000000", fontStyle: undefined });
    // Non-string theme colors fall back to defaults.
    expect(result!.bgColor).toBe("#ffffff");
    expect(result!.fgColor).toBe("#000000");
    h.unmount();
  });

  it("dedupes concurrent grammar loads for the same language", async () => {
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

  it("serves repeat highlights from the LRU cache", async () => {
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
    loadedLanguages.push("sql");
    codeToTokensImpl = () => {
      throw new Error("bad grammar");
    };
    const h = renderHook(() => useCodeHighlighter());
    await flushAsync();
    expect(h.current.highlight("select 1", "sql")).toBeNull();
    codeToTokensImpl = code => ({
      tokens: [[{ content: code, color: "#111", fontStyle: 1 }]],
    });
    h.unmount();
  });

  it("evicts the oldest cache entry past the cap", async () => {
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

  it("renders without a subscriber under SSR (getServerSnapshot path)", () => {
    function Probe() {
      const { isLoaded } = useCodeHighlighter();
      return createElement("span", null, `loaded=${isLoaded}`);
    }
    // Module-level highlighter state leaks between tests in this file, so only
    // assert SSR rendering works — the point is getServerSnapshot runs.
    expect(renderToStaticMarkup(createElement(Probe))).toMatch(/<span>loaded=(true|false)<\/span>/);
  });
});
