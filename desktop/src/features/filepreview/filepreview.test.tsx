import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../test/renderHook";
import { FilePreviewOverlay } from "./FilePreviewOverlay";
import { ImagePreview } from "./ImagePreview";
import { MarkdownPreview } from "./MarkdownPreview";
import { imageMimeForPath, previewKindForPath } from "./previewKind";
import { PreviewNotice } from "./PreviewNotice";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(() => Promise.resolve(null));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
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

describe("imagePreview", () => {
  it("shows a loading notice, then the image", async () => {
    invokeMock.mockResolvedValue("QUJD");
    const { container, cleanup } = mount(createElement(ImagePreview, {
      path: "/w/pic.png",
      name: "pic.png",
      onError: vi.fn(),
    }));
    expect(container.textContent).toContain("Loading");
    await flushAsync();
    const img = container.querySelector("img")!;
    expect(img.getAttribute("src")).toBe("data:image/png;base64,QUJD");
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
    invokeMock.mockResolvedValue("QUJD");
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
    invokeMock.mockResolvedValue("QUJD");
    const onClose = vi.fn();
    const { container, cleanup } = mount(createElement(FilePreviewOverlay, {
      path: "/w/a.png",
      name: "a.png",
      kind: "image",
      open: true,
      onClose,
    }));
    await flushAsync();
    expect(container.querySelector("img")).not.toBeNull();
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
