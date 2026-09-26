// @vitest-environment jsdom
import type { ComponentProps, ReactElement } from "react";
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../test/renderHook";
import { ActivityRail } from "./ActivityRail";

// The rail's remaining uncovered *branches* live in its variant axes: floating vs
// docked, the remote connection indicator, the macOS traffic-light inset, the
// skill badge/attention affordances and selection mode. Each is a distinct
// rendering mode, so each gets its own assertion rather than being inferred from
// a shared line-coverage reading.
const mocks = vi.hoisted(() => ({
  isMacOS: false,
  isFullscreen: false,
  skills: [] as { id: string; name: string }[],
}));

vi.mock("../../integrations/skills/skillsClient", () => ({
  listInstalledSkills: async () => mocks.skills,
}));
vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => undefined }));
vi.mock("../../lib/platform", () => ({
  get isMacOS() {
    return mocks.isMacOS;
  },
  isWindows: false,
  isLinux: false,
}));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => mocks.isFullscreen }));
vi.mock("./hooks/usePendingApprovalCounts", () => ({ usePendingApprovalCounts: () => new Map() }));
vi.mock("../../lib/useFloatingScrollbar", () => ({
  useFloatingScrollbar: () => ({ scrollRef: { current: null }, scrollbar: { height: 0, top: 0, visible: false }, handleScroll: () => {}, handleThumbPointerDown: () => {} }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

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

function props(overrides: Partial<ComponentProps<typeof ActivityRail>> = {}): ComponentProps<typeof ActivityRail> {
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
    threads: [thread("root")],
    unreadThreadIds: new Set<string>(),
    userEmail: "alice@example.com",
    workspaces: [alpha],
    ...overrides,
  };
}

const nav = (container: HTMLElement) => container.querySelector("nav")!;
function byLabel(container: HTMLElement, label: string) {
  return container.querySelector<HTMLElement>(`[aria-label="${label}"]`);
}
/** Expanded nav entries render a text label (no aria-label); collapsed ones use IconButton. */
function byText(container: HTMLElement, label: string) {
  return [...container.querySelectorAll<HTMLElement>("button")]
    .find(button => button.querySelector("span.truncate")?.textContent === label)!;
}
const skillsEntry = (container: HTMLElement) => byText(container, "Skills");
function remoteEntry(container: HTMLElement, expanded: boolean) {
  return expanded ? byText(container, "Phone Control") : byLabel(container, "Phone Control");
}

beforeEach(() => {
  mocks.isFullscreen = false;
  mocks.isMacOS = false;
  mocks.skills = [];
  localStorage.clear();
});
afterEach(() => {
  localStorage.clear();
  vi.useRealTimers();
});

describe("activity rail floating variant", () => {
  it("renders the floating skin and drops the docked divider", () => {
    const docked = mount(<ActivityRail {...props()} />);
    expect(nav(docked.container).className).toContain("shrink-0");
    expect(nav(docked.container).className).not.toContain("shadow-sidebar-floating");
    // The docked rail paints its own edge shadow; the floating one must not.
    expect(docked.container.querySelector(".shadow-sidebar-divider")).not.toBeNull();
    docked.unmount();

    const floating = mount(<ActivityRail {...props({ floating: true })} />);
    expect(nav(floating.container).className).toContain("rounded-r-lg");
    expect(nav(floating.container).className).toContain("shadow-sidebar-floating");
    expect(nav(floating.container).className).not.toContain("shrink-0");
    expect(floating.container.querySelector(".shadow-sidebar-divider")).toBeNull();
    floating.unmount();
  });

  it("labels its toggle from the floating/docked and expanded/collapsed axes", () => {
    // Floating is pinned by definition, so its toggle offers to pin, not collapse.
    const floating = mount(<ActivityRail {...props({ floating: true })} />);
    expect(byLabel(floating.container, "Pin sidebar")).not.toBeNull();
    expect(byLabel(floating.container, "Collapse sidebar")).toBeNull();
    floating.unmount();

    const dockedExpanded = mount(<ActivityRail {...props()} />);
    expect(byLabel(dockedExpanded.container, "Collapse sidebar")).not.toBeNull();
    dockedExpanded.unmount();

    const dockedCollapsed = mount(<ActivityRail {...props({ expanded: false })} />);
    expect(byLabel(dockedCollapsed.container, "Expand sidebar")).not.toBeNull();
    dockedCollapsed.unmount();
  });
});

describe("activity rail remote indicator", () => {
  it.each([
    ["connected", "bg-accent", false],
    ["connecting", "bg-warning", true],
    ["disconnected", "bg-danger", false],
    [null, null, false],
  ] as const)("paints %s as %s", (indicator, tone, pulses) => {
    const view = mount(<ActivityRail {...props({ remoteIndicator: indicator })} />);
    if (tone === null) {
      // No pairing yet: none of the three tones may render.
      expect(view.container.querySelector(".bg-accent.rounded-full")).toBeNull();
      expect(view.container.querySelector(".bg-warning")).toBeNull();
      expect(view.container.querySelector(".bg-danger")).toBeNull();
      view.unmount();
      return;
    }
    const dot = view.container.querySelector(`.${tone}`)!;
    expect(dot).not.toBeNull();
    expect(dot.className).toContain("rounded-full");
    // Only the "connecting" state animates — a static red/blue dot must not pulse.
    expect(dot.className.includes("animate-pulse")).toBe(pulses);
    view.unmount();
  });

  it("shows no indicator before pairing", () => {
    const view = mount(<ActivityRail {...props({ remoteIndicator: null })} />);
    expect(view.container.querySelector(".bg-accent.rounded-full")).toBeNull();
    expect(view.container.querySelector(".bg-warning")).toBeNull();
    expect(view.container.querySelector(".bg-danger")).toBeNull();
    view.unmount();
  });

  it("passes the indicator into the collapsed rail's remote entry too", () => {
    const view = mount(<ActivityRail {...props({ expanded: false, remoteIndicator: "connecting" })} />);
    const remote = byLabel(view.container, "Phone Control")!;
    expect(remote.querySelector(".bg-warning")!.className).toContain("animate-pulse");
    view.unmount();
  });
});

describe("activity rail macOS traffic-light inset", () => {
  it.each([
    { className: "absolute left-20 top-2", fullscreen: false, isMacOS: true, label: "macOS windowed" },
    { className: "absolute left-2 top-2", fullscreen: true, isMacOS: true, label: "macOS fullscreen" },
    { className: "absolute left-2 top-2", fullscreen: false, isMacOS: false, label: "other platforms" },
  ])("$label reserves the inset accordingly", ({ className, fullscreen, isMacOS }) => {
    mocks.isMacOS = isMacOS;
    mocks.isFullscreen = fullscreen;
    const view = mount(<ActivityRail {...props()} />);
    const toggle = byLabel(view.container, "Collapse sidebar")!;
    expect(toggle.className).toContain(className);
    view.unmount();
  });

  it("only applies the inset while the rail is expanded", () => {
    mocks.isMacOS = true;
    const view = mount(<ActivityRail {...props({ expanded: false })} />);
    // Collapsed, the toggle is centred and takes no absolute inset at all.
    const toggle = byLabel(view.container, "Expand sidebar")!;
    expect(toggle.className).not.toContain("left-20");
    expect(toggle.className).not.toContain("left-2");
    view.unmount();
  });
});

describe("activity rail skill affordances", () => {
  it("badges the installed-skill count once the list loads", async () => {
    mocks.skills = [{ id: "a", name: "A" }, { id: "b", name: "B" }];
    const view = mount(<ActivityRail {...props()} />);
    expect(skillsEntry(view.container).textContent).toBe("Skills");
    await flushAsync();

    const entry = skillsEntry(view.container);
    expect(entry.querySelector("span.truncate")!.textContent).toBe("Skills");
    expect(entry.textContent).toContain("2");
    expect(entry.querySelector(".bg-accent-soft")).not.toBeNull();
    view.unmount();
  });

  it("omits the badge for an empty or unreachable skill list", async () => {
    const view = mount(<ActivityRail {...props()} />);
    await flushAsync();
    expect(skillsEntry(view.container).querySelector(".bg-accent-soft")).toBeNull();
    view.unmount();
  });

  it("marks the intro dot while the guide has not been dismissed", async () => {
    mocks.skills = [{ id: "a", name: "A" }];
    const view = mount(<ActivityRail {...props({ skillIntroDismissed: false })} />);
    await flushAsync();
    // The dot is the one rounded swatch inside the entry's trailing cluster.
    expect(skillsEntry(view.container).querySelector("span.ml-auto .bg-accent.rounded-full")).not.toBeNull();
    view.unmount();
  });

  it("hides the intro dot and the bubble once dismissed", async () => {
    const view = mount(<ActivityRail {...props({ skillIntroDismissed: true })} />);
    await flushAsync();
    expect(skillsEntry(view.container).querySelector("span.ml-auto")).toBeNull();
    expect(skillsEntry(view.container).querySelector(".bg-accent.rounded-full")).toBeNull();
    view.unmount();
  });

  it("pulses the entry while it holds attention and clears it on click", async () => {
    vi.useFakeTimers();
    const view = mount(<ActivityRail {...props()} />);
    await act(async () => {
      await Promise.resolve();
    });
    const before = skillsEntry(view.container).className;

    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:skill-guide-dismissed", { detail: undefined }));
      await Promise.resolve();
    });
    const pulsed = skillsEntry(view.container);
    expect(pulsed.className).toContain("animate-skill-entry-pulse");
    expect(pulsed.className).not.toBe(before);

    // Visiting the Skills page spends the attention immediately.
    act(() => pulsed.click());
    expect(skillsEntry(view.container).className).not.toContain("animate-skill-entry-pulse");
    view.unmount();
  });

  it("marks the active nav entry, including the Skills page", () => {
    const view = mount(<ActivityRail {...props({ active: "skill" })} />);
    expect(skillsEntry(view.container).className).toContain("bg-surface-subtle");
    expect(skillsEntry(view.container).className).toContain("text-ink");
    view.unmount();
  });
});

describe("activity rail selection mode", () => {
  const twoThreads = [
    thread("a"),
    thread("b"),
    thread("ws-a", { mode: "workspace", workspaceId: "ws-a" }),
  ];

  const enterSelection = async (container: HTMLElement, scope: "chat" | "workspace" = "workspace") => {
    if (scope === "workspace") {
      const header = byLabel(container, "Workspace actions for Alpha")!.parentElement!;
      act(() => header.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })));
    }
    else {
      act(() => byLabel(container, "Chat section actions")!.click());
    }
    const item = [...container.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")]
      .find(button => button.textContent === "Select chats")!;
    act(() => item.click());
    await flushAsync();
  };

  it("scopes the batch to the whole chat list when entered from the chat section", async () => {
    const view = mount(<ActivityRail {...props({ threads: twoThreads })} />);
    await flushAsync();
    await enterSelection(view.container, "chat");

    // Every unpinned chat thread is in scope, and the toolbar counts them.
    expect(view.container.querySelectorAll("input[type=\"checkbox\"][aria-label]").length).toBe(2);
    view.unmount();
  });

  it("shows a checkbox per scoped thread and toggles the selection from it", async () => {
    const view = mount(<ActivityRail {...props({ threads: twoThreads })} />);
    await flushAsync();
    await enterSelection(view.container);

    const checkbox = byLabel(view.container, "Select ws-a") as HTMLInputElement;
    expect(checkbox.checked).toBe(false);
    act(() => checkbox.click());
    // Selecting the only scoped thread fills the scope, so the toolbar's box
    // becomes a plain checked one rather than an indeterminate one.
    expect((byLabel(view.container, "Select ws-a") as HTMLInputElement).checked).toBe(true);
    expect(view.container.querySelector<HTMLElement>("[role=\"toolbar\"]")!.textContent).toContain("1 selected");
    view.unmount();
  });

  it("offers select-all while the scope is partly selected and deselect-all once it is full", async () => {
    const view = mount(<ActivityRail {...props({ threads: twoThreads })} />);
    await flushAsync();
    await enterSelection(view.container, "chat");

    const toolbar = () => view.container.querySelector<HTMLElement>("[role=\"toolbar\"]")!;
    const toggleBox = () => toolbar().querySelector<HTMLInputElement>("input[type=\"checkbox\"]")!;
    const scoped = [...view.container.querySelectorAll<HTMLInputElement>("input[type=\"checkbox\"][aria-label]")];
    expect(toolbar().textContent).toContain("Select all");
    expect(scoped.length).toBe(2);

    act(() => scoped[0]!.click());
    expect(toolbar().textContent).toContain("1 selected");
    expect(toggleBox().indeterminate).toBe(true);

    // Nothing was selected before, so this checkbox asks to select everything.
    act(() => toggleBox().click());
    expect(toggleBox().checked).toBe(true);
    expect(toolbar().textContent).toContain("2 selected");

    // Now that the whole scope is selected the same control deselects it.
    act(() => toggleBox().click());
    expect(toolbar().textContent).toContain("Select all");
    view.unmount();
  });

  it("drops the per-thread actions menu while selection mode is open", async () => {
    const view = mount(<ActivityRail {...props({ threads: twoThreads })} />);
    await flushAsync();
    expect(byLabel(view.container, "Thread actions for a")).not.toBeNull();

    await enterSelection(view.container);
    expect(byLabel(view.container, "Thread actions for ws-a")).toBeNull();
    expect(byLabel(view.container, "Select ws-a")).not.toBeNull();
    view.unmount();
  });

  it("exits selection mode from the toolbar's cancel", async () => {
    const view = mount(<ActivityRail {...props({ threads: twoThreads })} />);
    await flushAsync();
    await enterSelection(view.container);

    act(() => byLabel(view.container, "Cancel")!.click());
    expect(view.container.querySelector("[role=\"toolbar\"]")).toBeNull();
    expect(byLabel(view.container, "Select ws-a")).toBeNull();
    expect(byLabel(view.container, "Thread actions for ws-a")).not.toBeNull();
    view.unmount();
  });
});

describe("activity rail thread ordering", () => {
  it("hoists a pinned thread into its own section above the unpinned list", () => {
    const pinned = thread("pinned-one", { pinned: true });
    const plain = thread("plain-one");
    // The pinned thread is given *first* so the sort's comparator is asked to
    // place an unpinned thread relative to a pinned one — the arm that says
    // "not pinned, so it sorts after".
    const view = mount(<ActivityRail {...props({ threads: [pinned, plain] })} />);
    expect(view.container.textContent).toContain("Pinned");
    const titles = [...view.container.querySelectorAll("button[title]")]
      .map(button => button.getAttribute("title"))
      .filter(title => title === "pinned-one" || title === "plain-one");
    expect(titles).toEqual(["pinned-one", "plain-one"]);
    view.unmount();
  });

  it("sorts unpinned threads of one section by recency", () => {
    const older = thread("older", { updatedAt: 10 });
    const newer = thread("newer", { updatedAt: 20 });
    const view = mount(<ActivityRail {...props({ threads: [older, newer] })} />);
    const titles = [...view.container.querySelectorAll("button[title]")]
      .map(button => button.getAttribute("title"))
      .filter(title => title === "older" || title === "newer");
    expect(titles).toEqual(["newer", "older"]);
    view.unmount();
  });

  it("uses the last-message time over the row's update time", () => {
    // A thread touched by an approval write but whose last message is older must
    // sort by the message, not by the write.
    const justWritten = thread("just-written", { lastMessageAt: 5, updatedAt: 900 });
    const recentlyMessaged = thread("recently-messaged", { lastMessageAt: 500, updatedAt: 600 });
    const view = mount(<ActivityRail {...props({ threads: [justWritten, recentlyMessaged] })} />);
    const titles = [...view.container.querySelectorAll("button[title]")]
      .map(button => button.getAttribute("title"))
      .filter(title => title === "just-written" || title === "recently-messaged");
    expect(titles).toEqual(["recently-messaged", "just-written"]);
    view.unmount();
  });

  it("keeps archived threads out of the live list entirely", () => {
    const view = mount(
      <ActivityRail {...props({
        threads: [thread("live"), thread("gone", { status: "archived" })],
      })}
      />,
    );
    expect(view.container.querySelector("button[title=\"gone\"]")).toBeNull();
    expect(view.container.querySelector("button[title=\"live\"]")).not.toBeNull();
    view.unmount();
  });
});

describe("activity rail signed-out variants", () => {
  it("omits the remote entry from the collapsed rail while signed out", () => {
    const view = mount(<ActivityRail {...props({ expanded: false, futureSessionStatus: "signed_out" })} />);
    expect(byLabel(view.container, "Phone Control")).toBeNull();
    // The other collapsed entries are unaffected.
    expect(byLabel(view.container, "New chat")).not.toBeNull();
    expect(byLabel(view.container, "Chat")).not.toBeNull();
    view.unmount();
  });

  it("shows the remote entry while the account is only temporarily unverifiable", () => {
    const view = mount(<ActivityRail {...props({ expanded: false, futureSessionStatus: "unavailable" })} />);
    expect(byLabel(view.container, "Phone Control")).not.toBeNull();
    view.unmount();
  });

  it("treats a still-checking account with a known email as signed in", () => {
    // The pairing code needs a sign-in; a remembered address during the initial
    // profile check is enough to keep the entry reachable.
    const withEmail = mount(<ActivityRail {...props({ expanded: false, futureSessionStatus: "checking", userEmail: "alice@example.com" })} />);
    expect(byLabel(withEmail.container, "Phone Control")).not.toBeNull();
    withEmail.unmount();

    const withoutEmail = mount(<ActivityRail {...props({ expanded: false, futureSessionStatus: "checking", userEmail: null })} />);
    expect(byLabel(withoutEmail.container, "Phone Control")).toBeNull();
    withoutEmail.unmount();
  });

  it("keeps the expanded remote entry reachable for a checking account with an email", () => {
    const view = mount(<ActivityRail {...props({ futureSessionStatus: "checking", userEmail: "alice@example.com" })} />);
    expect(remoteEntry(view.container, true)).toBeTruthy();
    view.unmount();
  });
});

/**
 * The rail's last two dead branch arms are held dead by *invariants in this
 * component*, not by the type system:
 *
 * - `sortThreads`'s `if (a.status !== b.status)` arm can only run if two
 *   different statuses reach it, which the `status === "active"` filter at the
 *   call site prevents.
 * - the pinned section's `threadSelectionMode(thread) ? toggleThreadSelection :
 *   undefined` can only take its truthy side if a *pinned* thread is in batch
 *   scope, which `isThreadInScope` prevents.
 *
 * Those invariants used to live only in the waiver ledger's prose, where
 * deleting a filter would silently rot the waiver. These tests lock them: if
 * either filter goes away, a test fails instead of a waiver quietly becoming
 * wrong.
 */
describe("activity rail scope invariants (they hold the dead arms dead)", () => {
  it("lists only active threads, so the sort never compares two different statuses", () => {
    const view = mount(
      <ActivityRail {...props({
        threads: [
          thread("keep"),
          thread("archived-one", { status: "archived" }),
          thread("deleted-one", { status: "deleted" }),
        ],
      })}
      />,
    );

    const titles = [...view.container.querySelectorAll("button[title]")]
      .map(button => button.getAttribute("title"))
      .filter(title => title !== null);
    expect(titles).toContain("keep");
    // Only active threads are ever handed to the sort, so its status comparison
    // is always false.
    expect(titles).not.toContain("archived-one");
    expect(titles).not.toContain("deleted-one");
    view.unmount();
  });

  it("keeps pinned threads out of batch scope, so their row never offers a selection toggle", async () => {
    const pinned = thread("pinned-one", { pinned: true });
    const plain = thread("plain-one");
    const view = mount(<ActivityRail {...props({ threads: [pinned, plain] })} />);
    await flushAsync();

    // Both render — the pinned one in its own section above the other.
    expect(view.container.textContent).toContain("Pinned");

    // Enter batch selection over the whole chat list.
    act(() => byLabel(view.container, "Chat section actions")!.click());
    const item = [...view.container.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")]
      .find(button => button.textContent === "Select chats")!;
    act(() => item.click());
    await flushAsync();

    // Only the unpinned thread is scoped, so only it gets a checkbox: the
    // pinned row's `threadSelectionMode()` stays false. This is the user-visible
    // consequence of `isThreadInScope` excluding pinned threads — a pinned chat
    // cannot be swept up by a batch delete.
    const labels = [...view.container.querySelectorAll<HTMLInputElement>("input[type=\"checkbox\"][aria-label]")]
      .map(box => box.getAttribute("aria-label"));
    expect(labels).toContain("Select plain-one");
    expect(labels).not.toContain("Select pinned-one");

    // The scope is exactly the one unpinned thread: taking everything still
    // reports a single selection, so the pinned chat was never in scope.
    const toolbar = view.container.querySelector<HTMLElement>("[role=\"toolbar\"]")!;
    expect(toolbar.textContent).toContain("Select all");
    // Taking everything in the scope still reports a single selection, so the
    // pinned chat was never in scope.
    act(() => toolbar.querySelector<HTMLInputElement>("input[type=\"checkbox\"]")!.click());
    expect(toolbar.textContent).toContain("1 selected");
    view.unmount();
  });
});
