// @vitest-environment jsdom
import type { ComponentProps, ReactElement } from "react";
import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ActivityRail } from "./ActivityRail";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defaultAgentModelId } from "../../integrations/agent/agentClient";
import { AppShell } from "./AppShell";

/** `Required` so the (optional) handler props are callable without a null check. */
type RailProps = Required<ComponentProps<typeof ActivityRail>>;

/** The module-boundary spies the shell consumes, one entry per integration. */
interface HookSpies {
  createWorkspace: (input: unknown) => Promise<unknown>;
  installAgentEventListener: () => void;
  markThreadOpened: (threadId: string) => void;
  openExternalUrl: (url: string) => void;
  pinThread: (input: unknown) => void;
  pinWorkspace: (input: unknown) => void;
  reclamp: () => void;
  refreshAgentModels: () => void;
  refreshAuth: () => void;
  refreshBalance: () => void;
  refreshRemote: () => void;
  refreshSkills: () => void;
  restoreThread: (threadId: string) => Promise<unknown>;
  revalidateAgentState: (threadId: string) => void;
  saveComposerDraft: (key: string, draft: unknown) => void;
  setSelectedModelId: (modelId: string) => void;
  startNewConversation: (input: unknown) => Promise<void>;
  toggleTerminal: () => void;
  useAgentStatus: () => unknown;
}

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * Child components and hooks are stubbed so this file measures the shell's own
 * orchestration (state, handlers, conditional layout, event bridges).
 */
const children = vi.hoisted(() => ({
  activityRail: null as unknown,
  agentThread: null as unknown,
  appShellDialogs: null as unknown,
  contextPanel: null as unknown,
  newConversation: null as unknown,
  onboardingGate: null as unknown,
  remoteView: null as unknown,
  settingsDialog: null as unknown,
  skillsView: null as unknown,
  terminalPanel: null as unknown,
  workspaceDialogs: null as unknown,
}));

const mocks = vi.hoisted(() => ({
  changeSettings: vi.fn(async () => {}),
  currentSection: "chat" as string,
  decideApproval: vi.fn(async () => {}),
  emit: vi.fn(),
  futureEvents: {} as Record<string, (detail: unknown) => void>,
  geometry: { left: true, right: true },
  hooks: {} as HookSpies,
  nudgeLeftPanel: undefined as undefined | ReturnType<typeof vi.fn>,
  invoke: vi.fn(async (_command: string, _args?: unknown) => {}),
  refreshStore: vi.fn(async (_threadId?: string) => {}),
  remoteStatus: { phase: "idle" } as Record<string, unknown>,
  startRemote: vi.fn(async () => {}),
  stopRemote: vi.fn(async () => {}),
  store: {} as Record<string, unknown>,
  approvalsThreadIds: [] as (string | null)[],
  tauriEvents: {} as Record<string, (payload: unknown) => void>,
  terminalTarget: null as string | null,
}));

vi.mock("../../features/agent/AgentThread", async () => {
  const { createElement } = await import("react");
  return {
    AgentThread: (props: unknown) => {
      children.agentThread = props;
      return createElement("div", { "data-child": "agent-thread" });
    },
  };
});
vi.mock("../../features/agent/NewConversation", async () => {
  const { createElement } = await import("react");
  return {
    NewConversation: (props: unknown) => {
      children.newConversation = props;
      return createElement("div", { "data-child": "new-conversation" });
    },
  };
});
vi.mock("../../features/remote/RemoteView", async () => {
  const { createElement } = await import("react");
  return {
    RemoteView: (props: unknown) => {
      children.remoteView = props;
      return createElement("div", { "data-child": "remote-view" });
    },
  };
});
vi.mock("../../features/settings/SettingsDialog", async () => {
  const { createElement } = await import("react");
  return {
    SettingsDialog: (props: { open: boolean }) => {
      children.settingsDialog = props;
      return props.open ? createElement("div", { "data-child": "settings-dialog" }) : null;
    },
  };
});
vi.mock("../../features/skills/SkillsView", async () => {
  const { createElement } = await import("react");
  return {
    SkillsView: (props: unknown) => {
      children.skillsView = props;
      return createElement("div", { "data-child": "skills-view" });
    },
  };
});
vi.mock("../../features/terminal/TerminalPanel", async () => {
  const { createElement } = await import("react");
  return {
    TerminalPanel: (props: unknown) => {
      children.terminalPanel = props;
      return createElement("div", { "data-child": "terminal-panel" });
    },
  };
});
vi.mock("../../features/terminal/TerminalToggleButton", async () => {
  const { createElement } = await import("react");
  return { TerminalToggleButton: () => createElement("button", { "data-child": "terminal-toggle", "type": "button" }) };
});
vi.mock("./AgentStatusGate", async () => {
  const { createElement } = await import("react");
  return {
    AgentStatusGate: (props: { status: { phase: string } }) =>
      createElement("div", { "data-child": "agent-status-gate", "data-phase": props.status.phase }),
  };
});
vi.mock("./AppShellDialogs", async () => {
  const { createElement } = await import("react");
  return {
    AppShellDialogs: (props: unknown) => {
      children.appShellDialogs = props;
      return createElement("div", { "data-child": "app-shell-dialogs" });
    },
  };
});
vi.mock("./ContextPanel", async () => {
  const { createElement } = await import("react");
  return {
    ContextPanel: (props: unknown) => {
      children.contextPanel = props;
      return createElement("div", { "data-child": "context-panel" });
    },
  };
});
vi.mock("./OnboardingGate", async () => {
  const { createElement } = await import("react");
  return {
    OnboardingGate: (props: unknown) => {
      children.onboardingGate = props;
      return createElement("div", { "data-child": "onboarding-gate" });
    },
  };
});
vi.mock("./WorkspaceDialogs", async () => {
  const { createElement } = await import("react");
  return {
    WorkspaceDialogs: (props: unknown) => {
      children.workspaceDialogs = props;
      return createElement("div", { "data-child": "workspace-dialogs" });
    },
  };
});
vi.mock("./ActivityRail", async () => {
  const { createElement } = await import("react");
  return {
    ActivityRail: (props: Record<string, unknown>) => {
      children.activityRail = props;
      // The rail owns section switching; render the same affordances the real
      // one does so the shell's handlers are driven through the DOM.
      return createElement("nav", { "data-child": "activity-rail" }, [
        createElement("button", { key: "chat", onClick: () => (props.onChange as (s: string) => void)("chat"), type: "button" }, "rail:chat"),
        createElement("button", { key: "skill", onClick: () => (props.onChange as (s: string) => void)("skill"), type: "button" }, "rail:skill"),
        createElement("button", { key: "remote", onClick: () => (props.onChange as (s: string) => void)("remote"), type: "button" }, "rail:remote"),
        createElement("button", { key: "settings", onClick: () => (props.onChange as (s: string) => void)("settings"), type: "button" }, "rail:settings"),
      ]);
    },
  };
});

// ── shell-local hooks ────────────────────────────────────────────────────────
vi.mock("./hooks/panelGeometry", () => ({
  MIN_LEFT_PANEL_WIDTH: 224,
  canShowLeftPanel: () => mocks.geometry.left,
  canShowRightPanel: () => mocks.geometry.right,
}));
vi.mock("./hooks/useAgentStatus", () => ({ useAgentStatus: () => mocks.hooks.useAgentStatus?.() }));
vi.mock("./hooks/useAgentDoneBell", () => ({ useAgentDoneBell: vi.fn() }));
vi.mock("./hooks/useAppSettings", () => ({
  useAppSettings: () => ({ appSettings: (mocks.store.appSettings as Record<string, unknown>) ?? {}, changeSettings: mocks.changeSettings }),
}));
vi.mock("./hooks/useAppStartup", () => ({ useAppStartup: () => (mocks.store.startup as Record<string, unknown>) ?? { phase: "ready", auth: { status: "authenticated" }, providers: { builtin: [], custom: [] } } }));
vi.mock("./hooks/useAutoUpgradeSkills", () => ({ useAutoUpgradeSkills: vi.fn() }));
vi.mock("./hooks/useUpdateChecker", () => ({
  useUpdateChecker: () => ({ cachedStatus: (mocks.store.cachedStatus as unknown) ?? null, hasUpdate: Boolean(mocks.store.hasUpdate), markSeen: vi.fn() }),
}));
vi.mock("./hooks/useFutureAccount", () => ({
  useFutureAccount: () => ({
    balance: (mocks.store.balance as number | null) ?? null,
    balanceStatus: "idle",
    email: (mocks.store.email as string | null) ?? null,
    refreshAuth: () => mocks.hooks.refreshAuth?.(),
    refreshBalance: () => mocks.hooks.refreshBalance?.(),
    status: (mocks.store.futureSessionStatus as string) ?? "authenticated",
  }),
}));
vi.mock("./hooks/useHasProviders", () => ({ useHasProviders: () => (mocks.store.hasProviders as Record<string, unknown>) }));
vi.mock("./hooks/useWindowWidth", () => ({ useWindowWidth: () => (mocks.store.windowWidth as number) ?? 1400 }));
vi.mock("./hooks/useLeftPanelWidth", () => ({
  useLeftPanelWidth: () => ({ maxWidth: 400, nudge: (mocks.nudgeLeftPanel ??= vi.fn()), resizing: Boolean(mocks.store.leftResizing), startResize: vi.fn(), width: (mocks.store.leftWidth as number) ?? 288 }),
}));
vi.mock("./hooks/useRightPanelWidth", () => ({
  useRightPanelWidth: () => ({ nudge: vi.fn(), reclamp: mocks.hooks.reclamp, resizing: Boolean(mocks.store.rightResizing), startResize: vi.fn(), width: 360 }),
}));
vi.mock("./hooks/useAgentConnection", () => ({
  useAgentConnection: () => ({
    agentConnection: (mocks.store.agentConnection as unknown) ?? { status: "connected" },
    modelOptions: (mocks.store.modelOptions as unknown[]) ?? [],
    refreshAgentModels: mocks.hooks.refreshAgentModels,
    selectedModelId: (mocks.store.selectedModelId as string) ?? "future/m1",
    setSelectedModelId: mocks.hooks.setSelectedModelId,
    visibleModelOptions: (mocks.store.visibleModelOptions as unknown[]) ?? [],
  }),
}));
vi.mock("./hooks/useApprovals", () => ({
  useApprovals: (threadId: string | null = null) => {
    // Record the scope so a test can prove approvals are not left scoped to a
    // stale thread when nothing is selected.
    mocks.approvalsThreadIds.push(threadId);
    return { activeApproval: (mocks.store.activeApproval as unknown) ?? null, decideApproval: mocks.decideApproval, reloadApprovals: vi.fn() };
  },
}));
vi.mock("./hooks/useModelSelection", () => ({
  useModelSelection: () => ({
    activeThreadModelId: "future/m1",
    activeThinkingLevel: "medium",
    changeDraftModel: vi.fn(),
    changeDraftThinkingLevel: vi.fn(),
    changeModel: vi.fn(),
    changeThinkingLevel: vi.fn(),
    modelsEmptyReason: undefined,
    selectedThinkingLevel: "medium",
    syncSelection: vi.fn(),
  }),
}));
vi.mock("./hooks/useNewConversation", () => ({
  useNewConversation: () => ({
    consumePendingPrompt: vi.fn(),
    pendingPrompt: null,
    startNewConversation: mocks.hooks.startNewConversation,
  }),
}));
vi.mock("./hooks/useRemoteStatus", () => ({
  useRemoteStatus: () => ({ indicator: null, refresh: mocks.hooks.refreshRemote, status: mocks.remoteStatus }),
}));
vi.mock("./hooks/useThreadStore", () => ({ useThreadStore: () => mocks.store.threadStore }));
vi.mock("./hooks/useUnreadThreads", () => ({ useUnreadThreads: () => (mocks.store.unreadThreadIds as Set<string>) ?? new Set() }));
vi.mock("./hooks/useThreadDialogs", () => ({ useThreadDialogs: () => mocks.store.threadDialogs }));
vi.mock("./hooks/useWorkspaceDialogs", () => ({ useWorkspaceDialogs: () => mocks.store.workspaceDialogs }));
vi.mock("../../features/terminal/useTerminalPanel", () => ({
  useTerminalPanel: () => ({ open: Boolean(mocks.store.terminalOpen), shortcut: "mod+j", toggle: mocks.hooks.toggleTerminal }),
}));
vi.mock("../../features/terminal/panelTarget", () => ({ terminalTarget: () => mocks.terminalTarget }));

// ── integration layer ───────────────────────────────────────────────────────
vi.mock("../../integrations/agent/agentStateCache", () => ({
  installAgentEventListener: () => mocks.hooks.installAgentEventListener?.(),
  revalidateAgentState: (threadId: string) => mocks.hooks.revalidateAgentState?.(threadId),
}));
vi.mock("../../integrations/agent/agentClient", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../integrations/agent/agentClient")>();
  return {
    ...actual,
    modelOption: (id: string, options: { provider: string; id: string }[]) =>
      options.find(option => `${option.provider}/${option.id}` === id),
    readLastUsedModel: () => (mocks.store.lastUsedModel as string | null) ?? null,
  };
});
vi.mock("../../integrations/agent/providers", () => ({
  getFutureEnvironment: async () => ({ platformUrl: "https://future.example" }),
}));
vi.mock("../../integrations/skills/skillsClient", () => ({ refreshSkills: () => mocks.hooks.refreshSkills?.() }));
vi.mock("../../integrations/storage/files", () => ({ openExternalUrl: (url: string) => mocks.hooks.openExternalUrl?.(url) }));
vi.mock("../../integrations/storage/threadStore", () => ({
  createWorkspace: (input: { createDirectory: boolean; path: string }) => mocks.hooks.createWorkspace?.(input),
  markThreadOpened: (threadId: string) => mocks.hooks.markThreadOpened?.(threadId),
  pinThread: (input: unknown) => mocks.hooks.pinThread?.(input),
  pinWorkspace: (input: unknown) => mocks.hooks.pinWorkspace?.(input),
  restoreThread: (threadId: string) => mocks.hooks.restoreThread?.(threadId),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (command: string, args?: unknown) => mocks.invoke(command, args),
}));
vi.mock("../../features/remote/remoteClient", () => ({
  startRemote: (...args: unknown[]) => mocks.startRemote(...(args as [])),
  stopRemote: (...args: unknown[]) => mocks.stopRemote(...(args as [])),
}));
vi.mock("../../features/agent/composerDraft", () => ({
  saveComposerDraft: (key: string, draft: unknown) => mocks.hooks.saveComposerDraft?.(key, draft),
}));
vi.mock("../../lib/futureEvents", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/futureEvents")>();
  return {
    ...actual,
    emitFutureEvent: (...args: unknown[]) => mocks.emit(...args),
    onFutureEvent: (name: string, handler: (detail: unknown) => void) => {
      mocks.futureEvents[name] = handler;
      return () => {};
    },
  };
});
vi.mock("../../lib/useTauriEvent", () => ({
  useTauriEvent: (name: string, handler: (payload: unknown) => void) => {
    mocks.tauriEvents[name] = handler;
  },
}));

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: `Title ${id}`,
    mode: "chat",
    workspaceId: `w-${id}`,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

const chatThread = thread("t1");
const workThread = thread("t2", { mode: "workspace", workspaceId: "w-t2" });
const chatWorkspace = { id: "w-t1", kind: "user", name: "Chat", path: "/tmp/chat" } as unknown as StoredWorkspace;
const workWorkspace = { id: "w-t2", kind: "user", name: "Work", path: "/tmp/work" } as unknown as StoredWorkspace;

const threadDialogFns = {
  confirmBatchDelete: vi.fn(async () => {}),
  confirmDelete: vi.fn(async () => {}),
  confirmRename: vi.fn(async () => {}),
  generateTitle: vi.fn(async () => {}),
  openBatchDelete: vi.fn(),
  openDelete: vi.fn(),
  openRename: vi.fn(),
  setBatchDeleteDialog: vi.fn(),
  setDeleteDialog: vi.fn(),
  setRenameDialog: vi.fn(),
};
const workspaceDialogFns = {
  confirmDelete: vi.fn(async () => {}),
  confirmRename: vi.fn(async () => {}),
  openDelete: vi.fn(),
  openRename: vi.fn(),
  setDeleteDialog: vi.fn(),
  setRenameDialog: vi.fn(),
};

function resetStore(overrides: Record<string, unknown> = {}) {
  mocks.store = {
    activeApproval: null,
    activeThread: chatThread,
    activeThreadId: "t1",
    activeWorkspace: chatWorkspace,
    agentConnection: { status: "connected" },
    appSettings: {
      approvalTier: "off",
      autoUpgradeSkills: false,
      bellOnComplete: true,
      communityEdition: false,
      hiddenModels: [],
      skillGuideDismissed: false,
      skillIntroDismissed: true,
      skillRecommend: true,
    },
    balance: 12,
    email: "alice@example.com",
    futureSessionStatus: "authenticated",
    hasProviders: {
      byokMode: false,
      cancelLogin: vi.fn(),
      enableBYOK: vi.fn(),
      finishInit: vi.fn(),
      forceOnboarding: false,
      hasAnyProvider: true,
      initPending: false,
      showGate: false,
    },
    leftWidth: 288,
    modelOptions: [{ id: "m1", label: "M1", provider: "future" }],
    selectedModelId: "future/m1",
    startup: { auth: { status: "authenticated" }, phase: "ready", providers: { builtin: [], custom: [] } },
    threadDialogs: { ...threadDialogFns, batchDeleteDialog: null, deleteDialog: null, renameDialog: null },
    threadStore: {
      activeThread: chatThread,
      activeThreadId: "t1",
      activeWorkspace: chatWorkspace,
      loadingStore: false,
      refreshStore: mocks.refreshStore,
      setActiveThreadId: vi.fn(),
      storeError: null,
      threadRunStatuses: {},
      threadStreamingStatuses: {},
      threads: [chatThread],
      workspaces: [chatWorkspace],
    },
    unreadThreadIds: new Set<string>(),
    visibleModelOptions: [{ id: "m1", label: "M1", provider: "future" }],
    windowWidth: 1400,
    workspaceDialogs: { ...workspaceDialogFns, deleteDialog: null, renameDialog: null },
    ...overrides,
  };
}

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

const rail = () => children.activityRail as RailProps;
/** The bus/event maps are indexed lookups; a missing handler is a broken test. */
const bus = (name: string) => mocks.futureEvents[name]!;
const tauri = (name: string) => mocks.tauriEvents[name]!;
function railButton(container: HTMLElement, label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === label)!;
}

beforeEach(() => {
  // The child-prop captures are module-level, so a test that reads one without
  // first rendering that child would silently borrow whatever a *previous* test
  // left there. Clearing them turns that defect into an immediate failure
  // instead of a test that passes for the wrong reason (found via
  // `--sequence.shuffle`; see docs/testing/desktop-shell.md).
  for (const key of Object.keys(children))
    (children as Record<string, unknown>)[key] = null;
  mocks.approvalsThreadIds = [];
  mocks.changeSettings.mockClear();
  mocks.decideApproval.mockClear();
  mocks.emit.mockClear();
  mocks.futureEvents = {};
  mocks.geometry = { left: true, right: true };
  mocks.nudgeLeftPanel = undefined;
  mocks.invoke.mockReset().mockResolvedValue(undefined);
  mocks.hooks = {
    createWorkspace: vi.fn(async () => chatWorkspace),
    installAgentEventListener: vi.fn(),
    markThreadOpened: vi.fn(async () => {}),
    openExternalUrl: vi.fn(async () => {}),
    pinThread: vi.fn(async () => {}),
    pinWorkspace: vi.fn(async () => {}),
    reclamp: vi.fn(),
    refreshAgentModels: vi.fn(async () => {}),
    refreshRemote: vi.fn(),
    refreshAuth: vi.fn(),
    refreshBalance: vi.fn(),
    refreshSkills: vi.fn(async () => {}),
    restoreThread: vi.fn(async () => chatThread),
    revalidateAgentState: vi.fn(),
    saveComposerDraft: vi.fn(),
    setSelectedModelId: vi.fn(),
    startNewConversation: vi.fn(async () => {}),
    toggleTerminal: vi.fn(),
    useAgentStatus: () => ({ agentVersion: null, desktopVersion: "1.0", phase: "ready", showWait: false }),
  };
  mocks.remoteStatus = { phase: "idle" };
  mocks.startRemote.mockClear();
  mocks.stopRemote.mockClear();
  mocks.tauriEvents = {};
  mocks.terminalTarget = null;
  resetStore();
});

describe("app shell readiness gate", () => {
  it("shows the agent status page until the agent is ready", () => {
    mocks.hooks.useAgentStatus = () => ({ agentVersion: null, desktopVersion: "1.0", phase: "starting", showWait: true });
    const view = mount(<AppShell />);
    const gate = view.container.querySelector("[data-child=\"agent-status-gate\"]")!;
    expect(gate.getAttribute("data-phase")).toBe("starting");
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).toBeNull();
    view.unmount();
  });

  it("shows the status page while startup is pending and when it fails", () => {
    mocks.store.startup = { phase: "pending" };
    const pending = mount(<AppShell />);
    expect(pending.container.querySelector("[data-child=\"agent-status-gate\"]")!.getAttribute("data-phase")).toBe("starting");
    pending.unmount();

    mocks.store.startup = { phase: "failed" };
    const failed = mount(<AppShell />);
    expect(failed.container.querySelector("[data-child=\"agent-status-gate\"]")!.getAttribute("data-phase")).toBe("unavailable");
    failed.unmount();
  });

  it("renders the onboarding gate instead of the shell while the gate is up", () => {
    mocks.store.hasProviders = {
      ...(mocks.store.hasProviders as Record<string, unknown>),
      forceOnboarding: true,
      hasAnyProvider: false,
      initPending: true,
      showGate: true,
    };
    const view = mount(<AppShell />);
    expect(view.container.querySelector("[data-child=\"onboarding-gate\"]")).not.toBeNull();
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).toBeNull();
    expect(children.onboardingGate).toMatchObject({ autoLogin: true, hasAnyProvider: false, initPending: true });
    view.unmount();
  });
});

describe("app shell layout", () => {
  it("renders the thread view with rail, context panel and dialogs", () => {
    const view = mount(<AppShell />);
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).not.toBeNull();
    expect(view.container.querySelector("[data-child=\"agent-thread\"]")).not.toBeNull();
    expect(view.container.querySelector("[data-child=\"context-panel\"]")).not.toBeNull();
    expect(view.container.querySelector("[data-child=\"workspace-dialogs\"]")).not.toBeNull();
    expect(rail().active).toBe("chat");
    expect(rail().activeThreadId).toBe("t1");
    view.unmount();
  });

  it("renders with no thread or workspace selected, refreshing the store without an id", async () => {
    // A fresh install (and the moment after deleting the open thread) has no
    // active thread and no active workspace. Every `activeThread?.id ?? fallback`
    // in the shell has to degrade cleanly rather than keep pointing at the last
    // thread; this drives that state through the real component.
    const refreshStore = vi.fn(async (_threadId?: string) => {});
    resetStore({
      threadStore: {
        activeThread: null,
        activeThreadId: null,
        activeWorkspace: null,
        loadingStore: false,
        refreshStore,
        setActiveThreadId: vi.fn(),
        storeError: null,
        threadRunStatuses: {},
        threadStreamingStatuses: {},
        threads: [],
        workspaces: [],
      },
    });

    const view = mount(<AppShell />);

    // The shell still renders — \"nothing selected\" is a normal state, not an error.
    expect(view.container.querySelector("[data-child=\"agent-thread\"]")).not.toBeNull();
    expect(view.container.querySelector("[data-child=\"context-panel\"]")).not.toBeNull();
    // The rail is told there is no active thread (`key` falls back, not `undefined`).
    expect(rail().activeThreadId).toBeNull();

    // Approvals are scoped to *nothing* rather than to a stale thread id.
    expect(mocks.approvalsThreadIds).toContain(null);
    expect(mocks.approvalsThreadIds).not.toContain("t1");

    // The context panel is told there is no active thread to scope itself to.
    expect(children.contextPanel).toMatchObject({ activeThread: null });

    // Store refreshes issued while nothing is selected ask for the whole store.
    // Four handlers each carry an `activeThread?.id ?? undefined`: adding a
    // workspace, pinning a workspace group, deciding an approval, and the agent
    // view reporting activity. None of them may reach for a stale thread id.
    await act(async () => {
      await rail().onTogglePinWorkspace(workWorkspace);
    });
    await act(async () => {
      await (children.agentThread as { onApprovalDecision: (a: unknown, s: string) => Promise<void> })
        .onApprovalDecision({ id: "a1" }, "approved");
    });
    act(() => {
      (children.agentThread as { onThreadActivity: () => void }).onThreadActivity();
    });

    expect(refreshStore).toHaveBeenCalledWith(undefined);
    expect(refreshStore.mock.calls.every(([id]) => id === undefined)).toBe(true);

    // Adding a workspace from the new-conversation view with nothing selected
    // refreshes the whole store rather than a thread's slice.
    refreshStore.mockClear();
    act(() => rail().onNewChat());
    await act(async () => {
      await (children.newConversation as { onAddWorkspace: (input: unknown) => Promise<unknown> })
        .onAddWorkspace({ createDirectory: true, path: "/tmp/new" });
    });
    expect(refreshStore).toHaveBeenCalledWith(undefined);
    view.unmount();
  });

  it("folds the side panels away when the window is too narrow", () => {
    mocks.geometry = { left: false, right: false };
    const view = mount(<AppShell />);
    // Below the floor the rail is not rendered at all; only its hover edge is.
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).toBeNull();
    const edge = view.container.querySelector<HTMLElement>("[aria-hidden=\"true\"]")!;
    expect(edge.className).toContain("cursor-col-resize");
    act(() => edge.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).not.toBeNull();
    // The conversation yields to the panel: no context panel either.
    expect(view.container.querySelector("[data-child=\"context-panel\"]")).toBeNull();
    view.unmount();
  });

  it("ignores a stale hover edge that outlives its own collapsed layout", () => {
    // Falsification attempt for the `if (showLeftPanel) return;` guard in
    // `handlePreviewLeftPanel`. The hook's `showLeftPanel` is captured per render,
    // and both places that call the handler live inside `!showLeftPanel` blocks,
    // so the guard looks untakeable. The one way it *could* be reachable is a
    // hover event delivered to an edge node that has already been detached by the
    // layout flipping back to expanded. So: capture the edge while collapsed,
    // expand (which removes it), dispatch on the detached node, then collapse
    // again — and check the preview did not silently arm itself for later.
    mocks.geometry = { left: false, right: true };
    const view = mount(<AppShell />);
    const edge = view.container.querySelector<HTMLElement>("[aria-hidden=\"true\"]")!;
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).toBeNull();

    // Expand: the edge node is removed from the document in this same commit.
    mocks.geometry = { left: true, right: true };
    view.rerender(<AppShell />);
    expect(edge.isConnected).toBe(false);
    expect(view.container.querySelector("[aria-hidden=\"true\"]")).toBeNull();

    // Deliver a hover to the detached node.
    act(() => edge.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));

    // Collapse again. If the detached dispatch had reached the handler and set
    // `leftOverlayOpen`, the preview rail would now be on screen uninvited.
    mocks.geometry = { left: false, right: true };
    view.rerender(<AppShell />);
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).toBeNull();

    // A live edge still works, so the assertion above is about staleness, not
    // about the preview being broken.
    const freshEdge = view.container.querySelector<HTMLElement>("[aria-hidden=\"true\"]")!;
    act(() => freshEdge.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).not.toBeNull();
    view.unmount();
  });

  it("previews the collapsed rail on hover and drops it again on leave", () => {
    mocks.geometry = { left: false, right: true };
    const view = mount(<AppShell />);
    const edge = view.container.querySelector<HTMLElement>("[aria-hidden=\"true\"]")!;
    act(() => edge.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    const floating = view.container.querySelector<HTMLElement>("[data-child=\"activity-rail\"]")!;
    expect(rail().floating).toBe(true);

    act(() => floating.parentElement!.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })));
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).toBeNull();

    // Below the two-column floor the toggle must not record "collapsed" as the
    // user's preference (checked here through the handler it wires).
    act(() => edge.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    act(() => rail().onToggleExpanded());
    expect(rail().expanded).toBe(true);
    view.unmount();
  });

  it("does not refresh skills or revalidate state while the agent connection is not connected", () => {
    // The reconnect effect exists to re-sync after a restart. While the
    // connection is anything but `connected` it must stay quiet: no skills
    // refresh and no forced `get_state` revalidation (which would overwrite a
    // draft model pick with the selected model's default).
    mocks.store.agentConnection = { status: "connecting" };
    const view = mount(<AppShell />);

    // `refreshSkills` is also triggered by a mount-time effect, so the
    // connection-gated effect is isolated by its own observable: the forced
    // `get_state` revalidation, which lives only inside that block.
    expect(mocks.hooks.revalidateAgentState).not.toHaveBeenCalled();
    const connectingCalls = (mocks.hooks.refreshSkills as ReturnType<typeof vi.fn>).mock.calls.length;
    // The shell still renders — a non-connected agent is a normal state.
    expect(view.container.querySelector("[data-child=\"agent-thread\"]")).not.toBeNull();

    // …and once it connects, the same effect does the sync exactly once.
    mocks.store.agentConnection = { status: "connected" };
    view.rerender(<AppShell />);
    expect((mocks.hooks.refreshSkills as ReturnType<typeof vi.fn>).mock.calls.length).toBe(connectingCalls + 1);
    expect(mocks.hooks.revalidateAgentState).toHaveBeenCalledWith("t1");
    view.unmount();
  });

  it("ignores non-arrow keys on the left panel's resize divider", () => {
    // The divider is focusable and nudges the panel width with the arrow keys.
    // Any other key must be left alone — no `preventDefault`, no width change.
    const view = mount(<AppShell />);
    const divider = view.container.querySelector<HTMLElement>("[tabindex=\"0\"][class*=cursor-col-resize]")
      ?? [...view.container.querySelectorAll<HTMLElement>("[tabindex=\"0\"]")]
        .find(node => node.className.includes("cursor-col-resize"))!;

    act(() => {
      divider.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "a" }));
    });
    expect(mocks.nudgeLeftPanel).not.toHaveBeenCalled();

    act(() => {
      divider.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowRight" }));
    });
    expect(mocks.nudgeLeftPanel).toHaveBeenCalledWith(16);
    view.unmount();
  });

  it("shows the resize overlays while a panel drag is in flight", () => {
    mocks.store.leftResizing = true;
    mocks.store.rightResizing = true;
    const view = mount(<AppShell />);
    const overlays = view.container.querySelectorAll("div.fixed.inset-0.z-50");
    expect(overlays.length).toBe(2);
    view.unmount();
  });

  it("surfaces a store failure in place of the conversation", () => {
    mocks.store.threadStore = { ...(mocks.store.threadStore as Record<string, unknown>), storeError: "database is locked" };
    const view = mount(<AppShell />);
    expect(view.container.textContent).toContain("database is locked");
    expect(view.container.querySelector("[data-child=\"agent-thread\"]")).toBeNull();
    view.unmount();
  });

  it("hides the right panel for the skill and remote sections", () => {
    const view = mount(<AppShell />);
    act(() => railButton(view.container, "rail:skill").click());
    expect(view.container.querySelector("[data-child=\"skills-view\"]")).not.toBeNull();
    expect(view.container.querySelector("[data-child=\"context-panel\"]")).toBeNull();

    act(() => railButton(view.container, "rail:remote").click());
    expect(view.container.querySelector("[data-child=\"remote-view\"]")).not.toBeNull();
    expect(view.container.querySelector("[data-child=\"context-panel\"]")).toBeNull();
    view.unmount();
  });

  it("opens Settings on its own tab from the rail's settings entry", () => {
    const view = mount(<AppShell />);
    act(() => railButton(view.container, "rail:settings").click());
    expect(children.settingsDialog).toMatchObject({ initialTab: "general", open: true });
    act(() => (children.settingsDialog as { onClose: () => void }).onClose());
    expect((children.settingsDialog as { open: boolean }).open).toBe(false);
    view.unmount();
  });

  it("mounts the terminal panel only for a real conversation", () => {
    mocks.terminalTarget = "t1";
    mocks.store.terminalOpen = true;
    const view = mount(<AppShell />);
    expect(view.container.querySelector("[data-child=\"terminal-panel\"]")).not.toBeNull();
    expect(children.terminalPanel).toMatchObject({ threadId: "t1" });
    expect(children.agentThread).toMatchObject({ headerAction: expect.anything() });
    view.unmount();

    mocks.terminalTarget = null;
    const hidden = mount(<AppShell />);
    expect(hidden.container.querySelector("[data-child=\"terminal-panel\"]")).toBeNull();
    expect((children.agentThread as { headerAction: unknown }).headerAction).toBeNull();
    hidden.unmount();
  });

  it("opens the new-conversation view from the rail and returns to the thread", () => {
    const view = mount(<AppShell />);
    act(() => rail().onNewChat());
    expect(view.container.querySelector("[data-child=\"new-conversation\"]")).not.toBeNull();
    expect(view.container.querySelector("[data-child=\"context-panel\"]")).toBeNull();
    expect(children.newConversation).toMatchObject({ initialMode: "chat", initialWorkspaceId: null });

    act(() => railButton(view.container, "rail:chat").click());
    expect(view.container.querySelector("[data-child=\"agent-thread\"]")).not.toBeNull();
    view.unmount();
  });

  it("opens the new-conversation view bound to a workspace", () => {
    const view = mount(<AppShell />);
    act(() => rail().onNewChat("w-t2"));
    expect(children.newConversation).toMatchObject({ initialMode: "workspace", initialWorkspaceId: "w-t2" });
    expect(rail().active).toBe("workspace");
    view.unmount();
  });

  it("reopens the create-workspace form on every workspace '+' click", () => {
    const view = mount(<AppShell />);
    act(() => rail().onNewWorkspace());
    expect(children.newConversation).toMatchObject({ initialWorkspaceForm: "open" });
    const firstKey = (view.container.querySelector("[data-child=\"new-conversation\"]") as HTMLElement).getAttribute("data-child");
    act(() => rail().onNewWorkspace());
    // The remount nonce is part of the key; the view is recreated rather than
    // left with a cancelled dialog.
    expect(firstKey).toBe("new-conversation");
    expect(children.newConversation).toMatchObject({ initialWorkspaceForm: "open" });
    view.unmount();
  });
});

describe("app shell workspace handlers", () => {
  it("creates a workspace and refreshes the store onto the active thread", async () => {
    const view = mount(<AppShell />);
    // Arrange: the composer's props only exist once the new-conversation view is
    // actually mounted. Without this the test read `children.newConversation`
    // left over from a *previous* test — so it passed for the wrong reason (and
    // failed under `--sequence.shuffle`). It now exercises its own render.
    act(() => rail().onNewChat());
    const created = await (children.newConversation as { onAddWorkspace: (input: unknown) => Promise<unknown> }).onAddWorkspace({ createDirectory: true, path: "/tmp/new" });
    expect((mocks.hooks.createWorkspace as ReturnType<typeof vi.fn>)).toHaveBeenCalledWith({ createDirectory: true, path: "/tmp/new" });
    expect(mocks.refreshStore).toHaveBeenCalledWith("t1");
    expect(created).toBe(chatWorkspace);
    view.unmount();
  });

  it("toggles a thread and a workspace pin, refreshing after each write", async () => {
    const view = mount(<AppShell />);
    await act(async () => {
      await rail().onTogglePinThread(chatThread);
    });
    expect(mocks.hooks.pinThread).toHaveBeenCalledWith({ pinned: true, threadId: "t1" });
    expect(mocks.refreshStore).toHaveBeenLastCalledWith("t1");

    await act(async () => {
      await rail().onTogglePinWorkspace(chatWorkspace);
    });
    expect(mocks.hooks.pinWorkspace).toHaveBeenCalledWith({ pinned: true, workspaceId: "w-t1" });
    expect(mocks.refreshStore).toHaveBeenLastCalledWith("t1");
    view.unmount();
  });

  it("unpins an already pinned thread", async () => {
    const view = mount(<AppShell />);
    await act(async () => {
      await rail().onTogglePinThread(thread("t9", { pinned: true }));
    });
    expect(mocks.hooks.pinThread).toHaveBeenCalledWith({ pinned: false, threadId: "t9" });
    view.unmount();
  });

  it("restores an archived thread onto its section", async () => {
    (mocks.hooks.restoreThread as ReturnType<typeof vi.fn>).mockResolvedValue(thread("t3", { mode: "workspace" }));
    const view = mount(<AppShell />);
    await act(async () => {
      await rail().onRestoreThread(thread("t3"));
    });
    expect(mocks.hooks.restoreThread).toHaveBeenCalledWith("t3");
    expect(mocks.refreshStore).toHaveBeenCalledWith("t3");
    expect(rail().active).toBe("workspace");
    view.unmount();
  });

  it("restores an archived chat thread back onto the chat section", async () => {
    // The sibling test above restores a *workspace* thread; a chat-mode restore
    // takes the other side of `restoredThread.mode === "workspace" ? … : …` and
    // must land on the chat section rather than the workspace one.
    (mocks.hooks.restoreThread as ReturnType<typeof vi.fn>).mockResolvedValue(thread("t4", { mode: "chat" }));
    const view = mount(<AppShell />);
    act(() => rail().onChange("workspace"));
    expect(rail().active).toBe("workspace");

    await act(async () => {
      await rail().onRestoreThread(thread("t4"));
    });
    expect(mocks.hooks.restoreThread).toHaveBeenCalledWith("t4");
    expect(rail().active).toBe("chat");
    view.unmount();
  });

  it("selects a thread, routing workspace threads to the workspace section", () => {
    const view = mount(<AppShell />);
    act(() => rail().onSelectThread(chatThread));
    expect(rail().active).toBe("chat");
    act(() => rail().onSelectThread(workThread));
    expect(rail().active).toBe("workspace");
    view.unmount();
  });

  it("selects a workspace's latest thread, or clears the selection when it is empty", () => {
    const view = mount(<AppShell />);
    const setActiveThreadId = (mocks.store.threadStore as { setActiveThreadId: ReturnType<typeof vi.fn> }).setActiveThreadId;

    act(() => rail().onSelectWorkspace(workWorkspace, [workThread]));
    expect(setActiveThreadId).toHaveBeenLastCalledWith("t2");
    expect(rail().active).toBe("workspace");

    act(() => rail().onSelectWorkspace(workWorkspace, []));
    expect(setActiveThreadId).toHaveBeenLastCalledWith(null);
    view.unmount();
  });

  it("records an approval decision and refreshes", async () => {
    const approval = { id: "a1", threadId: "t1" };
    const view = mount(<AppShell />);
    await act(async () => {
      await (children.agentThread as { onApprovalDecision: (a: unknown, s: string) => Promise<void> })
        .onApprovalDecision(approval, "approved");
    });
    expect(mocks.decideApproval).toHaveBeenCalledWith(approval, "approved");
    expect(mocks.refreshStore).toHaveBeenCalledWith("t1");
    view.unmount();
  });

  it("forks and refreshes onto the forked thread, and refreshes on thread activity", async () => {
    const view = mount(<AppShell />);
    const threadProps = children.agentThread as {
      onForked: (id: string) => void;
      onThreadActivity: () => void;
      onPromptConsumed: (id: string) => void;
      onRetryAgentConnection: () => void;
      onOpenAccount: () => void;
      onOpenModels: () => void;
      onOpenProviders: () => void;
      onChangeApprovalTier: (tier: string) => void;
    };
    act(() => threadProps.onForked("t7"));
    expect(mocks.refreshStore).toHaveBeenCalledWith("t7");
    act(() => threadProps.onThreadActivity());
    expect(mocks.refreshStore).toHaveBeenCalledWith("t1");
    act(() => threadProps.onPromptConsumed("p1"));
    act(() => threadProps.onRetryAgentConnection());
    expect(mocks.hooks.refreshAgentModels).toHaveBeenCalledTimes(1);
    act(() => threadProps.onChangeApprovalTier("manual"));
    expect(mocks.changeSettings).toHaveBeenCalledWith({ approvalTier: "manual" });
    act(() => threadProps.onOpenAccount());
    expect(children.settingsDialog).toMatchObject({ initialTab: "account", open: true });
    act(() => threadProps.onOpenModels());
    expect(children.settingsDialog).toMatchObject({ initialTab: "models" });
    act(() => threadProps.onOpenProviders());
    expect(children.settingsDialog).toMatchObject({ initialTab: "providers" });
    view.unmount();
  });

  it("dismisses the skill intro through the persisted setting", () => {
    const view = mount(<AppShell />);
    act(() => rail().onDismissSkillIntro());
    expect(mocks.changeSettings).toHaveBeenCalledWith({ skillIntroDismissed: true });
    view.unmount();
  });

  it("routes the account recharge and the update entry", async () => {
    const view = mount(<AppShell />);
    await act(async () => {
      rail().onRecharge();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.hooks.openExternalUrl).toHaveBeenCalledWith("https://future.example/platform/#recharge");

    act(() => rail().onOpenUpdate());
    expect(children.settingsDialog).toMatchObject({ initialTab: "update", open: true });
    view.unmount();
  });

  it("starts the coach conversation with the composer's current pick", async () => {
    const view = mount(<AppShell />);
    act(() => railButton(view.container, "rail:skill").click());
    await act(async () => {
      await (children.skillsView as { onStartCoachConversation: (c: string) => Promise<void> })
        .onStartCoachConversation("teach me");
    });
    expect(mocks.hooks.startNewConversation).toHaveBeenCalledWith({
      content: "teach me",
      mode: "chat",
      modelId: "future/m1",
      thinkingLevel: "medium",
    });
    view.unmount();
  });

  it("falls back to the agent's default model for the coach conversation when none is picked", async () => {
    // With no model selected, the coach conversation is created against
    // `defaultAgentModelId` rather than sending the empty selection through as
    // if it were a choice. (That constant is "" today, meaning \"let the agent
    // decide\", so the contract asserted here is the *precedence*: an empty
    // selection yields the default, never the empty string as a picked model.)
    resetStore({ selectedModelId: "" });
    const view = mount(<AppShell />);
    act(() => railButton(view.container, "rail:skill").click());
    await act(async () => {
      await (children.skillsView as { onStartCoachConversation: (c: string) => Promise<void> })
        .onStartCoachConversation("teach me");
    });
    const call = (mocks.hooks.startNewConversation as ReturnType<typeof vi.fn>).mock.calls[0]![0] as { modelId?: string };
    expect(call.modelId).toBe(defaultAgentModelId);
    // And the rest of the payload is unaffected by the missing pick.
    expect(call).toMatchObject({ content: "teach me", mode: "chat" });
    view.unmount();
  });

  it("pre-fills the new-chat draft when a skill is tried from the Skills page", () => {
    const view = mount(<AppShell />);
    act(() => railButton(view.container, "rail:skill").click());
    act(() => (children.skillsView as { onTrySkill: (name: string) => void }).onTrySkill("deep-research"));
    expect(mocks.hooks.saveComposerDraft).toHaveBeenCalledWith("new", { text: "/deep-research " });
    expect(view.container.querySelector("[data-child=\"new-conversation\"]")).not.toBeNull();
    view.unmount();
  });

  it("reuses the visible model options for the new-conversation composer", () => {
    const view = mount(<AppShell />);
    act(() => railButton(view.container, "rail:skill").click());
    act(() => (children.skillsView as { onTrySkill: (name: string) => void }).onTrySkill("x"));
    expect(children.newConversation).toMatchObject({
      modelId: "future/m1",
      modelOptions: [{ id: "m1", label: "M1", provider: "future" }],
      thinkingLevel: "medium",
    });
    view.unmount();
  });
});

describe("app shell event bridges", () => {
  it("observes the active agent session and refreshes the store when the cwd moves", async () => {
    const view = mount(<AppShell />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith("observe_session", { sessionId: "t1", threadId: "t1" });
    expect(mocks.hooks.markThreadOpened).toHaveBeenCalledWith("t1");

    mocks.refreshStore.mockClear();
    await act(async () => {
      window.dispatchEvent(new Event("future:cwd-changed"));
      await Promise.resolve();
    });
    expect(mocks.refreshStore).toHaveBeenCalled();
    view.unmount();
  });

  it("re-scans skills on startup and again whenever the agent reconnects", async () => {
    const view = mount(<AppShell />);
    await act(async () => {
      await Promise.resolve();
    });
    // Once for the startup scan, once for the connected transition.
    expect((mocks.hooks.refreshSkills as ReturnType<typeof vi.fn>).mock.calls.length).toBeGreaterThanOrEqual(1);
    expect(mocks.hooks.revalidateAgentState).toHaveBeenCalledWith("t1");
    expect(mocks.hooks.installAgentEventListener).toHaveBeenCalledTimes(1);

    // A new thread has no authoritative agent state yet, so its draft cache
    // must not be force-revalidated.
    const revalidate = vi.fn();
    const fresh = thread("t9", { agentSessionId: null as unknown as string });
    mocks.store.threadStore = { ...(mocks.store.threadStore as Record<string, unknown>), activeThread: fresh, activeThreadId: "t9" };
    view.unmount();
    mocks.hooks.revalidateAgentState = revalidate;
    const other = mount(<AppShell />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(revalidate).not.toHaveBeenCalled();
    other.unmount();
  });

  it("adopts the model the onboarding gate picked", () => {
    mocks.store.lastUsedModel = "future/m1";
    const view = mount(<AppShell />);
    const setSelectedModelId = mocks.hooks.setSelectedModelId as ReturnType<typeof vi.fn>;
    setSelectedModelId.mockClear();
    act(() => bus("future-models-synced")(undefined));
    expect(setSelectedModelId).toHaveBeenCalledWith("future/m1");
    view.unmount();
  });

  it("ignores a last-used model that the catalog no longer offers", () => {
    mocks.store.lastUsedModel = "future/gone";
    const view = mount(<AppShell />);
    const setSelectedModelId = mocks.hooks.setSelectedModelId as ReturnType<typeof vi.fn>;
    setSelectedModelId.mockClear();
    act(() => bus("future-models-synced")(undefined));
    expect(setSelectedModelId).not.toHaveBeenCalled();
    view.unmount();
  });

  it("bridges the backend's catalogue refresh into the frontend invalidation path", () => {
    const view = mount(<AppShell />);
    act(() => tauri("scheduler-future-models")({ synced: true }));
    expect(mocks.emit).toHaveBeenCalledWith("future-models-synced", undefined);
    mocks.emit.mockClear();
    act(() => tauri("scheduler-future-models")({ synced: false }));
    expect(mocks.emit).not.toHaveBeenCalled();
    view.unmount();
  });

  it("refreshes the thread list from the backend's own events", async () => {
    const view = mount(<AppShell />);
    mocks.refreshStore.mockClear();
    await act(async () => {
      tauri("threads-updated")(undefined);
      tauri("remote-activity")(undefined);
      await Promise.resolve();
    });
    expect(mocks.refreshStore).toHaveBeenCalledTimes(2);
    act(() => tauri("skills_changed")(undefined));
    expect(mocks.emit).toHaveBeenCalledWith("skills-changed", undefined);
    act(() => tauri("review-updated")("t1"));
    expect(mocks.emit).toHaveBeenCalledWith("review-updated", { threadId: "t1" });
    view.unmount();
  });

  it("opens Settings on the About tab from the macOS menu event", () => {
    const view = mount(<AppShell />);
    act(() => tauri("open-settings")(undefined));
    expect(children.settingsDialog).toMatchObject({ initialTab: "about", open: true });
    view.unmount();
  });

  it("opens Settings on Providers when the user chose bring-your-own-key", () => {
    mocks.store.hasProviders = { ...(mocks.store.hasProviders as Record<string, unknown>), byokMode: true };
    const view = mount(<AppShell />);
    expect(children.settingsDialog).toMatchObject({ initialTab: "providers", open: true });
    view.unmount();
  });
});

describe("app shell account + remote coordination", () => {
  it("drops back to chat when the account signs out while on Phone Control", () => {
    const view = mount(<AppShell />);
    act(() => railButton(view.container, "rail:remote").click());
    expect(rail().active).toBe("remote");

    mocks.store.futureSessionStatus = "signed_out";
    view.rerender(<AppShell />);
    expect(rail().active).toBe("chat");
    view.unmount();
  });

  it("stops the remote bridge when the account is signed out", async () => {
    const view = mount(<AppShell />);
    await act(async () => {
      await Promise.resolve();
    });
    mocks.store.futureSessionStatus = "invalid";
    await act(async () => {
      view.rerender(<AppShell />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.stopRemote).toHaveBeenCalled();
    view.unmount();
  });

  it("re-checks the account when remote reports an authorization failure", async () => {
    mocks.remoteStatus = { phase: "failed", reason: "account_authorization" };
    const view = mount(<AppShell />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(mocks.hooks.refreshAuth).toHaveBeenCalled();
    view.unmount();
  });

  it("retries the persisted pairing once a reauthentication succeeds", async () => {
    mocks.remoteStatus = { pairId: "pair-1", phase: "failed", reason: "account_authorization" };
    mocks.store.futureSessionStatus = "checking";
    const view = mount(<AppShell />);

    // Checking → authenticated is the reauthentication signal.
    mocks.store.futureSessionStatus = "authenticated";
    await act(async () => {
      view.rerender(<AppShell />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.startRemote).toHaveBeenCalledWith({});
    expect(mocks.hooks.refreshRemote).toHaveBeenCalled();
    view.unmount();
  });

  it("keeps a stale remote failure untouched when no pairing exists to retry", async () => {
    mocks.remoteStatus = { pairId: null, phase: "failed", reason: "account_authorization" };
    mocks.store.futureSessionStatus = "checking";
    const view = mount(<AppShell />);
    mocks.store.futureSessionStatus = "authenticated";
    await act(async () => {
      view.rerender(<AppShell />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.startRemote).not.toHaveBeenCalled();
    view.unmount();
  });
});

describe("app shell dialogs wiring", () => {
  it("opens the rename dialog and confirms through the thread-dialog hook", async () => {
    const view = mount(<AppShell />);
    act(() => rail().onRenameThread(chatThread));
    expect(threadDialogFns.openRename).toHaveBeenCalledWith(chatThread);
    act(() => rail().onDeleteThread(chatThread));
    expect(threadDialogFns.openDelete).toHaveBeenCalledWith(chatThread);
    act(() => rail().onBatchDeleteThreads([chatThread]));
    expect(threadDialogFns.openBatchDelete).toHaveBeenCalledWith([chatThread]);
    act(() => rail().onRenameWorkspace(chatWorkspace));
    expect(workspaceDialogFns.openRename).toHaveBeenCalledWith(chatWorkspace);
    act(() => rail().onDeleteWorkspace(chatWorkspace));
    expect(workspaceDialogFns.openDelete).toHaveBeenCalledWith(chatWorkspace);
    view.unmount();
  });

  it("forwards every dialog confirmation to its hook", async () => {
    const view = mount(<AppShell />);
    const dialogs = children.appShellDialogs as {
      onConfirmBatchDeleteThread: () => void;
      onConfirmDeleteThread: () => void;
      onConfirmRenameThread: () => void;
      onGenerateTitle: () => void;
    };
    await act(async () => {
      dialogs.onConfirmBatchDeleteThread();
      dialogs.onConfirmDeleteThread();
      dialogs.onConfirmRenameThread();
      dialogs.onGenerateTitle();
      await Promise.resolve();
    });
    expect(threadDialogFns.confirmBatchDelete).toHaveBeenCalled();
    expect(threadDialogFns.confirmDelete).toHaveBeenCalled();
    expect(threadDialogFns.confirmRename).toHaveBeenCalled();
    expect(threadDialogFns.generateTitle).toHaveBeenCalled();

    const workspaceDialogs = children.workspaceDialogs as {
      onConfirmDeleteWorkspace: () => void;
      onConfirmRenameWorkspace: () => void;
    };
    await act(async () => {
      workspaceDialogs.onConfirmDeleteWorkspace();
      workspaceDialogs.onConfirmRenameWorkspace();
      await Promise.resolve();
    });
    expect(workspaceDialogFns.confirmDelete).toHaveBeenCalled();
    expect(workspaceDialogFns.confirmRename).toHaveBeenCalled();
    view.unmount();
  });
});

describe("app shell collapsed-panel affordances", () => {
  it("shows the rail only as a hover overlay while collapsed", () => {
    mocks.geometry = { left: false, right: true };
    const view = mount(<AppShell />);
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).toBeNull();
    const edge = view.container.querySelector<HTMLElement>("[aria-hidden=\"true\"]")!;
    expect(edge).not.toBeNull();
    act(() => edge.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(view.container.querySelectorAll("[data-child=\"activity-rail\"]").length).toBe(1);
    view.unmount();
  });

  it("keeps the expanded rail when the window can hold it", () => {
    const view = mount(<AppShell />);
    expect(view.container.querySelector("[aria-hidden=\"true\"]")).toBeNull();
    expect(view.container.querySelector("[role=\"separator\"]")).not.toBeNull();
    view.unmount();
  });

  it("exposes the left resize divider as a coherent ARIA separator", () => {
    // The divider is not only a drag handle: it carries the ARIA separator
    // pattern (role, orientation, value range, focusability), which is what makes
    // it operable without a pointer. The earlier test asserted `min` and `now`
    // individually; what it could not catch is a **broken range** — the classic
    // slider bug where `now` falls outside `[min, max]`. So the invariant is
    // asserted, not just the attributes, plus `aria-valuemax` (which tracks the
    // panel hook's computed ceiling rather than a hard-coded number) and
    // focusability.
    const view = mount(<AppShell />);
    const separator = view.container.querySelector<HTMLElement>("[role=\"separator\"]")!;
    const min = Number(separator.getAttribute("aria-valuemin"));
    const max = Number(separator.getAttribute("aria-valuemax"));
    const now = Number(separator.getAttribute("aria-valuenow"));

    expect([min, max, now].every(Number.isFinite)).toBe(true);
    expect(min).toBeLessThanOrEqual(now);
    expect(now).toBeLessThanOrEqual(max);
    // The ceiling is the hook's computed max for this window (400 in this mock),
    // so a regression to a hard-coded/absent value shows up here.
    expect(max).toBe(400);
    // Focusable, or the arrow-key nudge is unreachable by keyboard.
    expect(separator.tabIndex).toBe(0);
    separator.focus();
    expect(document.activeElement).toBe(separator);
    expect(separator.getAttribute("aria-orientation")).toBe("vertical");
    view.unmount();
  });

  it("collapses the rail from its own toggle and reopens it from the hover edge", () => {
    const leftPanel = mocks.store as Record<string, unknown>;
    void leftPanel;
    const view = mount(<AppShell />);
    const separator = view.container.querySelector<HTMLElement>("[role=\"separator\"]")!;
    expect(separator.getAttribute("aria-valuemin")).toBe("224");
    expect(separator.getAttribute("aria-valuenow")).toBe("288");

    // Keyboard resizing goes through the panel hook's nudge with a signed delta.
    act(() => {
      separator.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowRight" }));
    });
    expect(children.contextPanel).toBeTruthy();
    act(() => {
      separator.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowLeft" }));
    });
    expect(separator.getAttribute("aria-orientation")).toBe("vertical");

    // The rail's own toggle collapses it; the hover edge then reopens the
    // floating preview instead of the inline column.
    act(() => rail().onToggleExpanded());
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).toBeNull();
    const edge = view.container.querySelector<HTMLElement>("[aria-hidden=\"true\"]")!;
    act(() => edge.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(rail().floating).toBe(true);
    // The floating rail keeps the hover alive through its own mouseenter.
    act(() => view.container.querySelector<HTMLElement>("[data-child=\"activity-rail\"]")!.parentElement!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(rail().floating).toBe(true);
    view.unmount();
  });

  it("forwards the composer's tier and guide-dismissal edits to the settings write", () => {
    const view = mount(<AppShell />);
    act(() => rail().onNewChat());
    const composer = children.newConversation as {
      onChangeApprovalTier: (tier: string) => void;
      onDismissSkillGuide: () => void;
    };
    act(() => composer.onChangeApprovalTier("sandbox"));
    expect(mocks.changeSettings).toHaveBeenCalledWith({ approvalTier: "sandbox" });
    act(() => composer.onDismissSkillGuide());
    expect(mocks.changeSettings).toHaveBeenCalledWith({ skillGuideDismissed: true });
    view.unmount();
  });

  it("forwards the remote view's settings and refresh callbacks", () => {
    const view = mount(<AppShell />);
    act(() => railButton(view.container, "rail:remote").click());
    const remote = children.remoteView as {
      onChangeSettings: (patch: Record<string, unknown>) => void;
      onRefreshRemote: () => void;
      onToggleLeftPanel: () => void;
    };
    act(() => remote.onChangeSettings({ autoConnectRemote: true }));
    expect(mocks.changeSettings).toHaveBeenCalledWith({ autoConnectRemote: true });
    act(() => remote.onRefreshRemote());
    expect(mocks.hooks.refreshRemote).toHaveBeenCalled();
    // Collapsing removes the rail column entirely, so the collapsed state is
    // observable as its absence rather than through its props.
    act(() => remote.onToggleLeftPanel());
    expect(view.container.querySelector("[data-child=\"activity-rail\"]")).toBeNull();
    view.unmount();
  });

  it("forwards the context panel's expand toggle and the settings provider refresh", () => {
    const view = mount(<AppShell />);
    const contextPanel = children.contextPanel as { expanded: boolean; onToggleExpanded: () => void };
    expect(contextPanel.expanded).toBe(false);
    act(() => contextPanel.onToggleExpanded());
    expect((children.contextPanel as { expanded: boolean }).expanded).toBe(true);

    act(() => (children.settingsDialog as { onProvidersChanged: () => void }).onProvidersChanged());
    expect(mocks.hooks.refreshAgentModels).toHaveBeenCalledTimes(1);
    view.unmount();
  });
});
