import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { Badge } from "./Badge";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { MenuPanel } from "./MenuPanel";
import { Overlay } from "./Overlay";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe("badge", () => {
  it("renders each tone with its styling and children", () => {
    const html = renderToStaticMarkup(
      createElement(Badge, { tone: "success", className: "extra" }, "done"),
    );
    expect(html).toContain("bg-success-soft");
    expect(html).toContain("extra");
    expect(html).toContain("done");
  });
});

describe("button", () => {
  it("renders variant/size classes with a left icon", () => {
    const html = renderToStaticMarkup(
      createElement(Button, {
        "variant": "primary",
        "size": "xs",
        "leftIcon": createElement("span", { "data-icon": true }),
        "aria-label": "go",
      }, "Go"),
    );
    expect(html).toContain("bg-accent");
    expect(html).toContain("h-7");
    expect(html).toContain("data-icon");
    expect(html).toContain("aria-label=\"go\"");
  });
});

describe("iconButton", () => {
  it("renders the active state and aria label", () => {
    const html = renderToStaticMarkup(
      createElement(IconButton, { icon: createElement("i"), label: "Refresh", active: true }),
    );
    expect(html).toContain("aria-label=\"Refresh\"");
    expect(html).toContain("bg-accent-soft");
  });
});

describe("menuPanel", () => {
  it("renders the panel surface with style and children", () => {
    const html = renderToStaticMarkup(
      <MenuPanel className="w-40" style={{ top: 4 }}>menu</MenuPanel>,
    );
    expect(html).toContain("shadow-panel");
    expect(html).toContain("w-40");
    expect(html).toContain("top:4px");
    expect(html).toContain("menu");
  });
});

describe("overlay", () => {
  function mountOverlay(open: boolean, onClose: () => void) {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(<Overlay onClose={onClose} open={open}><div>body</div></Overlay>);
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
      root.render(
        <div>
          <Overlay onClose={parentClose} open>{null}</Overlay>
          <Overlay onClose={childClose} open>{null}</Overlay>
        </div>,
      );
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
