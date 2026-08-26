import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";
import { FilePreviewOverlay } from "./FilePreviewOverlay";
import { ImagePreview } from "./ImagePreview";
import { JsonPreview } from "./JsonPreview";
import { MarkdownPreview } from "./MarkdownPreview";
import { imageMimeForPath, previewKindForPath } from "./previewKind";
import { PreviewNotice } from "./PreviewNotice";
import {
  PREVIEW_LOADING_DELAY_MS,
  PREVIEW_LOADING_MIN_VISIBLE_MS,
  usePreviewLoadingGate,
} from "./usePreviewLoadingGate";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(() => Promise.resolve(null));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `asset:${path}`,
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
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
    expect(previewKindForPath("/a/b.jsonl")).toBeNull();
    expect(previewKindForPath("/a/b.pdf")).toBeNull();
    expect(previewKindForPath("/a/b")).toBeNull();
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
