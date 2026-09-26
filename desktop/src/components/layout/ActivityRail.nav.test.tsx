// @vitest-environment jsdom
import type { ComponentProps, ReactElement } from "react";
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../test/renderHook";
import { ActivityRail } from "./ActivityRail";

vi.mock("../../integrations/skills/skillsClient", () => ({ listInstalledSkills: async () => [] }));
vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => undefined }));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => false }));
vi.mock("./hooks/usePendingApprovalCounts", () => ({ usePendingApprovalCounts: () => new Map() }));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({ scrollRef: { current: null }, scrollbar: { height: 0, top: 0, visible: false }, handleScroll: () => {}, handleThumbPointerDown: () => {} }),
}));

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: id,
    mode: "chat",
    workspaceId: id,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

const alpha: StoredWorkspace = {
  cleanupStatus: "active",
  createdAt: 0,
  id: "ws-a",
  kind: "user",
  name: "Alpha",
  path: "/tmp/alpha",
  updatedAt: 0,
};

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

function props(threads: StoredThread[], overrides: Partial<ComponentProps<typeof ActivityRail>> = {}): ComponentProps<typeof ActivityRail> {
  return {
    active: "chat" as const,
    activeThreadId: "root",
    expanded: true,
    futureSessionStatus: "authenticated",
    onBatchDeleteThreads: vi.fn(),
    onChange: vi.fn(),
    onDeleteThread: vi.fn(),
    onDeleteWorkspace: vi.fn(),
    onDismissSkillIntro: vi.fn(),
    onNewChat: vi.fn(),
    onNewWorkspace: vi.fn(),
    onOpenModels: vi.fn(),
    onRenameThread: vi.fn(),
    onRenameWorkspace: vi.fn(),
    onRestoreThread: vi.fn(),
    onSelectThread: vi.fn(),
    onSelectWorkspace: vi.fn(),
    onToggleExpanded: vi.fn(),
    onTogglePinThread: vi.fn(),
    onTogglePinWorkspace: vi.fn(),
    skillIntroDismissed: true,
    threadRunStatuses: {},
    threadStreamingStatuses: {},
    threads,
    unreadThreadIds: new Set<string>(),
    userEmail: "alice@example.com",
    workspaces: [alpha],
    ...overrides,
  };
}

function buttonByText(container: HTMLElement, text: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === text);
}

/**
 * The complete set of labelled buttons in the expanded rail's nav, in render
 *  order. Discovered from a deliberate mismatch while writing the guard test
 *  below; the trailing two come from that test's workspace/account fixtures.
 */
const EXPECTED_NAV_ENTRIES = ["New Chat", "Models", "Skills", "Phone Control", "Alpha", "Aalice"];

beforeEach(() => {
  localStorage.clear();
});
afterEach(() => {
  localStorage.clear();
  vi.useRealTimers();
});

describe("activity rail navigation", () => {
  it("routes every expanded nav entry and the workspace/chat '+' buttons", () => {
    const p = props([thread("root"), thread("ws-thread", { mode: "workspace", workspaceId: "ws-a" })]);
    const view = mount(<ActivityRail {...p} />);

    act(() => buttonByText(view.container, "New Chat")!.click());
    expect(p.onNewChat).toHaveBeenCalledWith();
    act(() => buttonByText(view.container, "Models")!.click());
    expect(p.onOpenModels).toHaveBeenCalledTimes(1);
    act(() => buttonByText(view.container, "Skills")!.click());
    expect(p.onChange).toHaveBeenCalledWith("skill");
    act(() => buttonByText(view.container, "Phone Control")!.click());
    expect(p.onChange).toHaveBeenLastCalledWith("remote");

    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"New workspace\"]")!.click());
    expect(p.onNewWorkspace).toHaveBeenCalledTimes(1);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"New chat\"]")!.click());
    expect(p.onNewChat).toHaveBeenCalledTimes(2);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"New chat in Alpha\"]")!.click());
    expect(p.onNewChat).toHaveBeenLastCalledWith("ws-a");
    view.unmount();
  });

  it("renders exactly the expected set of nav entries", () => {
    // A *set* assertion, not a spot check. The sibling test above clicks each
    // entry by name, which proves those entries exist but says nothing about
    // extras — and `ActivityRail` has a `featureItems` array that is currently
    // empty but is documented as "add them back to restore" (`ActivityRail.tsx`
    // L88-92). Injecting one entry there makes a new nav button appear, and
    // before this test existed **55 of 55 rail tests still passed**.
    //
    // The list below is the complete set of labelled buttons in the nav, in
    // render order: the four section entries (each wired to a different handler,
    // which is why they cannot be isolated by "who calls onChange"), then the
    // workspace group header and the account button. The last two come from this
    // test's own fixtures (workspace "Alpha", email "alice@example.com"), so the
    // expectation is deterministic; a deliberate UI addition updates this line.
    const view = mount(<ActivityRail {...props([thread("root")])} />);
    const nav = view.container.querySelector("nav")!;
    const labels = [...nav.querySelectorAll<HTMLButtonElement>("button")]
      .map(button => (button.textContent ?? "").trim())
      .filter(Boolean);
    expect(labels).toEqual(EXPECTED_NAV_ENTRIES);
    view.unmount();
  });

  it("hides Phone Control while the account is not authenticated", () => {
    const view = mount(<ActivityRail {...props([thread("root")], { futureSessionStatus: "signed_out" })} />);
    expect(buttonByText(view.container, "Phone Control")).toBeUndefined();
    view.unmount();
  });

  it("routes the collapsed rail's icon buttons", () => {
    const p = props([thread("root")], { expanded: false });
    const view = mount(<ActivityRail {...p} />);

    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"New chat\"]")!.click());
    expect(p.onNewChat).toHaveBeenCalledWith();
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Models\"]")!.click());
    expect(p.onOpenModels).toHaveBeenCalledTimes(1);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Phone Control\"]")!.click());
    expect(p.onChange).toHaveBeenCalledWith("remote");
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Workspace\"]")!.click());
    expect(p.onChange).toHaveBeenCalledWith("workspace");
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Chat\"]")!.click());
    expect(p.onChange).toHaveBeenCalledWith("chat");
    view.unmount();
  });

  it("collapses and re-expands each list section independently", () => {
    const p = props([thread("root"), thread("ws-thread", { mode: "workspace", workspaceId: "ws-a" })]);
    const view = mount(<ActivityRail {...p} />);
    const has = (title: string) => view.container.querySelector(`button[aria-label="${title}"]`) !== null;

    expect(has("root")).toBe(true);
    expect(has("ws-thread")).toBe(true);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Collapse chat list\"]")!.click());
    expect(has("root")).toBe(false);
    // Workspace section is independent — its rows stay.
    expect(has("ws-thread")).toBe(true);
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Expand chat list\"]")!.click());
    expect(has("root")).toBe(true);

    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Collapse workspace list\"]")!.click());
    expect(has("ws-thread")).toBe(false);
    expect(has("root")).toBe(true);
    view.unmount();
  });
});

describe("activity rail workspace group actions", () => {
  it("lists a non-user workspace that still owns conversations", () => {
    const shared = { ...alpha, id: "ws-shared", kind: "shared", name: "Shared" } as unknown as StoredWorkspace;
    const p = props([thread("shared-thread", { mode: "workspace", workspaceId: "ws-shared" })], { workspaces: [shared] });
    const view = mount(<ActivityRail {...p} />);
    expect([...view.container.querySelectorAll("button")].some(button => button.textContent === "Shared")).toBe(true);
    view.unmount();
  });

  it("selects the workspace, opens its menu on right-click and starts a chat inside it", () => {
    const p = props([thread("ws-thread", { mode: "workspace", workspaceId: "ws-a" })]);
    const view = mount(<ActivityRail {...p} />);
    const nameButton = [...view.container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Alpha")!;

    act(() => nameButton.click());
    expect(p.onSelectWorkspace).toHaveBeenCalledWith(alpha, [expect.objectContaining({ id: "ws-thread" })]);

    const header = nameButton.parentElement!;
    act(() => {
      header.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    });
    expect(view.container.querySelector("[role=\"menu\"]")).not.toBeNull();
    view.unmount();
  });

  it("enters selection mode for a workspace from its header menu", async () => {
    const p = props([
      thread("ws-a", { mode: "workspace", workspaceId: "ws-a" }),
      thread("ws-b", { mode: "workspace", workspaceId: "ws-a" }),
    ]);
    const view = mount(<ActivityRail {...p} />);
    const nameButton = [...view.container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Alpha")!;
    act(() => {
      nameButton.parentElement!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    });
    const selectChats = [...view.container.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")]
      .find(item => item.textContent === "Select chats")!;
    act(() => selectChats.click());
    await flushAsync();

    // Selection mode replaces the rows' actions with checkboxes.
    expect(view.container.querySelector("input[type=\"checkbox\"]")).not.toBeNull();
    view.unmount();
  });
});

describe("activity rail thread menu", () => {
  it("opens a thread's actions menu from the row and routes its items", () => {
    const p = props([thread("root")]);
    const view = mount(<ActivityRail {...p} />);
    const trigger = view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Thread actions for root\"]")!;
    expect(trigger.getAttribute("aria-expanded")).toBe("false");

    act(() => trigger.click());
    expect(view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Thread actions for root\"]")!.getAttribute("aria-expanded")).toBe("true");
    const rename = [...view.container.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")]
      .find(item => item.textContent === "Rename")!;
    act(() => rename.click());
    expect(p.onRenameThread).toHaveBeenCalledWith(expect.objectContaining({ id: "root" }));
    expect(view.container.querySelector("[role=\"menu\"]")).toBeNull();
    view.unmount();
  });
});

describe("activity rail skill attention", () => {
  it("routes the intro bubble's go action to the Skills page", async () => {
    const p = props([thread("root")], { skillIntroDismissed: false });
    const view = mount(<ActivityRail {...p} />);
    // The bubble waits for the installed-skills count to land.
    expect(buttonByText(view.container, "Take a look")).toBeUndefined();
    await flushAsync();
    act(() => buttonByText(view.container, "Take a look")!.click());
    expect(p.onDismissSkillIntro).toHaveBeenCalledTimes(1);
    expect(p.onChange).toHaveBeenCalledWith("skill");
    view.unmount();
  });

  it("pulses the Skills entry after the guide is dismissed and stops after 4s", async () => {
    vi.useFakeTimers();
    const view = mount(<ActivityRail {...props([thread("root")])} />);
    await act(async () => {
      await Promise.resolve();
    });
    const skillsClass = () => buttonByText(view.container, "Skills")!.className;
    const before = skillsClass();

    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:skill-guide-dismissed", { detail: undefined }));
      await Promise.resolve();
    });
    expect(skillsClass()).not.toBe(before);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(4000);
    });
    expect(skillsClass()).toBe(before);
    view.unmount();
  });
});
