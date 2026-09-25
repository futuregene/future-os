import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { bundledLanguagesInfo } from "shiki";
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";
import { FilePreviewOverlay } from "./FilePreviewOverlay";
import { ImagePreview } from "./ImagePreview";
import { JsonPreview } from "./JsonPreview";
import { MarkdownPreview } from "./MarkdownPreview";
import { codeLanguageForPath, imageMimeForPath, isTextReadablePath, LANGUAGE_BY_EXTENSION, LANGUAGE_BY_NAME, previewKindForPath } from "./previewKind";
import { PreviewNotice } from "./PreviewNotice";
import { TextPreview } from "./TextPreview";
import {
  PREVIEW_LOADING_DELAY_MS,
  PREVIEW_LOADING_MIN_VISIBLE_MS,
  usePreviewLoadingGate,
} from "./usePreviewLoadingGate";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(() => Promise.resolve(null));

// Fake the highlighter so text-preview assertions are deterministic: real Shiki
// tokenizes asynchronously and would only ever be ready after the assertions.
// The real module is kept for `MAX_HIGHLIGHTABLE_CODE_LENGTH`, so the bound the
// previews gate on stays the production one.
let highlighterState: {
  highlight: (code: string, language?: string) => unknown;
  isLoaded: boolean;
};

vi.mock("../markdown/useCodeHighlighter", async importOriginal => ({
  ...await importOriginal<typeof import("../markdown/useCodeHighlighter")>(),
  useCodeHighlighter: () => highlighterState,
}));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `asset:${path}`,
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
  highlighterState = { highlight: () => null, isLoaded: false };
});

afterEach(() => {
  vi.useRealTimers();
});

function mount(node: React.ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(node);
  });
  return {
    container,
    root,
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

describe("previewKind", () => {
  it("classifies by extension", () => {
    expect(previewKindForPath("/a/b.PNG")).toBe("image");
    expect(previewKindForPath("/a/b.md")).toBe("markdown");
    expect(previewKindForPath("/a/b.markdown")).toBe("markdown");
    expect(previewKindForPath("/a/b.JSON")).toBe("json");
    // A notebook is one JSON document.
    expect(previewKindForPath("/a/plan.ipynb")).toBe("json");
    expect(previewKindForPath("/a/b.pdf")).toBeNull();
    expect(previewKindForPath("/a/b")).toBeNull();
  });

  it("classifies code and config files as text", () => {
    expect(previewKindForPath("/a/b.py")).toBe("text");
    expect(previewKindForPath("/a/b.RS")).toBe("text");
    expect(previewKindForPath("/a/b.go")).toBe("text");
    expect(previewKindForPath("/a/b.tsx")).toBe("text");
    expect(previewKindForPath("/a/b.toml")).toBe("text");
    expect(previewKindForPath("/a/b.sh")).toBe("text");
    expect(previewKindForPath("/a/b.jsonl")).toBe("text");
    expect(previewKindForPath("/a/b.proto")).toBe("text");
    // Suffix-less build and lock files read as text too.
    expect(previewKindForPath("/a/Makefile")).toBe("text");
    expect(previewKindForPath("/a/Dockerfile.dev")).toBe("text");
    expect(previewKindForPath("/a/Cargo.lock")).toBe("text");
    expect(previewKindForPath("/a/.gitignore")).toBe("text");
    expect(previewKindForPath("/a/go.mod")).toBe("text");
    // Unsupported or ambiguous types keep the OS-handler path.
    expect(previewKindForPath("/a/b.bin")).toBeNull();
    expect(previewKindForPath("/a/b.h5")).toBeNull();
    expect(previewKindForPath("/a/dataset.parquet")).toBeNull();
  });

  it("detects text-readable paths for the artifact preview", () => {
    expect(isTextReadablePath("/a/b.rs")).toBe(true);
    expect(isTextReadablePath("/a/b.md")).toBe(true);
    expect(isTextReadablePath("/a/b.json")).toBe(true);
    expect(isTextReadablePath("/a/Makefile")).toBe(true);
    expect(isTextReadablePath("/a/b.pdf")).toBe(false);
    expect(isTextReadablePath("/a/b.png")).toBe(false);
  });

  it("maps extensions to a syntax grammar", () => {
    expect(codeLanguageForPath("/w/main.rs")).toBe("rust");
    expect(codeLanguageForPath("/w/App.TSX")).toBe("tsx");
    expect(codeLanguageForPath("C:\\w\\boot.asm")).toBe("asm");
    expect(codeLanguageForPath("/w/Cargo.toml")).toBe("toml");
    expect(codeLanguageForPath("/w/deploy.sh")).toBe("shellscript");
    expect(codeLanguageForPath("/w/main.tf")).toBe("hcl");
    expect(codeLanguageForPath("/w/prod.tfvars")).toBe("hcl");
    expect(codeLanguageForPath("/w/solver.m")).toBe("matlab");
    expect(codeLanguageForPath("/w/AppDelegate.mm")).toBe("objective-cpp");
    expect(codeLanguageForPath("/w/legacy.f")).toBe("fortran-fixed-form");
    expect(codeLanguageForPath("/w/modern.f90")).toBe("fortran-free-form");
    expect(codeLanguageForPath("/w/rows.jsonl")).toBe("jsonl");
    expect(codeLanguageForPath("/w/api.proto")).toBe("protobuf");
    expect(codeLanguageForPath("/w/build.mk")).toBe("make");
    expect(codeLanguageForPath("/w/CMakeLists.txt")).toBe("cmake");
    expect(codeLanguageForPath("/w/plan.ipynb")).toBe("json");
    // Suffix-less names are resolved by name, not by extension.
    expect(codeLanguageForPath("/w/Makefile")).toBe("make");
    expect(codeLanguageForPath("/w/Dockerfile.dev")).toBe("dockerfile");
    expect(codeLanguageForPath("/w/Jenkinsfile")).toBe("groovy");
    expect(codeLanguageForPath("/w/Gemfile")).toBe("ruby");
    expect(codeLanguageForPath("/w/.bashrc")).toBe("shellscript");
    expect(codeLanguageForPath("/w/.env.local")).toBe("ini");
    // Reads as text but has no grammar: the preview stays plain.
    expect(codeLanguageForPath("/w/notes.txt")).toBeNull();
    expect(codeLanguageForPath("/w/wire.h5")).toBeNull();
    expect(codeLanguageForPath("/w/Cargo.lock")).toBeNull();
    expect(codeLanguageForPath("/w/.gitignore")).toBeNull();
    expect(codeLanguageForPath("/w/README")).toBeNull();
  });

  it("maps every extension and name to a grammar the highlighter can actually load", () => {
    // A typo'd id silently degrades to plain text, which no screenshot would
    // flag; Shiki's registry is the authority the highlighter itself uses.
    const known = new Set(bundledLanguagesInfo.flatMap(language => [language.id, ...(language.aliases ?? [])]));
    expect(Object.entries({ ...LANGUAGE_BY_EXTENSION, ...LANGUAGE_BY_NAME }).filter(([, language]) => !known.has(language))).toEqual([]);
  });

  it("maps extensions to MIME types with a fallback", () => {
    expect(imageMimeForPath("/a/b.png")).toBe("image/png");
    expect(imageMimeForPath("/a/b.jpg")).toBe("image/jpeg");
    expect(imageMimeForPath("/a/b.svg")).toBe("image/svg+xml");
    expect(imageMimeForPath("/a/b.bin")).toBe("application/octet-stream");
  });
});

describe("previewNotice", () => {
  it("renders the message", () => {
    expect(renderToStaticMarkup(createElement(PreviewNotice, { message: "Loading…" }))).toContain("Loading…");
  });
});

describe("previewLoadingGate", () => {
  it("keeps a preview that finishes within 200ms free of a loading notice", () => {
    vi.useFakeTimers();
    let loading = true;
    const hook = renderHook(() => usePreviewLoadingGate(loading));

    expect(hook.current).toEqual({ showContent: false, showLoading: false });
    act(() => vi.advanceTimersByTime(PREVIEW_LOADING_DELAY_MS - 1));
    loading = false;
    hook.rerender();

    expect(hook.current).toEqual({ showContent: true, showLoading: false });
    hook.unmount();
  });

  it("holds a visible loading notice for at least 300ms", () => {
    vi.useFakeTimers();
    let loading = true;
    const hook = renderHook(() => usePreviewLoadingGate(loading));

    act(() => vi.advanceTimersByTime(PREVIEW_LOADING_DELAY_MS));
    expect(hook.current).toEqual({ showContent: false, showLoading: true });

    loading = false;
    hook.rerender();
    act(() => vi.advanceTimersByTime(PREVIEW_LOADING_MIN_VISIBLE_MS - 1));
    expect(hook.current).toEqual({ showContent: false, showLoading: true });

    act(() => vi.advanceTimersByTime(1));
    expect(hook.current).toEqual({ showContent: true, showLoading: false });
    hook.unmount();
  });

  it("moves straight to ready once the minimum visible time has elapsed", () => {
    vi.useFakeTimers();
    let loading = true;
    const hook = renderHook(() => usePreviewLoadingGate(loading));

    act(() => vi.advanceTimersByTime(PREVIEW_LOADING_DELAY_MS));
    expect(hook.current).toEqual({ showContent: false, showLoading: true });

    // The visible window has already elapsed by the time loading flips off.
    act(() => vi.advanceTimersByTime(PREVIEW_LOADING_MIN_VISIBLE_MS));
    loading = false;
    hook.rerender();

    expect(hook.current).toEqual({ showContent: true, showLoading: false });
    hook.unmount();
  });
});

describe("imagePreview", () => {
  it("keeps a fast load quiet until the image has decoded", async () => {
    invokeMock.mockResolvedValue({ path: "/w/pic.png", version: "1" });
    const { container, cleanup } = mount(createElement(ImagePreview, {
      path: "/w/pic.png",
      name: "pic.png",
      onError: vi.fn(),
    }));
    expect(container.textContent).not.toContain("Loading");
    await flushAsync();
    const img = container.querySelector("img")!;
    expect(img.getAttribute("src")).toBe("asset:/w/pic.png?v=1");
    expect(img.className).toContain("invisible");
    act(() => img.dispatchEvent(new Event("load")));
    await flushAsync();
    expect(img.className).toContain("visible");
    cleanup();
  });

  it("routes read failures to onError", async () => {
    invokeMock.mockRejectedValue(new Error("too large"));
    const onError = vi.fn();
    const { cleanup } = mount(createElement(ImagePreview, {
      path: "/w/pic.png",
      name: "pic.png",
      onError,
    }));
    await flushAsync();
    expect(onError).toHaveBeenCalledTimes(1);
    cleanup();
  });

  it("img onError also routes to onError", async () => {
    invokeMock.mockResolvedValue({ path: "/w/pic.png", version: "1" });
    const onError = vi.fn();
    const { container, cleanup } = mount(createElement(ImagePreview, {
      path: "/w/pic.png",
      name: "pic.png",
      onError,
    }));
    await flushAsync();
    act(() => {
      container.querySelector("img")!.dispatchEvent(new Event("error"));
    });
    await flushAsync();
    expect(onError).toHaveBeenCalled();
    cleanup();
  });
});

describe("markdownPreview", () => {
  it("renders the file content through the markdown renderer", async () => {
    invokeMock.mockResolvedValue({ content: "# Title\n\nbody", size: 14, truncated: false });
    const { container, cleanup } = mount(createElement(MarkdownPreview, {
      path: "/w/doc.md",
      onError: vi.fn(),
    }));
    expect(container.textContent).not.toContain("Loading");
    await flushAsync();
    expect(container.querySelector("h1")?.textContent).toBe("Title");
    cleanup();
  });

  it("reflows prose wrapped at the file's column instead of showing its line breaks", async () => {
    // The file's newlines are soft breaks, which CommonMark renders as spaces.
    invokeMock.mockResolvedValue({ content: "投影最小（1 706 tok）、回本最快\n基础指令，压缩只要 0.42 元。", size: 40, truncated: false });
    const { container, cleanup } = mount(createElement(MarkdownPreview, {
      path: "/w/doc.md",
      onError: vi.fn(),
    }));
    await flushAsync();
    expect(container.querySelector("p")?.textContent).toBe("投影最小（1 706 tok）、回本最快 基础指令，压缩只要 0.42 元。");
    cleanup();
  });

  it("routes read failures to onError", async () => {
    invokeMock.mockRejectedValue(new Error("gone"));
    const onError = vi.fn();
    const { cleanup } = mount(createElement(MarkdownPreview, { path: "/w/doc.md", onError }));
    await flushAsync();
    expect(onError).toHaveBeenCalledTimes(1);
    cleanup();
  });

  it("shows a loading notice while a slow read is in flight", async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation(() => new Promise(() => {}));
    const { container, cleanup } = mount(createElement(MarkdownPreview, {
      path: "/w/doc.md",
      onError: vi.fn(),
    }));
    act(() => vi.advanceTimersByTime(PREVIEW_LOADING_DELAY_MS));
    await flushAsync();
    expect(container.textContent).toContain("Loading preview");
    cleanup();
  });
});

describe("textPreview", () => {
  it("renders code as monospace text", async () => {
    invokeMock.mockResolvedValue({
      content: "fn main() {}\n",
      size: 13,
      truncated: false,
      validUtf8: true,
    });
    const { container, cleanup } = mount(createElement(TextPreview, {
      path: "/w/main.rs",
      onError: vi.fn(),
    }));
    expect(container.textContent).not.toContain("Loading");
    await flushAsync();
    expect(container.querySelector("pre")?.textContent).toBe("fn main() {}\n");
    expect(container.textContent).not.toContain("Large file");
    cleanup();
  });

  it("marks a truncated read", async () => {
    invokeMock.mockResolvedValue({ content: "a", size: 900_000, truncated: true, validUtf8: true });
    const { container, cleanup } = mount(createElement(TextPreview, {
      path: "/w/big.py",
      onError: vi.fn(),
    }));
    await flushAsync();
    expect(container.textContent).toContain("Large file");
    cleanup();
  });

  it("routes read failures and non-UTF-8 bytes to onError", async () => {
    const onError = vi.fn();
    invokeMock.mockRejectedValue(new Error("gone"));
    const failed = mount(createElement(TextPreview, { path: "/w/gone.rs", onError }));
    await flushAsync();
    expect(onError).toHaveBeenCalledTimes(1);
    failed.cleanup();

    // A code extension over a binary file must not render replacement chars.
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ content: "\uFFFD\uFFFD", size: 4, truncated: false, validUtf8: false });
    const binary = mount(createElement(TextPreview, { path: "/w/bin.c", onError }));
    await flushAsync();
    expect(onError).toHaveBeenCalledTimes(2);
    // Nothing was rendered for the failed read.
    expect(binary.container.querySelector("pre")).toBeNull();
    binary.cleanup();
  });

  it("renders the source plain while the highlighter or grammar is still loading", async () => {
    const source = "fn main() {}\n";
    highlighterState = { highlight: () => null, isLoaded: false };
    invokeMock.mockResolvedValue({ content: source, size: source.length, truncated: false, validUtf8: true });
    const { container, cleanup } = mount(createElement(TextPreview, { path: "/w/main.rs", onError: vi.fn() }));
    await flushAsync();
    const pre = container.querySelector("pre")!;
    expect(pre.textContent).toBe(source);
    expect(pre.querySelector("span")).toBeNull();
    cleanup();
  });

  it("colors code with the grammar for its extension, without altering the source", async () => {
    // CRLF on purpose: Shiki's token stream drops line separators, so this is
    // what proves the preview re-inserts the source's own ones.
    const source = "fn main() {\r\n    let x = 1;\r\n}\r\n";
    const highlight = vi.fn(() => ({
      fgColor: "#24292e",
      lines: source.split("\r\n").map(content => ({
        tokens: content ? [{ content, color: "#d73a49", fontStyle: 1 }] : [],
      })),
    }));
    highlighterState = { highlight, isLoaded: true };
    invokeMock.mockResolvedValue({ content: source, size: source.length, truncated: false, validUtf8: true });
    const { container, cleanup } = mount(createElement(TextPreview, { path: "/w/main.rs", onError: vi.fn() }));
    await flushAsync();
    expect(highlight).toHaveBeenCalledWith(source, "rust");
    const pre = container.querySelector("pre") as HTMLPreElement;
    expect(pre.textContent).toBe(source);
    // jsdom normalizes inline hex colors to `rgb(...)`.
    const token = pre.querySelector("span > span") as HTMLSpanElement;
    expect(token.style.color).toBe("rgb(215, 58, 73)");
    expect(token.style.fontStyle).toBe("italic");
    expect(pre.style.color).toBe("rgb(36, 41, 46)");
    cleanup();
  });

  it.each([
    ["no grammar", "/w/notes.txt"],
    ["source past the highlight bound", "/w/huge.py"],
  ])("stays plain monospace with %s", async (_case, path) => {
    const content = path.endsWith(".py") ? "#".repeat(100_001) : "just words";
    const highlight = vi.fn(() => ({
      fgColor: "#24292e",
      lines: [{ tokens: [{ content, color: "#111" }] }],
    }));
    highlighterState = { highlight, isLoaded: true };
    invokeMock.mockResolvedValue({ content, size: content.length, truncated: true, validUtf8: true });
    const { container, cleanup } = mount(createElement(TextPreview, { path, onError: vi.fn() }));
    await flushAsync();
    expect(highlight).not.toHaveBeenCalled();
    const pre = container.querySelector("pre")!;
    expect(pre.textContent).toBe(content);
    // Unchanged styling: the muted text class, no per-token spans.
    expect(pre.className).toContain("text-ink-soft");
    expect(pre.querySelector("span")).toBeNull();
    cleanup();
  });

  it("shows a loading notice while a slow read is in flight", async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation(() => new Promise(() => {}));
    const { container, cleanup } = mount(createElement(TextPreview, {
      path: "/w/main.go",
      onError: vi.fn(),
    }));
    act(() => vi.advanceTimersByTime(PREVIEW_LOADING_DELAY_MS));
    await flushAsync();
    expect(container.textContent).toContain("Loading preview");
    cleanup();
  });
});

describe("jsonPreview", () => {
  it("formats and tokenizes a valid JSON file", async () => {
    invokeMock.mockResolvedValue({
      content: "{\"id\":900719925474099312345,\"ok\":true}",
      size: 41,
      truncated: false,
      validUtf8: true,
    });
    const { container, cleanup } = mount(createElement(JsonPreview, {
      path: "/w/data.json",
      onError: vi.fn(),
    }));
    expect(container.textContent).not.toContain("Loading");
    await flushAsync();
    expect(container.textContent).toContain("900719925474099312345");
    expect(container.textContent).toContain("true");
    expect(container.querySelector(".text-accent")?.textContent).toBe("\"id\"");
    cleanup();
  });

  it("shows raw source and an error for invalid JSON", async () => {
    invokeMock.mockResolvedValue({ content: "{bad", size: 4, truncated: false, validUtf8: true });
    const { container, cleanup } = mount(createElement(JsonPreview, {
      path: "/w/bad.json",
      onError: vi.fn(),
    }));
    await flushAsync();
    expect(container.textContent).toContain("Invalid JSON");
    expect(container.textContent).toContain("{bad");
    cleanup();
  });

  it("routes read failures to onError", async () => {
    invokeMock.mockRejectedValue(new Error("unreadable"));
    const onError = vi.fn();
    const { cleanup } = mount(createElement(JsonPreview, { path: "/w/bad.json", onError }));
    await flushAsync();
    expect(onError).toHaveBeenCalledTimes(1);
    cleanup();
  });

  it("shows a loading notice while a slow read is in flight", async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation(() => new Promise(() => {}));
    const { container, cleanup } = mount(createElement(JsonPreview, {
      path: "/w/data.json",
      onError: vi.fn(),
    }));
    act(() => vi.advanceTimersByTime(PREVIEW_LOADING_DELAY_MS));
    await flushAsync();
    expect(container.textContent).toContain("Loading preview");
    cleanup();
  });

  it("styles string tokens distinctly from keys", async () => {
    invokeMock.mockResolvedValue({
      content: "{\"name\":\"hello\"}",
      size: 16,
      truncated: false,
      validUtf8: true,
    });
    const { container, cleanup } = mount(createElement(JsonPreview, {
      path: "/w/s.json",
      onError: vi.fn(),
    }));
    await flushAsync();
    expect(container.querySelector(".text-success")?.textContent).toBe("\"hello\"");
    cleanup();
  });

  it("recomputes the visible window when the JSON scrolls", async () => {
    invokeMock.mockResolvedValue({
      content: JSON.stringify({ a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8 }),
      size: 80,
      truncated: false,
      validUtf8: true,
    });
    const { container, cleanup } = mount(createElement(JsonPreview, {
      path: "/w/data.json",
      onError: vi.fn(),
    }));
    await flushAsync();
    const scroller = container.querySelector(".overflow-auto");
    expect(scroller).toBeTruthy();
    act(() => {
      scroller!.dispatchEvent(new Event("scroll"));
    });
    cleanup();
  });
});

describe("filePreviewOverlay", () => {
  it("renders nothing when closed", () => {
    const { container, cleanup } = mount(createElement(FilePreviewOverlay, {
      path: "/w/a.png",
      name: "a.png",
      kind: "image",
      open: false,
      onClose: vi.fn(),
    }));
    expect(container.innerHTML).toBe("");
    cleanup();
  });

  it("renders the image preview with a close button", async () => {
    invokeMock.mockResolvedValue({ path: "/w/a.png", version: "1" });
    const onClose = vi.fn();
    const { container, cleanup } = mount(createElement(FilePreviewOverlay, {
      path: "/w/a.png",
      name: "a.png",
      kind: "image",
      open: true,
      onClose,
    }));
    await flushAsync();
    const image = container.querySelector("img")!;
    act(() => image.dispatchEvent(new Event("load")));
    await flushAsync();
    expect(image.className).toContain("visible");
    const closeButton = container.querySelector("button[aria-label]")!;
    act(() => {
      closeButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalled();
    cleanup();
  });

  it("renders the markdown preview", async () => {
    invokeMock.mockResolvedValue({ content: "hello", size: 5, truncated: false });
    const { container, cleanup } = mount(createElement(FilePreviewOverlay, {
      path: "/w/a.md",
      name: "a.md",
      kind: "markdown",
      open: true,
      onClose: vi.fn(),
    }));
    await flushAsync();
    expect(container.textContent).toContain("hello");
    cleanup();
  });

  it("renders the JSON preview", async () => {
    invokeMock.mockResolvedValue({ content: "{\"ok\":true}", size: 11, truncated: false, validUtf8: true });
    const { container, cleanup } = mount(createElement(FilePreviewOverlay, {
      path: "/w/a.json",
      name: "a.json",
      kind: "json",
      open: true,
      onClose: vi.fn(),
    }));
    await flushAsync();
    expect(container.textContent).toContain("\"ok\"");
    cleanup();
  });

  it("renders the text preview", async () => {
    invokeMock.mockResolvedValue({ content: "print('hi')", size: 11, truncated: false, validUtf8: true });
    const { container, cleanup } = mount(createElement(FilePreviewOverlay, {
      path: "/w/train.py",
      name: "train.py",
      kind: "text",
      open: true,
      onClose: vi.fn(),
    }));
    await flushAsync();
    expect(container.querySelector("pre")?.textContent).toContain("print('hi')");
    cleanup();
  });

  it("toasts, closes, and falls back to the OS handler on preview failure", async () => {
    invokeMock.mockRejectedValue(new Error("unreadable"));
    const events: CustomEvent[] = [];
    window.addEventListener("futureos:toast", e => events.push(e as CustomEvent));
    const onClose = vi.fn();
    const onOpenExternal = vi.fn();
    const { cleanup } = mount(createElement(FilePreviewOverlay, {
      path: "/w/a.md",
      name: "a.md",
      kind: "markdown",
      open: true,
      onClose,
      onOpenExternal,
    }));
    await flushAsync();
    await flushAsync();
    expect(events.length).toBe(1);
    expect(onClose).toHaveBeenCalled();
    expect(onOpenExternal).toHaveBeenCalled();
    cleanup();
  });

  it("uses the unavailableMessage override when given", async () => {
    invokeMock.mockRejectedValue(new Error("unreadable"));
    const events: CustomEvent[] = [];
    window.addEventListener("futureos:toast", e => events.push(e as CustomEvent));
    const { cleanup } = mount(createElement(FilePreviewOverlay, {
      path: "/w/a.md",
      name: "a.md",
      kind: "markdown",
      open: true,
      onClose: vi.fn(),
      unavailableMessage: "original gone",
    }));
    await flushAsync();
    await flushAsync();
    expect(events[0]?.detail.message).toBe("original gone");
    cleanup();
  });
});
