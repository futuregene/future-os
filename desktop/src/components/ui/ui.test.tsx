// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { Badge } from "./Badge";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { MenuPanel } from "./MenuPanel";
import { Overlay } from "./Overlay";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe("Badge", () => {
  it("renders each tone with its styling and children", () => {
    const html = renderToStaticMarkup(
      createElement(Badge, { tone: "success", className: "extra" }, "done"),
    );
    expect(html).toContain("bg-success-soft");
    expect(html).toContain("extra");
    expect(html).toContain("done");
  });
});

describe("Button", () => {
  it("renders variant/size classes with a left icon", () => {
    const html = renderToStaticMarkup(
      createElement(Button, {
        variant: "primary",
        size: "xs",
        leftIcon: createElement("span", { "data-icon": true }),
        "aria-label": "go",
      }, "Go"),
    );
    expect(html).toContain("bg-accent");
    expect(html).toContain("h-7");
    expect(html).toContain("data-icon");
    expect(html).toContain("aria-label=\"go\"");
  });
});

describe("IconButton", () => {
  it("renders the active state and aria label", () => {
    const html = renderToStaticMarkup(
      createElement(IconButton, { icon: createElement("i"), label: "Refresh", active: true }),
    );
    expect(html).toContain("aria-label=\"Refresh\"");
    expect(html).toContain("bg-accent-soft");
  });
});

describe("MenuPanel", () => {
  it("renders the panel surface with style and children", () => {
    const html = renderToStaticMarkup(
      createElement(MenuPanel, { className: "w-40", style: { top: 4 } }, "menu"),
    );
    expect(html).toContain("shadow-panel");
    expect(html).toContain("w-40");
    expect(html).toContain("top:4px");
    expect(html).toContain("menu");
  });
});

describe("Overlay", () => {
  function mountOverlay(open: boolean, onClose: () => void) {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(createElement(Overlay, { open, onClose }, createElement("div", null, "body")));
    });
    return { container, root };
  }

  it("renders nothing when closed", () => {
    const { container, root } = mountOverlay(false, vi.fn());
    expect(container.innerHTML).toBe("");
    act(() => root.unmount());
  });

  it("closes on backdrop click", () => {
    const onClose = vi.fn();
    const { container, root } = mountOverlay(true, onClose);
    const backdrop = container.querySelector("button")!;
    act(() => {
      backdrop.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
    act(() => root.unmount());
  });

  it("closes on Escape only when topmost", () => {
    const parentClose = vi.fn();
    const childClose = vi.fn();
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(createElement("div", null,
        createElement(Overlay, { open: true, onClose: parentClose }),
        createElement(Overlay, { open: true, onClose: childClose }),
      ));
    });
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    // Only the topmost (last-mounted) overlay handles Escape.
    expect(childClose).toHaveBeenCalledTimes(1);
    expect(parentClose).not.toHaveBeenCalled();
    // Non-Escape keys are ignored.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    });
    expect(childClose).toHaveBeenCalledTimes(1);
    act(() => root.unmount());
    container.remove();
  });
});
