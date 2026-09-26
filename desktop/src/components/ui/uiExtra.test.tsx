// @vitest-environment jsdom
import type { ReactNode } from "react";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { CopyablePre } from "./CopyablePre";
import { CopyButton } from "./CopyButton";
import { Dialog } from "./Dialog";
import { EmptyState } from "./EmptyState";
import { Field } from "./Field";
import { FileTypeIcon } from "./FileTypeIcon";
import { hasOpenOverlay, useOverlayLayer } from "./overlayStack";
import { Select } from "./Select";
import { SelectMenu, SelectMenuItem } from "./SelectMenu";
import { Switch } from "./Switch";
import { TextInput } from "./TextInput";

const copyText = vi.hoisted(() => vi.fn(async () => {}));
vi.mock("../../lib/clipboard", () => ({ copyText }));

function mount(node: ReactNode) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    rerender: (next: ReactNode) => act(() => root.render(next)),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

afterEach(() => {
  copyText.mockClear();
});

describe("text input", () => {
  it("turns off autocorrect/autocapitalize/spellcheck by default", () => {
    // renderToStaticMarkup emits the React prop names verbatim, so the
    // attributes are asserted in camelCase here; the browser-cased attributes
    // on the mounted element are covered by the ref test below.
    const html = renderToStaticMarkup(createElement(TextInput, { placeholder: "id" }));
    expect(html).toContain("autoCapitalize=\"none\"");
    expect(html).toContain("autoCorrect=\"off\"");
    expect(html).toContain("spellCheck=\"false\"");
    expect(html).toContain("placeholder=\"id\"");
  });

  it("lets a call site opt back in and forwards the ref to the input element", () => {
    const html = renderToStaticMarkup(createElement(TextInput, {
      "spellCheck": true,
      "autoCapitalize": "sentences",
      "aria-label": "name",
    }));
    expect(html).toContain("spellCheck=\"true\"");
    expect(html).toContain("autoCapitalize=\"sentences\"");

    // A real mount proves the ref lands on the <input> element itself.
    const holder: { input: HTMLInputElement | null } = { input: null };
    const view = mount(createElement(TextInput, {
      ref: (node: HTMLInputElement | null) => {
        holder.input = node;
      },
    }));
    expect(holder.input).toBeInstanceOf(HTMLInputElement);
    expect(holder.input!.getAttribute("autocapitalize")).toBe("none");
    expect(holder.input!.getAttribute("spellcheck")).toBe("false");
    view.unmount();
  });
});

describe("select", () => {
  it("renders the requested size and wrapper classes", () => {
    const html = renderToStaticMarkup(
      createElement(Select, { "size": "sm", "wrapperClassName": "w-28", "aria-label": "lang", "value": "en", "onChange": () => {} }, createElement("option", { value: "en" }, "English"), createElement("option", { value: "zh" }, "中文")),
    );
    expect(html).toContain("h-8");
    expect(html).toContain("w-28");
    expect(html).toContain("English");
    expect(html).toContain("中文");
    expect(html).toContain("lucide-chevron-down");
  });

  it("defaults to the medium height", () => {
    const html = renderToStaticMarkup(createElement(Select, { "aria-label": "x" }));
    expect(html).toContain("h-9");
  });
});

describe("switch", () => {
  it("reports the toggled value on click and reflects the checked state", () => {
    const onChange = vi.fn();
    const view = mount(createElement(Switch, { checked: false, label: "bells", onChange }));
    const button = view.container.querySelector("button")!;
    expect(button.getAttribute("aria-checked")).toBe("false");
    expect(button.getAttribute("aria-label")).toBe("bells");
    act(() => button.click());
    expect(onChange).toHaveBeenCalledWith(true);

    view.rerender(createElement(Switch, { checked: true, onChange }));
    expect(view.container.querySelector("button")!.getAttribute("aria-checked")).toBe("true");
    act(() => view.container.querySelector("button")!.click());
    expect(onChange).toHaveBeenLastCalledWith(false);
    view.unmount();
  });

  it("does not fire while disabled", () => {
    const onChange = vi.fn();
    const view = mount(createElement(Switch, { checked: false, disabled: true, onChange }));
    const button = view.container.querySelector("button")!;
    expect(button.disabled).toBe(true);
    act(() => button.click());
    expect(onChange).not.toHaveBeenCalled();
    view.unmount();
  });
});

describe("dialog", () => {
  it("renders title, description and footer with a labelled dialog role", () => {
    const html = renderToStaticMarkup(createElement(Dialog, {
      onClose: () => {},
      open: true,
      title: "Delete thread",
      description: "This cannot be undone",
      footer: createElement("button", { type: "button" }, "Confirm"),
      children: createElement("p", null, "body"),
    }));
    expect(html).toContain("role=\"dialog\"");
    expect(html).toContain("aria-modal=\"true\"");
    expect(html).toContain("Delete thread");
    expect(html).toContain("This cannot be undone");
    expect(html).toContain("Confirm");
    expect(html).toContain("body");
    expect(html).toMatch(/aria-labelledby="[^"]+"/);
  });

  it("renders nothing while closed and closes on Escape", () => {
    const onClose = vi.fn();
    const view = mount(createElement(Dialog, { children: "x", onClose, open: false, title: "Hidden" }));
    expect(view.container.textContent).toBe("");
    view.rerender(createElement(Dialog, { children: "x", onClose, open: true, title: "Shown" }));
    expect(view.container.textContent).toContain("Shown");
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
    view.unmount();
  });
});

describe("field and empty state", () => {
  it("wraps children in a label with the given label text", () => {
    const html = renderToStaticMarkup(createElement(Field, { children: createElement("input", { "aria-label": "inner" }), label: "Name" }));
    expect(html.startsWith("<label")).toBe(true);
    expect(html).toContain("Name");
    expect(html).toContain("aria-label=\"inner\"");
  });

  it("renders the detail line only when provided", () => {
    const withDetail = renderToStaticMarkup(createElement(EmptyState, { title: "No runs", detail: "Try again later" }));
    expect(withDetail).toContain("No runs");
    expect(withDetail).toContain("Try again later");
    const without = renderToStaticMarkup(createElement(EmptyState, { title: "No runs", className: "mt-4" }));
    expect(without).toContain("No runs");
    expect(without).toContain("mt-4");
    expect(without).not.toContain("Try again later");
  });
});

describe("file type icon", () => {
  it.each([
    ["folder", "lucide-folder"],
    ["image", "lucide-file-image"],
    ["pdf", "lucide-book-text"],
    ["markdown", "lucide-file-down"],
    ["html", "lucide-file-code"],
    ["archive", "lucide-file-archive"],
    ["shell", "lucide-file-terminal"],
    ["code", "lucide-file-braces"],
    ["text", "lucide-file-text"],
  ] as const)("maps %s to its glyph", (kind, cls) => {
    const view = mount(createElement(FileTypeIcon, { kind, className: "size-4" }));
    const svg = view.container.querySelector("svg")!;
    expect(svg.classList.contains(cls)).toBe(true);
    expect(svg.classList.contains("size-4")).toBe(true);
    view.unmount();
  });
});

describe("copy button and copyable pre", () => {
  it("shows the clipboard glyph until copied, then the check mark", () => {
    const onCopy = vi.fn();
    const view = mount(createElement(CopyButton, { copied: false, onCopy }));
    const button = view.container.querySelector("button")!;
    expect(button.getAttribute("aria-label")).toBe("Copy");
    expect(button.getAttribute("title")).toBe("Copy");
    expect(button.querySelector("svg")!.classList.contains("lucide-clipboard")).toBe(true);
    act(() => button.click());
    expect(onCopy).toHaveBeenCalledTimes(1);

    view.rerender(createElement(CopyButton, { copied: true, label: "Copy path", onCopy }));
    expect(view.container.querySelector("button")!.getAttribute("aria-label")).toBe("Copy path");
    expect(view.container.querySelector("svg")!.classList.contains("lucide-check")).toBe(true);
    view.unmount();
  });

  it("copies the pre's text and flashes the copied state", async () => {
    const view = mount(createElement(CopyablePre, { maxHeightClassName: "max-h-40", text: "npm run test" }));
    expect(view.container.querySelector("pre")!.textContent).toBe("npm run test");
    const button = view.container.querySelector("button")!;
    await act(async () => {
      button.click();
      await Promise.resolve();
    });
    expect(copyText).toHaveBeenCalledWith("npm run test");
    expect(view.container.querySelector("svg")!.classList.contains("lucide-check")).toBe(true);
    view.unmount();
  });

  it("switches the pre to the shrink-to-fit layout when fill is set", () => {
    const view = mount(createElement(CopyablePre, { fill: true, maxHeightClassName: "max-h-40", text: "x" }));
    const root = view.container.firstElementChild!;
    expect(root.classList.contains("flex")).toBe(true);
    expect(root.classList.contains("min-h-0")).toBe(true);
    expect(view.container.querySelector("pre")!.classList.contains("min-h-0")).toBe(true);
    view.unmount();
  });

  it("toasts instead of flashing copied when the clipboard write fails", async () => {
    copyText.mockRejectedValueOnce(new Error("denied"));
    const view = mount(createElement(CopyablePre, { maxHeightClassName: "max-h-40", text: "x" }));
    await act(async () => {
      view.container.querySelector("button")!.click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(copyText).toHaveBeenCalledWith("x");
    expect(view.container.querySelector("svg")!.classList.contains("lucide-clipboard")).toBe(true);
    view.unmount();
  });
});

describe("select menu", () => {
  const row = () => createElement(SelectMenuItem, { children: "Choose me", onSelect: () => {}, selected: true });

  it("renders nothing until open, then anchors the panel per align", () => {
    const closed = renderToStaticMarkup(createElement(SelectMenu, {
      children: row(),
      onDismiss: () => {},
      open: false,
      trigger: createElement("button", { type: "button" }, "open"),
    }));
    expect(closed).not.toContain("Choose me");

    const right = renderToStaticMarkup(createElement(SelectMenu, {
      children: row(),
      onDismiss: () => {},
      open: true,
      trigger: createElement("button", { type: "button" }, "open"),
    }));
    expect(right).toContain("Choose me");
    expect(right).toContain("right-0");

    const left = renderToStaticMarkup(createElement(SelectMenu, {
      align: "left",
      children: row(),
      onDismiss: () => {},
      open: true,
      panelClassName: "w-40",
      trigger: createElement("button", { type: "button" }, "open"),
    }));
    expect(left).toContain("left-0");
    expect(left).toContain("w-40");
  });

  it("dismisses on an outside pointerdown but not from inside the menu", () => {
    const onDismiss = vi.fn();
    const view = mount(createElement(SelectMenu, {
      children: row(),
      onDismiss,
      open: true,
      trigger: createElement("button", { type: "button" }, "open"),
    }));
    const inside = [...view.container.querySelectorAll("button")].slice(-1)[0]!;
    act(() => {
      inside.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(onDismiss).not.toHaveBeenCalled();
    act(() => {
      document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("dismisses on Escape", () => {
    const onDismiss = vi.fn();
    const view = mount(createElement(SelectMenu, {
      children: row(),
      onDismiss,
      open: true,
      trigger: createElement("button", { type: "button" }, "open"),
    }));
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(onDismiss).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("disables a menu item and shows its check only when selected", () => {
    const onSelect = vi.fn();
    const view = mount(createElement(SelectMenuItem, { children: "Row", disabled: true, onSelect, selected: false, title: "tip" }));
    const item = view.container.querySelector("button")!;
    expect(item.disabled).toBe(true);
    expect(item.getAttribute("title")).toBe("tip");
    expect(item.querySelector("svg")).toBeNull();
    act(() => item.click());
    expect(onSelect).not.toHaveBeenCalled();

    view.rerender(createElement(SelectMenuItem, { children: "Row", onSelect, selected: true }));
    expect(view.container.querySelector("svg")!.classList.contains("lucide-check")).toBe(true);
    view.unmount();
  });
});

describe("overlay stack", () => {
  it("reports whether any overlay is open and unwinds on close", () => {
    const ref = { current: true };
    const harness = renderHook(() => useOverlayLayer(ref.current));
    expect(hasOpenOverlay()).toBe(true);
    expect(harness.current.isTop()).toBe(true);

    ref.current = false;
    harness.rerender();
    expect(hasOpenOverlay()).toBe(false);
    expect(harness.current.isTop()).toBe(false);
    harness.unmount();
  });

  it("only the topmost layer answers isTop", () => {
    const parent = renderHook(() => useOverlayLayer(true));
    const child = renderHook(() => useOverlayLayer(true));
    expect(parent.current.isTop()).toBe(false);
    expect(child.current.isTop()).toBe(true);
    expect(hasOpenOverlay()).toBe(true);
    child.unmount();
    expect(parent.current.isTop()).toBe(true);
    parent.unmount();
    expect(hasOpenOverlay()).toBe(false);
  });
});
