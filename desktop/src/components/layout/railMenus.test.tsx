// @vitest-environment jsdom
import type { ReactElement } from "react";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ActivityRailAccountFooter } from "./ActivityRailAccountFooter";
import { ChatSectionMenu, ThreadItemMenu, WorkspaceHeaderMenu } from "./ActivityRailMenus";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({ openPath: vi.fn(async (_path: string) => {}) }));
vi.mock("../../integrations/storage/files", () => ({
  openPath: (path: string) => mocks.openPath(path),
}));

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    rerender: (next: ReactElement) => act(() => root.render(next)),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function item(container: HTMLElement, label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")].find(button => button.textContent === label);
}

function workspace(overrides: Partial<StoredWorkspace> = {}): StoredWorkspace {
  return {
    cleanupStatus: "active",
    createdAt: 0,
    id: "ws-1",
    kind: "user",
    name: "Alpha",
    path: "/tmp/alpha",
    updatedAt: 0,
    ...overrides,
  };
}

beforeEach(() => {
  mocks.openPath.mockReset().mockResolvedValue(undefined);
  // Defer the focus callback past React's commit so the menu exists when the
  // handler runs, mirroring a real animation frame.
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    setTimeout(callback, 0, 0);
    return 0;
  });
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("thread item menu", () => {
  function mountMenu(overrides: Partial<Parameters<typeof ThreadItemMenu>[0]> = {}) {
    const handlers = {
      onClose: vi.fn(),
      onDelete: vi.fn(),
      onRename: vi.fn(),
      onRestore: vi.fn(),
      onTogglePin: vi.fn(),
    };
    const view = mount(createElement(ThreadItemMenu, { pinned: false, ...handlers, ...overrides }));
    return { ...view, handlers };
  }

  it("offers rename / pin / delete for a live thread, closing before acting", () => {
    const { container, handlers, unmount } = mountMenu();
    expect(item(container, "Rename")).toBeTruthy();
    expect(item(container, "Pin")).toBeTruthy();
    expect(item(container, "Unpin")).toBeUndefined();
    expect(item(container, "Restore")).toBeUndefined();

    act(() => item(container, "Rename")!.click());
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
    expect(handlers.onRename).toHaveBeenCalledTimes(1);

    act(() => item(container, "Delete")!.click());
    expect(handlers.onDelete).toHaveBeenCalledTimes(1);
    unmount();
  });

  it("offers unpin and, for an archived thread, restore instead of rename/pin", () => {
    const pinned = mountMenu({ pinned: true });
    expect(item(pinned.container, "Unpin")).toBeTruthy();
    act(() => item(pinned.container, "Unpin")!.click());
    expect(pinned.handlers.onTogglePin).toHaveBeenCalledTimes(1);
    pinned.unmount();

    const archived = mountMenu({ archived: true });
    expect(item(archived.container, "Restore")).toBeTruthy();
    expect(item(archived.container, "Rename")).toBeUndefined();
    expect(item(archived.container, "Pin")).toBeUndefined();
    act(() => item(archived.container, "Restore")!.click());
    expect(archived.handlers.onRestore).toHaveBeenCalledTimes(1);
    archived.unmount();
  });

  it("moves focus between items with the arrow keys and Home/End", () => {
    const { container, unmount } = mountMenu({ archived: true });
    const panel = container.querySelector("[role=\"menu\"]")!;
    const [restore, remove] = [...panel.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")];

    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
    });
    expect(document.activeElement).toBe(restore);
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
    });
    expect(document.activeElement).toBe(remove);
    // Wraps back to the first item.
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
    });
    expect(document.activeElement).toBe(restore);
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowUp" }));
    });
    expect(document.activeElement).toBe(remove);
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Home" }));
    });
    expect(document.activeElement).toBe(restore);
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "End" }));
    });
    expect(document.activeElement).toBe(remove);

    // Unrelated keys are ignored.
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "a" }));
    });
    expect(document.activeElement).toBe(remove);
    unmount();
  });
});

describe("workspace header menu", () => {
  const alpha = workspace();

  it("renders nothing until open and marks the trigger", () => {
    const closed = mount(createElement(WorkspaceHeaderMenu, {
      onDelete: vi.fn(),
      onOpenChange: vi.fn(),
      onRename: vi.fn(),
      onTogglePin: vi.fn(),
      open: false,
      workspace: alpha,
    }));
    expect(closed.container.querySelector("[role=\"menu\"]")).toBeNull();
    expect(closed.container.querySelector("button")!.getAttribute("aria-expanded")).toBe("false");
    closed.unmount();
  });

  it("runs every action and only offers Select chats when a handler is wired", () => {
    const onDelete = vi.fn();
    const onOpenChange = vi.fn();
    const onRename = vi.fn();
    const onSelect = vi.fn();
    const onTogglePin = vi.fn();
    const view = mount(createElement(WorkspaceHeaderMenu, {
      onDelete,
      onOpenChange,
      onRename,
      onSelect,
      onTogglePin,
      open: true,
      workspace: alpha,
    }));

    expect(view.container.querySelector("button")!.getAttribute("aria-label")).toBe("Workspace actions for Alpha");
    act(() => item(view.container, "Rename")!.click());
    expect(onRename).toHaveBeenCalledWith(alpha);
    act(() => item(view.container, "Pin")!.click());
    expect(onTogglePin).toHaveBeenCalledWith(alpha);
    act(() => item(view.container, "Select chats")!.click());
    expect(onSelect).toHaveBeenCalledTimes(1);
    act(() => item(view.container, "Delete")!.click());
    expect(onDelete).toHaveBeenCalledWith(alpha);
    expect(onOpenChange).toHaveBeenCalledWith(false);

    act(() => item(view.container, "Open Folder")!.click());
    expect(mocks.openPath).toHaveBeenCalledWith("/tmp/alpha");
    view.unmount();
  });

  it("shows Unpin for a pinned workspace and hides Select chats without a handler", () => {
    const view = mount(createElement(WorkspaceHeaderMenu, {
      onDelete: vi.fn(),
      onOpenChange: vi.fn(),
      onRename: vi.fn(),
      onTogglePin: vi.fn(),
      open: true,
      workspace: workspace({ pinned: true }),
    }));
    expect(item(view.container, "Unpin")).toBeTruthy();
    expect(item(view.container, "Pin")).toBeUndefined();
    expect(item(view.container, "Select chats")).toBeUndefined();
    view.unmount();
  });

  it("toggles from the trigger, closes on Escape and restores focus to the trigger", () => {
    const onOpenChange = vi.fn();
    const view = mount(createElement(WorkspaceHeaderMenu, {
      onDelete: vi.fn(),
      onOpenChange,
      onRename: vi.fn(),
      onTogglePin: vi.fn(),
      open: true,
      workspace: alpha,
    }));
    const trigger = view.container.querySelector<HTMLButtonElement>("button")!;
    act(() => trigger.click());
    expect(onOpenChange).toHaveBeenCalledWith(false);

    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(onOpenChange).toHaveBeenLastCalledWith(false);
    expect(document.activeElement).toBe(trigger);
    view.unmount();
  });

  it("closes on a pointerdown outside the layer", () => {
    const onOpenChange = vi.fn();
    const view = mount(createElement(WorkspaceHeaderMenu, {
      onDelete: vi.fn(),
      onOpenChange,
      onRename: vi.fn(),
      onTogglePin: vi.fn(),
      open: true,
      workspace: alpha,
    }));
    act(() => {
      document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
    view.unmount();
  });
});

describe("chat section menu", () => {
  it("opens its trigger without bubbling and selects all chats", () => {
    const onOpenChange = vi.fn();
    const onSelect = vi.fn();
    const parentClick = vi.fn();
    const view = mount(createElement("div", { onClick: parentClick }, createElement(ChatSectionMenu, { onOpenChange, onSelect, open: true })));
    const trigger = view.container.querySelector<HTMLButtonElement>("button")!;
    expect(trigger.getAttribute("aria-label")).toBe("Chat section actions");

    act(() => item(view.container, "Select chats")!.click());
    expect(onSelect).toHaveBeenCalledTimes(1);

    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(document.activeElement).toBe(trigger);

    parentClick.mockClear();
    act(() => {
      document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);

    act(() => trigger.click());
    expect(onOpenChange).toHaveBeenLastCalledWith(false);
    // The click must not reach the section header behind the trigger.
    expect(parentClick).not.toHaveBeenCalled();
    view.unmount();
  });
});

describe("activity rail account footer", () => {
  const base = {
    active: "chat" as const,
    balance: 12,
    onChange: vi.fn(),
  };

  it("collapses to an icon button that reports the settings section", () => {
    const onChange = vi.fn();
    const view = mount(createElement(ActivityRailAccountFooter, { ...base, expanded: false, hasUpdate: true, onChange }));
    const button = view.container.querySelector<HTMLButtonElement>("button")!;
    expect(button.getAttribute("aria-label")).toBe("Settings");
    expect(view.container.querySelector(".bg-accent")).not.toBeNull();
    act(() => button.click());
    expect(onChange).toHaveBeenCalledWith("settings");
    view.unmount();
  });

  it("shows a plain settings row when there is no account or it is a community build", () => {
    const onChange = vi.fn();
    const view = mount(createElement(ActivityRailAccountFooter, { ...base, expanded: true, onChange }));
    const button = view.container.querySelector<HTMLButtonElement>("button")!;
    expect(button.textContent).toBe("Settings");
    act(() => button.click());
    expect(onChange).toHaveBeenCalledWith("settings");
    view.unmount();

    const community = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      communityEdition: true,
      expanded: true,
      onChange,
      userEmail: "me@example.com",
    }));
    expect(community.container.querySelector("button")!.textContent).toBe("Settings");
    community.unmount();
  });

  it("highlights the settings row while that section is active", () => {
    const view = mount(createElement(ActivityRailAccountFooter, { ...base, active: "settings", expanded: true, onChange: vi.fn() }));
    expect(view.container.querySelector("button")!.className).toContain("bg-accent-soft");
    view.unmount();
  });

  it("points the trigger's aria-controls at the menu it actually opens", () => {
    // `aria-controls` is a real contract, not decoration: assistive tech follows
    // the id to find the controlled element. A common React defect is a *dangling*
    // reference, because the menu is only rendered while open — so the assertion
    // is that the id resolves to the element with role="menu" **once open**, and
    // the closed state is checked too (see the note below).
    const view = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      expanded: true,
      onChange: vi.fn(),
      userEmail: "alice@example.com",
    }));
    const trigger = view.container.querySelector<HTMLButtonElement>("button[aria-haspopup=\"menu\"]")!;
    const controls = trigger.getAttribute("aria-controls");
    expect(controls).toBeTruthy();

    // Closed: the referenced element is not in the document yet.
    expect(document.getElementById(controls!)).toBeNull();

    act(() => trigger.click());
    const menu = document.getElementById(controls!);
    // Open: the reference resolves, and to the menu itself.
    expect(menu).not.toBeNull();
    expect(menu!.getAttribute("role")).toBe("menu");
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    view.unmount();
  });

  it("opens the account menu with the balance and the recharge action", async () => {
    const onRecharge = vi.fn();
    const onOpenSettings = vi.fn();
    const view = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      balance: 12.7,
      expanded: true,
      onChange: section => (section === "settings" ? onOpenSettings() : undefined),
      onRecharge,
      userEmail: "alice@example.com",
    }));

    const trigger = view.container.querySelector<HTMLButtonElement>("button")!;
    expect(trigger.textContent).toContain("alice");
    act(() => trigger.click());
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 0));
    });
    // The first menu item takes focus once the menu is open.
    expect((document.activeElement as HTMLElement)?.textContent).toContain("Settings");

    act(() => item(view.container, "Settings")!.click());
    expect(onOpenSettings).toHaveBeenCalledTimes(1);

    act(() => view.container.querySelector<HTMLButtonElement>("button")!.click());
    const balanceRow = [...view.container.querySelectorAll("[role=\"menuitem\"]")].find(row => row.textContent!.includes("Balance"));
    expect(balanceRow!.textContent).toContain("12");
    expect(balanceRow!.textContent).toContain("Recharge");
    act(() => (balanceRow as HTMLButtonElement).click());
    expect(onRecharge).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("shows an em dash while the balance is unknown", () => {
    const view = mount(createElement(ActivityRailAccountFooter, { ...base, balance: null, expanded: true, onChange: vi.fn(), userEmail: "bob@example.com" }));
    act(() => view.container.querySelector<HTMLButtonElement>("button")!.click());
    const balanceRow = [...view.container.querySelectorAll("[role=\"menuitem\"]")].find(row => row.textContent!.includes("Balance"));
    expect(balanceRow!.textContent).toContain("—");
    view.unmount();
  });

  it("falls back to the whole address as the avatar label when there is no domain prefix", () => {
    const view = mount(createElement(ActivityRailAccountFooter, { ...base, expanded: true, onChange: vi.fn(), userEmail: "" }));
    // An empty address cannot render an AccountMenuButton; the settings row is used.
    expect(view.container.querySelector("[role=\"menu\"]")).toBeNull();
    view.unmount();

    const fallback = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      expanded: true,
      onChange: vi.fn(),
      userEmail: "solo",
    }));
    const initials = fallback.container.querySelector("span.bg-accent-soft")!;
    expect(initials.textContent).toBe("S");
    expect(fallback.container.textContent).toContain("solo");
    fallback.unmount();
  });

  it("uses the whole address when the local part is empty", () => {
    // `email.split("@")[0]` is `""` for an address that starts with `@`, and an
    // empty string is falsy — so the `|| email` fallback keeps the whole address
    // as the avatar label (and its first character as the initial).
    const view = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      expanded: true,
      onChange: vi.fn(),
      userEmail: "@example.com",
    }));
    const initials = view.container.querySelector("span.bg-accent-soft")!;
    expect(initials.textContent).toBe("@");
    expect(view.container.textContent).toContain("@example.com");
    view.unmount();
  });

  it("marks the settings row with the update dot when there is no account", () => {
    // The other `hasUpdate` dot sits on the *settings row* branch, which is only
    // taken without an account — so the upgrade-action test above (which always
    // has an email) never renders this one.
    const view = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      expanded: true,
      hasUpdate: true,
      onChange: vi.fn(),
      userEmail: null,
    }));
    const dot = [...view.container.querySelectorAll("span")]
      .find(span => span.className.includes("-right-1") && span.className.includes("bg-accent"));
    expect(dot).toBeTruthy();
    view.unmount();

    // And it is absent when there is nothing to upgrade to — the null arm.
    const quiet = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      expanded: true,
      onChange: vi.fn(),
      userEmail: null,
    }));
    expect([...quiet.container.querySelectorAll("span")]
      .find(span => span.className.includes("-right-1") && span.className.includes("bg-accent"))).toBeUndefined();
    quiet.unmount();
  });

  it("surfaces the upgrade action beside the account and closes on click", () => {
    const onOpenUpdate = vi.fn();
    const view = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      expanded: true,
      hasUpdate: true,
      onChange: vi.fn(),
      onOpenUpdate,
      userEmail: "alice@example.com",
    }));
    const upgrade = [...view.container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.getAttribute("aria-label") === "Upgrade")!;
    act(() => upgrade.click());
    expect(onOpenUpdate).toHaveBeenCalledTimes(1);
    expect(view.container.querySelector("[role=\"menu\"]")).toBeNull();
    view.unmount();
  });

  it("closes the account menu on Escape, outside click and a second trigger click", () => {
    const view = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      expanded: true,
      onChange: vi.fn(),
      userEmail: "alice@example.com",
    }));
    const trigger = view.container.querySelector<HTMLButtonElement>("button")!;

    act(() => trigger.click());
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(view.container.querySelector("[role=\"menu\"]")).toBeNull();
    expect(document.activeElement).toBe(trigger);

    act(() => trigger.click());
    act(() => {
      document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(view.container.querySelector("[role=\"menu\"]")).toBeNull();

    act(() => trigger.click());
    expect(view.container.querySelector("[role=\"menu\"]")).not.toBeNull();
    act(() => trigger.click());
    expect(view.container.querySelector("[role=\"menu\"]")).toBeNull();
    view.unmount();
  });

  it("cycles focus through the account menu with arrows, Home and End", async () => {
    const view = mount(createElement(ActivityRailAccountFooter, {
      ...base,
      expanded: true,
      onChange: vi.fn(),
      userEmail: "alice@example.com",
    }));
    act(() => view.container.querySelector<HTMLButtonElement>("button")!.click());
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 0));
    });
    const panel = view.container.querySelector<HTMLElement>("[role=\"menu\"]")!;
    const rows = [...panel.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")];
    expect(document.activeElement).toBe(rows[0]);

    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
    });
    expect(document.activeElement).toBe(rows[1]);
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowUp" }));
    });
    expect(document.activeElement).toBe(rows[0]);
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "End" }));
    });
    expect(document.activeElement).toBe(rows[1]);
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Home" }));
    });
    expect(document.activeElement).toBe(rows[0]);
    act(() => {
      panel.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "x" }));
    });
    expect(document.activeElement).toBe(rows[0]);
    view.unmount();
  });
});

/**
 * `useDropUpMenu` flips a dropdown above its trigger when the trigger sits too
 * close to the bottom of its clipping container, so a thread near the sidebar
 * bottom still shows its whole menu (including Delete). jsdom reports every
 * rect as zero, so the hook's real measurement always resolves to "drop down" and
 * the flip arm was never exercised. Stubbing the measured rect is how the real
 * geometry is reproduced here.
 */
describe("dropdown flip", () => {
  const originalRect = Element.prototype.getBoundingClientRect;

  afterEach(() => {
    Element.prototype.getBoundingClientRect = originalRect;
  });

  /** Report a menu whose bottom edge is below the clip boundary. */
  function stubMenuBottomAt(bottom: number) {
    Element.prototype.getBoundingClientRect = function (this: Element) {
      return { bottom, height: 0, left: 0, right: 0, top: bottom, width: 0, x: 0, y: bottom, toJSON: () => ({}) } as DOMRect;
    };
  }

  const menuClasses = (container: HTMLElement) => container.querySelector("[role=\"menu\"]")!.className;

  it("flips the thread menu above the trigger when it would spill past its container", () => {
    stubMenuBottomAt(window.innerHeight + 200);
    const view = mount(createElement(ThreadItemMenu, {
      onClose: vi.fn(),
      onDelete: vi.fn(),
      onRename: vi.fn(),
      onRestore: vi.fn(),
      onTogglePin: vi.fn(),
      pinned: false,
    }));
    expect(menuClasses(view.container)).toContain("bottom-7");
    expect(menuClasses(view.container)).not.toContain("top-7");
    view.unmount();
  });

  it("flips the workspace header menu the same way", () => {
    stubMenuBottomAt(window.innerHeight + 200);
    const view = mount(createElement(WorkspaceHeaderMenu, {
      onDelete: vi.fn(),
      onOpenChange: vi.fn(),
      onRename: vi.fn(),
      onTogglePin: vi.fn(),
      open: true,
      workspace: workspace(),
    }));
    expect(menuClasses(view.container)).toContain("bottom-7");
    view.unmount();
  });

  it("flips the chat section menu the same way", () => {
    stubMenuBottomAt(window.innerHeight + 200);
    const view = mount(createElement(ChatSectionMenu, {
      onOpenChange: vi.fn(),
      onSelect: vi.fn(),
      open: true,
    }));
    expect(menuClasses(view.container)).toContain("bottom-7");
    view.unmount();
  });

  it("keeps the default downward placement while there is room below", () => {
    stubMenuBottomAt(10);
    const view = mount(createElement(ThreadItemMenu, {
      onClose: vi.fn(),
      onDelete: vi.fn(),
      onRename: vi.fn(),
      onRestore: vi.fn(),
      onTogglePin: vi.fn(),
      pinned: false,
    }));
    expect(menuClasses(view.container)).toContain("top-7");
    expect(menuClasses(view.container)).not.toContain("bottom-7");
    view.unmount();
  });
});
