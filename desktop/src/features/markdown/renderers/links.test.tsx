// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { StoredFile } from "../../../integrations/storage/types";
import { PreviewMarkdownContext } from "../PreviewMarkdownContext";
import { FileLink } from "./FileLink";
import { LinkContextMenu } from "./LinkContextMenu";
import { SafeImage, SafeLink } from "./SafeLink";
import { useLinkContextMenu } from "./useLinkContextMenu";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(() => Promise.resolve(null));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

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

function click(el: Element) {
  act(() => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function rightClick(el: Element, coords: { clientX: number; clientY: number } = { clientX: 10, clientY: 10 }) {
  act(() => {
    el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, ...coords }));
  });
}

beforeEach(() => {
  invokeMock.mockClear();
});

describe("SafeLink", () => {
  it("renders inert text for disallowed protocols", () => {
    const html = renderToStaticMarkup(createElement(SafeLink, { href: "javascript:alert(1)" }, "x"));
    expect(html).toContain("<span");
    expect(html).not.toContain("<a");
  });

  it("renders inert text for malformed URLs", () => {
    const html = renderToStaticMarkup(createElement(SafeLink, { href: "not a url" }, "x"));
    expect(html).toContain("<span");
  });

  it("opens the URL in the system handler on click", () => {
    const { container, cleanup } = mount(createElement(SafeLink, { href: "https://example.com" }, "site"));
    click(container.querySelector("a")!);
    expect(invokeMock).toHaveBeenCalledWith("open_external_url", { url: "https://example.com" });
    cleanup();
  });

  it("opens the context menu on right click and runs menu actions", async () => {
    const exec = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, "execCommand", { value: exec, configurable: true });
    const { container, cleanup } = mount(createElement(SafeLink, { href: "https://example.com" }, "site"));
    rightClick(container.querySelector("a")!);
    const menuButtons = [...document.querySelectorAll(".fixed button")];
    expect(menuButtons.length).toBe(2);
    // visit
    click(menuButtons[0]);
    expect(invokeMock).toHaveBeenCalledWith("open_external_url", { url: "https://example.com" });
    // menu closed after selection
    expect(document.querySelectorAll(".fixed button").length).toBe(0);
    // copy link
    rightClick(container.querySelector("a")!);
    click([...document.querySelectorAll(".fixed button")][1]);
    expect(exec).toHaveBeenCalledWith("copy");
    cleanup();
  });

  it("suppresses the custom menu in preview mode", () => {
    const { container, cleanup } = mount(createElement(
      PreviewMarkdownContext.Provider,
      { value: { basePath: "/w/doc.md" } },
      createElement(SafeLink, { href: "https://example.com" }, "site"),
    ));
    rightClick(container.querySelector("a")!);
    expect(document.querySelectorAll(".fixed button").length).toBe(0);
    cleanup();
  });
});

describe("SafeImage", () => {
  it("renders the fallback chip for disallowed protocols", () => {
    const html = renderToStaticMarkup(createElement(SafeImage, { alt: "pic", src: "data:image/png;base64,x" }));
    expect(html).toContain("pic");
    expect(html).not.toContain("<img");
  });

  it("renders the img and falls back on error", () => {
    const { container, cleanup } = mount(createElement(SafeImage, { alt: "", src: "https://x/y.png", title: "t" }));
    const img = container.querySelector("img")!;
    expect(img.getAttribute("src")).toBe("https://x/y.png");
    act(() => {
      img.dispatchEvent(new Event("error"));
    });
    expect(container.querySelector("img")).toBeNull();
    // alt empty → localized "unavailable" label
    expect(container.textContent).toContain("Image unavailable");
    cleanup();
  });
});

describe("LinkContextMenu", () => {
  function controller(position: { x: number; y: number } | null) {
    return { close: vi.fn(), layerRef: { current: null }, position };
  }

  it("renders nothing without a position", () => {
    const html = renderToStaticMarkup(createElement(LinkContextMenu, {
      controller: controller(null),
      items: [{ label: "a", onSelect: () => {} }],
    }));
    expect(html).toBe("");
  });

  it("renders dividers and danger styling, and selects items", () => {
    const onSelect = vi.fn();
    const close = vi.fn();
    const { cleanup } = mount(createElement(LinkContextMenu, {
      controller: { close, layerRef: { current: null }, position: { x: 5, y: 5 } },
      items: [
        { label: "safe", onSelect },
        { divider: true, danger: true, label: "del", onSelect },
      ],
    }));
    const buttons = [...document.querySelectorAll("button")];
    expect(buttons.length).toBe(2);
    expect(document.querySelector(".border-t")).not.toBeNull();
    expect(buttons[1].className).toContain("text-danger");
    click(buttons[0]);
    expect(close).toHaveBeenCalled();
    expect(onSelect).toHaveBeenCalled();
    cleanup();
  });

  it("clamps the menu inside the viewport once measured", () => {
    const rect = {
      width: 200, height: 100, top: 0, left: 0, right: 200, bottom: 100, x: 0, y: 0,
      toJSON: () => ({}),
    } as DOMRect;
    const spy = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(rect);
    const { cleanup } = mount(createElement(LinkContextMenu, {
      controller: { close: vi.fn(), layerRef: { current: null }, position: { x: 99999, y: -50 } },
      items: [{ label: "a", onSelect: () => {} }],
    }));
    const panel = document.querySelector(".fixed") as HTMLElement;
    const maxX = window.innerWidth - 8 - 200;
    expect(panel.style.left).toBe(`${maxX}px`);
    expect(panel.style.top).toBe("8px");
    spy.mockRestore();
    cleanup();
  });

  it("uses the raw cursor position when the layer ref is not attached", () => {
    // React assigns ref.current during commit; a write-swallowing ref object
    // keeps it null so the layout effect takes the fallback branch.
    const layerRef = Object.defineProperty({}, "current", {
      get: () => null,
      set: () => {},
    }) as React.RefObject<HTMLDivElement | null>;
    const { cleanup } = mount(createElement(LinkContextMenu, {
      controller: { close: vi.fn(), layerRef, position: { x: 12, y: 34 } },
      items: [{ label: "a", onSelect: () => {} }],
    }));
    const panel = document.querySelector(".fixed") as HTMLElement;
    expect(panel.style.left).toBe("12px");
    expect(panel.style.top).toBe("34px");
    cleanup();
  });
});

describe("useLinkContextMenu", () => {
  it("opens at the cursor and closes on demand", () => {
    let controller!: ReturnType<typeof useLinkContextMenu>;
    function Probe() {
      controller = useLinkContextMenu();
      return null;
    }
    const { cleanup } = mount(createElement(Probe));
    expect(controller.position).toBeNull();
    act(() => {
      controller.open({ preventDefault: vi.fn(), clientX: 3, clientY: 4 } as unknown as React.MouseEvent<HTMLElement>);
    });
    expect(controller.position).toEqual({ x: 3, y: 4 });
    act(() => {
      controller.close();
    });
    expect(controller.position).toBeNull();
    cleanup();
  });
});

describe("FileLink", () => {
  const file: StoredFile = { path: "/w/src/a.bin", name: "a.bin", relativePath: "src/a.bin", insideWorkspace: true };

  it("shows the workspace-relative path and opens with the OS handler on click", () => {
    const { container, cleanup } = mount(createElement(FileLink, { file }));
    const anchor = container.querySelector("a")!;
    expect(anchor.textContent).toBe("src/a.bin");
    expect(anchor.getAttribute("href")).toBe("file:///w/src/a.bin");
    click(anchor);
    expect(invokeMock).toHaveBeenCalledWith("open_path", { path: "/w/src/a.bin" });
    cleanup();
  });

  it("shows the full path for files outside the workspace", () => {
    const outside: StoredFile = { path: "/tmp/x.bin", name: "x.bin", insideWorkspace: false };
    const { container, cleanup } = mount(createElement(FileLink, { file: outside }));
    expect(container.querySelector("a")!.textContent).toBe("/tmp/x.bin");
    cleanup();
  });

  it("normalizes Windows paths in the href", () => {
    const win: StoredFile = { path: "C:\\docs\\x.bin", name: "x.bin", insideWorkspace: false };
    const { container, cleanup } = mount(createElement(FileLink, { file: win }));
    expect(container.querySelector("a")!.getAttribute("href")).toBe("file:///C:/docs/x.bin");
    cleanup();
  });

  it("toasts when the OS handler reports the file missing", async () => {
    invokeMock.mockRejectedValueOnce(new Error("no such file"));
    const events: CustomEvent[] = [];
    window.addEventListener("futureos:toast", e => events.push(e as CustomEvent));
    const { container, cleanup } = mount(createElement(FileLink, { file }));
    click(container.querySelector("a")!);
    await act(async () => {
      await Promise.resolve();
    });
    expect(events).toHaveLength(1);
    expect(events[0].detail.tone).toBe("error");
    cleanup();
  });

  it("offers preview / copy actions in the context menu", async () => {
    const exec = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, "execCommand", { value: exec, configurable: true });
    const md: StoredFile = { path: "/w/doc.md", name: "doc.md", relativePath: "doc.md", insideWorkspace: true };
    const { container, cleanup } = mount(createElement(FileLink, { file: md }));
    rightClick(container.querySelector("a")!);
    const labels = [...document.querySelectorAll(".fixed button")].map(b => b.textContent);
    expect(labels.length).toBe(5); // preview, copy path, copy relative, copy name, open
    cleanup();
  });

  it("left-click opens the in-app preview for previewable types", () => {
    const md: StoredFile = { path: "/w/doc.md", name: "doc.md", relativePath: "doc.md", insideWorkspace: true };
    const { container, cleanup } = mount(createElement(FileLink, { file: md }));
    click(container.querySelector("a")!);
    // Overlay mounted (open=true) — backdrop button present.
    expect(document.querySelector(".fixed.inset-0")).not.toBeNull();
    // Close via Escape.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(document.querySelector(".fixed.inset-0")).toBeNull();
    cleanup();
  });

  it("preview mode opens every target with the OS handler and has no menu", () => {
    const md: StoredFile = { path: "/w/doc.md", name: "doc.md", relativePath: "doc.md", insideWorkspace: true };
    const { container, cleanup } = mount(createElement(
      PreviewMarkdownContext.Provider,
      { value: { basePath: "/w/other.md" } },
      createElement(FileLink, { file: md }),
    ));
    click(container.querySelector("a")!);
    expect(invokeMock).toHaveBeenCalledWith("open_path", { path: "/w/doc.md" });
    expect(document.querySelector(".fixed.inset-0")).toBeNull();
    rightClick(container.querySelector("a")!);
    expect(document.querySelectorAll(".fixed button").length).toBe(0);
    cleanup();
  });
});
