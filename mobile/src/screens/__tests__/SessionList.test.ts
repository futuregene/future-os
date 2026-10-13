import { createElement } from "react";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import {
  Animated,
  BackHandler,
  FlatList,
  Modal,
  PanResponder,
  StyleSheet,
  Text,
  TextInput,
  type GestureResponderEvent,
  type PanResponderGestureState,
} from "react-native";
import { Button } from "../../components/Button";
import { DialogSurface } from "../../components/DialogSurface";
import { RenameModal } from "../../features/chat/components/RenameModal";
import type { RemoteSession, RemoteWorkspace } from "../../remote/types";
import { SessionList } from "../SessionList";
import { ActionMenu } from "../../components/ActionMenu";
import type { DesktopCommands } from "../../remote/desktopCommands";
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView", useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }) }));

const mockRemote: {
  sessions: RemoteSession[];
  workspaces: RemoteWorkspace[];
  unreadSessions: Set<string>;
  desktopOnline: boolean;
  capabilities: Set<string>;
  desktops: { desktopId: string; pairId: string; name?: string }[];
  deleteSession: jest.Mock;
  deleteWorkspace: jest.Mock;
  setWorkspacePinned: jest.Mock;
  renameWorkspace: jest.Mock;
  selectSession: jest.Mock;
  switchDesktop: jest.Mock;
  refreshWorkspaces: jest.Mock;
  refreshModels: jest.Mock;
  openSessionOnDesktop: jest.Mock;
  newConversation: jest.Mock;
} = {
  sessions: [
    { sessionId: "s1", threadId: "t1", title: "First", streaming: false },
    { sessionId: "s2", threadId: "t2", title: "Second", streaming: false },
    { sessionId: "child", threadId: "tc", title: "Child", parentSessionId: "s1", streaming: false },
  ],
  workspaces: [],
  unreadSessions: new Set<string>(),
  desktopOnline: true,
  capabilities: new Set(["workspace_pinning_v1"]),
  desktops: [{ desktopId: "desk-a", pairId: "p1", name: "Studio" }],
  deleteSession: jest.fn(),
  deleteWorkspace: jest.fn(),
  setWorkspacePinned: jest.fn(),
  renameWorkspace: jest.fn(),
  selectSession: jest.fn(),
  switchDesktop: jest.fn(),
  refreshWorkspaces: jest.fn(),
  refreshModels: jest.fn(),
  openSessionOnDesktop: jest.fn(),
  newConversation: jest.fn(),
};
const mockStorage = new Map<string, string>();
jest.mock("@react-native-async-storage/async-storage", () => ({
  __esModule: true,
  default: {
    getItem: jest.fn(async (key: string) => mockStorage.get(key) ?? null),
    setItem: jest.fn(async (key: string, value: string) => {
      mockStorage.set(key, value);
    }),
    removeItem: jest.fn(async (key: string) => {
      mockStorage.delete(key);
    }),
  },
}));
jest.mock("lucide-react-native", () =>
  Object.fromEntries(
    [
      "Check",
      "CheckCheck",
      "ChevronDown",
      "ChevronRight",
      "CircleAlert",
      "Folder",
      "ListChecks",
      "Minus",
      "MoreHorizontal",
      "Pencil",
      "Pin",
      "Plus",
      "Search",
      "Trash2",
      "X",
    ].map(name => [name, name]),
  ),
);
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { title?: string; count?: number }) =>
      options?.title
        ? `${key}:${options.title}`
        : options?.count !== undefined
          ? `${key}:${options.count}`
          : key,
  }),
}));

let tree: ReactTestRenderer;
const onMenu = jest.fn();
const onTabChange = jest.fn();

/**
 * What each machine answers, as the list sees it.
 *
 * Every action a row offers is a command to the machine that owns it, so the
 * list is handed one object per machine rather than reaching into the remote
 * context: the active one (the rows that carry no source, or `desk-a` in the
 * merged tests) and `desk-b`, the machine a merged test's rows come from.
 */
function machineCommands(capabilities: string[] = []) {
  return {
    capabilities: new Set(capabilities),
    renameSession: jest.fn(async (_sessionId: string, _name: string) => {}),
    generateTitle: jest.fn(async (_sessionId: string, _language: string) => "Generated"),
    setSessionPinned: jest.fn(async (_sessionId: string, _threadId: string, _pinned: boolean) => {}),
    deleteSession: jest.fn(async (_sessionId: string, _threadId: string) => {}),
    renameWorkspace: jest.fn(async (_workspaceId: string, _name: string) => {}),
    setWorkspacePinned: jest.fn(async (_workspaceId: string, _pinned: boolean) => {}),
    deleteWorkspace: jest.fn(async (_workspaceId: string) => {}),
  } satisfies DesktopCommands;
}
const activeCommands = machineCommands(["workspace_pinning_v1"]);
const peerCommands = machineCommands(["workspace_pinning_v1"]);
/** Machines that answer at all; empty means the phone reached none of them. */
let machinesAnswer = true;
/** Machines specifically withheld — a peer this phone cannot reach. */
const unreachableMachines = new Set<string>();
const testCommandsFor = (desktopId: string | null): DesktopCommands | null =>
  !machinesAnswer || (desktopId !== null && unreachableMachines.has(desktopId))
    ? null
    : desktopId && desktopId !== "desk-a"
      ? peerCommands
      : activeCommands;
// Slide-out/settle animations resolve immediately here; the tab actually
// changing is what these tests are about.
const finishedAnimation = {
  start: (callback?: (result: { finished: boolean }) => void) => callback?.({ finished: true }),
  stop: () => {},
  reset: () => {},
} as Animated.CompositeAnimation;

/**
 * Render one page of the list and drive its swipe the way its controller
 * receives it. The view's responder props hide the gesture state (PanResponder
 * derives it from touch history), so the pan config is captured from
 * PanResponder.create — exactly how the zoom controller's test drives its pinch.
 */
function swipeHarness(tab: "workspace" | "chat") {
  const pan = jest.spyOn(PanResponder, "create");
  jest.spyOn(Animated, "timing").mockImplementation(() => finishedAnimation);
  const tabChange = jest.fn();
  let rendered!: ReactTestRenderer;
  act(() => {
    rendered = create(
      createElement(SessionList, {
        tab,
        empty: null,
        onMenu,
        onTabChange: tabChange,
        commandsFor: testCommandsFor,
      }),
    );
  });
  const handlers = pan.mock.calls.at(-1)![0];
  rendered.root
    .findAll(node => node.props.testID === "session-list-page")[0]!
    .props.onLayout({ nativeEvent: { layout: { width: 320, height: 600, x: 0, y: 0 } } });
  const gesture = (dx: number, dy: number) => ({ dx, dy, vx: 0, vy: 0 }) as PanResponderGestureState;
  return {
    rendered,
    tabChange,
    press: (label: string) => {
      act(() =>
        rendered.root
          .findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!
          .props.onPress(),
      );
    },
    /** Returns whether the page claimed the drag, as the list would. */
    drag: (dx: number, dy = 0) => {
      const event = {} as GestureResponderEvent;
      const state = gesture(dx, dy);
      let claimed = false;
      act(() => {
        claimed = handlers.onMoveShouldSetPanResponder!(event, state);
        if (!claimed) return;
        handlers.onPanResponderGrant!(event, state);
        handlers.onPanResponderMove!(event, state);
        handlers.onPanResponderRelease!(event, state);
      });
      return claimed;
    },
  };
}
const button = (label: string) =>
  tree.root.findAll(
    node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function",
  )[0]!;
const sessionBody = (title: string) => tree.root.findAll(node =>
  node.props.accessibilityRole === "button" && typeof node.props.onLongPress === "function",
).find(node => node.findAllByType(Text).some(text => text.props.children === title))!;

// Mirrors of SessionList's column geometry (see PRODUCT.md §5.2): a row is
// [toggle 16][gap 4][title]; hitSlop pads the toggle back to 44x44 without
// reaching the title column.
const ROW_INSET = 8;
const TOGGLE_WIDTH = 16;
const TOGGLE_GAP = 4;
const TOGGLE_SLOP_LEFT = 44 - TOGGLE_WIDTH - TOGGLE_GAP;
const TOGGLE_SLOP_RIGHT = TOGGLE_GAP;
const rowInset = (title: string) =>
  StyleSheet.flatten(sessionBody(title).parent!.props.style).paddingLeft;
beforeEach(async () => {
  jest.clearAllMocks();
  machinesAnswer = true;
  unreachableMachines.clear();
  activeCommands.capabilities = new Set(["workspace_pinning_v1"]);
  peerCommands.capabilities = new Set(["workspace_pinning_v1"]);
  mockRemote.newConversation.mockResolvedValue(undefined);
  mockRemote.switchDesktop.mockResolvedValue(undefined);
  mockRemote.refreshWorkspaces.mockResolvedValue(undefined);
  mockRemote.refreshModels.mockResolvedValue(undefined);
  mockRemote.openSessionOnDesktop.mockResolvedValue(undefined);
  await act(async () => {
    tree = create(createElement(SessionList, {
      tab: "chat",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    }));
    // Let the persisted-folds read land inside act() so it cannot update the
    // tree after the test body has finished.
    await Promise.resolve();
  });
});
afterEach(() => {
  act(() => tree.unmount());
  jest.restoreAllMocks();
});

test("session titles use a compact single line without reducing touch targets", () => {
  const title = tree.root.findAllByType(Text).find(node => node.props.children === "First")!;
  expect(title.props.numberOfLines).toBe(1);
  expect(title.props.ellipsizeMode).toBe("tail");
  const body = sessionBody("First");
  expect(StyleSheet.flatten(body.props.style)).toMatchObject({ minHeight: 44, paddingVertical: 8 });
});

test("explicit row action opens rename/pin/delete menu; offline management is disabled", () => {
  act(() => button("sessions.actions:First").props.onPress());
  expect(onMenu).toHaveBeenCalledWith(mockRemote.sessions[0], null);
  machinesAnswer = false;
  act(() => tree.update(createElement(SessionList, {
      tab: "chat",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  expect(button("sessions.actions:First").props.disabled).toBe(true);
  expect(button("sessions.select").props.disabled).toBe(true);
});

test("search finds a collapsed child without changing folds", () => {
  expect(button("sessions.actions:Child")).toBeUndefined();
  act(() => button("sessions.search").props.onPress());
  act(() => tree.root.findByType(TextInput).props.onChangeText("Child"));
  expect(button("sessions.actions:Child")).toBeDefined();
  act(() => button("sessions.clearSearch").props.onPress());
  expect(button("sessions.actions:Child")).toBeUndefined();
});

test("tabs, search and selection share one toolbar with full touch targets", () => {
  expect(tree.root.findAllByType(TextInput)).toHaveLength(0);
  const toolbar = tree.root.findByProps({ testID: "session-toolbar" });
  const tabs = toolbar.findAll(node => node.props.accessibilityRole === "tab" && node.props.onPress);
  expect(tabs).toHaveLength(2);
  for (const tab of tabs) {
    expect(StyleSheet.flatten(tab.props.style)).toMatchObject({ flex: 1, minHeight: 44, minWidth: 0 });
    const label = tab.findByType(Text);
    expect(label.props.numberOfLines).toBe(1);
    expect(label.props.adjustsFontSizeToFit).toBe(true);
  }
  const tabBar = tabs[0]!.parent!;
  expect(StyleSheet.flatten(tabBar.props.style)).toMatchObject({
    width: "65%",
    maxWidth: 240,
    flexShrink: 1,
  });
  const toolActions = button("sessions.search").parent!;
  expect(StyleSheet.flatten(toolActions.props.style)).toMatchObject({ marginLeft: "auto" });
  act(() => tabs[0]!.props.onPress());
  expect(onTabChange).toHaveBeenCalledWith("workspace");
  expect(toolbar.findAll(node => node.props.accessibilityLabel === "sessions.search").length).toBeGreaterThan(0);
  expect(toolbar.findAll(node => node.props.accessibilityLabel === "sessions.select").length).toBeGreaterThan(0);
});

test("a horizontal swipe pages the list between workspaces and conversations", () => {
  const fromConversations = swipeHarness("chat");
  // Conversations is the right-hand page: dragging right leaves it ...
  expect(fromConversations.drag(120, 4)).toBe(true);
  expect(fromConversations.tabChange).toHaveBeenCalledWith("workspace");
  act(() => fromConversations.rendered.unmount());

  const fromWorkspaces = swipeHarness("workspace");
  // ... dragging left opens it from the workspaces page ...
  expect(fromWorkspaces.drag(-120, 4)).toBe(true);
  expect(fromWorkspaces.tabChange).toHaveBeenCalledWith("chat");
  fromWorkspaces.tabChange.mockClear();
  // ... and dragging right has no page behind the workspaces page.
  expect(fromWorkspaces.drag(120, 4)).toBe(true);
  expect(fromWorkspaces.tabChange).not.toHaveBeenCalled();
  act(() => fromWorkspaces.rendered.unmount());
});

test("a vertical drag stays with the list, and search/selection keep the tabs put", () => {
  const harness = swipeHarness("workspace");
  expect(harness.drag(-120, 60)).toBe(false);
  // The tab bar is hidden while searching or selecting, so a swipe there would
  // change mode unseen rather than move between two visible pages.
  harness.press("sessions.search");
  expect(harness.drag(-120, 4)).toBe(false);
  harness.press("chat.cancel");
  harness.press("sessions.select");
  expect(harness.drag(-120, 4)).toBe(false);
  expect(harness.tabChange).not.toHaveBeenCalled();
  act(() => harness.rendered.unmount());
});

test("search replaces tabs in place and cancel clears the filter", () => {
  act(() => button("sessions.search").props.onPress());
  expect(tree.root.findAll(node => node.props.accessibilityRole === "tab")).toHaveLength(0);
  expect(button("sessions.select")).toBeUndefined();
  expect(tree.root.findByType(TextInput).props.autoFocus).toBe(true);
  act(() => tree.root.findByType(TextInput).props.onChangeText("absent"));
  expect(tree.root.findByType(FlatList).props.data).toHaveLength(0);
  act(() => button("chat.cancel").props.onPress());
  expect(tree.root.findAllByType(TextInput)).toHaveLength(0);
  expect(button("sessions.conversations").props.accessibilityState.selected).toBe(true);
  expect(tree.root.findByType(FlatList).props.data).not.toHaveLength(0);
  act(() => button("sessions.search").props.onPress());
  expect(tree.root.findByType(TextInput).props.value).toBe("");
});

test("selection replaces the toolbar and can be cancelled even after going offline", () => {
  act(() => button("sessions.select").props.onPress());
  expect(button("sessions.search")).toBeUndefined();
  expect(button("sessions.conversations")).toBeUndefined();
  expect(tree.root.findAllByType(TextInput)).toHaveLength(0);
  act(() => button("First").props.onPress());
  machinesAnswer = false;
  act(() => tree.update(createElement(SessionList, {
      tab: "chat",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  expect(button("chat.cancel").props.disabled).toBe(false);
  expect(button("sessions.deleteSelected").props.disabled).toBe(true);
  act(() => button("chat.cancel").props.onPress());
  expect(button("sessions.conversations")).toBeDefined();
});

test("Android back closes search without leaving a hidden filter", () => {
  const subscribe = jest.spyOn(BackHandler, "addEventListener");
  act(() => button("sessions.search").props.onPress());
  act(() => tree.root.findByType(TextInput).props.onChangeText("absent"));
  const back = subscribe.mock.calls.at(-1)![1];
  act(() => { expect(back({} as never)).toBe(true); });
  expect(tree.root.findAllByType(TextInput)).toHaveLength(0);
  expect(tree.root.findByType(FlatList).props.data).not.toHaveLength(0);
});

test("cancelling deletion does not send any requests", () => {
  act(() => button("sessions.select").props.onPress());
  act(() => button("sessions.selectVisible").props.onPress());
  act(() => button("sessions.deleteSelected").props.onPress());
  const modal = activeDialog();
  act(() => modal.findAllByType(Button).find(node => node.props.variant === "secondary")!.props.onPress());
  act(() => modal.props.onDismiss());
  expect(activeCommands.deleteSession).not.toHaveBeenCalled();
});

test("a successful batch exits selection and duplicate confirmation cannot delete twice", async () => {
  act(() => button("sessions.select").props.onPress());
  act(() => button("First").props.onPress());
  act(() => button("sessions.deleteSelected").props.onPress());
  const modal = activeDialog();
  const confirm = modal.findAllByType(Button).find(node => node.props.variant === "danger")!;
  act(() => { confirm.props.onPress(); confirm.props.onPress(); });
  expect(activeCommands.deleteSession).not.toHaveBeenCalled();
  await act(async () => { modal.props.onDismiss(); modal.props.onDismiss(); });
  expect(activeCommands.deleteSession).toHaveBeenCalledTimes(1);
  expect(button("sessions.select")).toBeDefined();
});

test("batch deletion confirms exact visible selection, preserves hidden children, and retains failures for retry", async () => {
  activeCommands.deleteSession.mockImplementation(async (id: string, _threadId: string) => {
    if (id === "s2") throw new Error("offline");
  });
  act(() => button("sessions.select").props.onPress());
  // A collapsed child must never be implicitly selected by Select visible.
  act(() => button("sessions.selectVisible").props.onPress());
  act(() => button("sessions.deleteSelected").props.onPress());
  expect(activeCommands.deleteSession).not.toHaveBeenCalled();
  expect(dialogText()).toEqual(expect.arrayContaining(["sessions.deleteSelected", "sessions.deleteSelectedConfirm:2"]));
  await act(async () => { confirmAlert(); });
  expect(activeCommands.deleteSession.mock.calls).toEqual([
    ["s1", "t1"],
    ["s2", "t2"],
  ]);
  expect(button("First").props.accessibilityState.checked).toBe(false);
  expect(button("Second").props.accessibilityState.checked).toBe(true);
  expect(dialogText()).toEqual(expect.arrayContaining(["common.error", "sessions.deletePartialFailure:1"]));
});

test("select visible skips pinned chats, which carry no checkbox", () => {
  mockRemote.sessions = mockRemote.sessions.map(session => ({ ...session, pinned: session.sessionId === "s2" }));
  act(() => tree.update(createElement(SessionList, {
      tab: "chat",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  act(() => button("sessions.select").props.onPress());
  act(() => button("sessions.selectVisible").props.onPress());
  // Only the unpinned chat joined the batch; the pinned shortcut above it did not.
  expect(button("First").props.accessibilityState.checked).toBe(true);
  expect(button("Second")).toBeUndefined();
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:1").length).toBeGreaterThan(0);
  // Pressing the pinned shortcut's row cannot put it in the batch either.
  act(() => sessionBody("Second").props.onPress());
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:1").length).toBeGreaterThan(0);
});

// ── workspace rows ──────────────────────────────────────────────────────────

/** Re-render the list on the workspace tab with two workspace sessions and one
 * plain chat, one of the workspace sessions nested under the other. */
function renderWorkspaceTab(): void {
  mockRemote.workspaces = [{ id: "w1", name: "Project", path: "/tmp/project" }];
  mockRemote.sessions = [
    { sessionId: "w1a", threadId: "tw1", title: "Plan", mode: "workspace", workspaceId: "w1", streaming: false },
    {
      sessionId: "w1b",
      threadId: "tw2",
      title: "Follow-up",
      mode: "workspace",
      workspaceId: "w1",
      parentSessionId: "w1a",
      streaming: false,
    },
    { sessionId: "c1", threadId: "tc1", title: "Chat", streaming: false },
  ];
  act(() => {
    tree.update(createElement(SessionList, {
      tab: "workspace",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    }));
  });
}

/** Re-render the list on the chat tab with the original three sessions.
 * `renderWorkspaceTab` replaces `mockRemote.sessions`, and the workspace section
 * does not restore it, so a test appended after it must reset the catalogue it
 * expects rather than inherit one. */
function renderChatTab(): void {
  mockRemote.sessions = [
    { sessionId: "s1", threadId: "t1", title: "First", streaming: false },
    { sessionId: "s2", threadId: "t2", title: "Second", streaming: false },
    { sessionId: "child", threadId: "tc", title: "Child", parentSessionId: "s1", streaming: false },
  ];
  mockRemote.workspaces = [];
  act(() => tree.update(createElement(SessionList, {
      tab: "chat",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
}

/** Open a workspace heading's sheet and take one of its actions. */
function pressWorkspaceAction(heading: string, action: string): void {
  act(() => button(`sessions.workspaceActions:${heading}`).props.onPress());
  const menu = tree.root.findByType(ActionMenu);
  expect(menu.props.actions.map((item: { label: string }) => item.label)).toContain(action);
  act(() => button(action).props.onPress());
  act(() => menu.findByType(Modal).props.onDismiss());
}

/** Select a real app sheet row and finish its iOS dismissal before navigation. */
function pressWorkspaceMenu(action: string): void {
  pressWorkspaceAction("Project", action);
}

function activeDialog() {
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  expect(modal.findAllByType(DialogSurface)).toHaveLength(1);
  return modal;
}
function dialogText() {
  return activeDialog().findAllByType(Text).map(node => node.props.children);
}
/** How many *app dialogs* are on screen right now. Unlike `activeDialog` this
 * tolerates zero, which is what a guard that must not re-open a confirmation has
 * to be asserted against; the workspace sheet's own Modal carries no
 * `DialogSurface`, so it can never be counted here. */
function visibleDialogs(): number {
  return tree.root
    .findAllByType(Modal)
    .filter(node => node.props.visible && node.findAllByType(DialogSurface).length > 0).length;
}
function confirmAlert(): void {
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible);
  if (!modal) return;
  act(() => modal.findAllByType(Button).find(node => node.props.variant === "danger")!.props.onPress());
  act(() => modal.props.onDismiss());
}

test.each(["workspace", "chat"] as const)("%s parents fold with +/−, not the workspace header chevron", (tab) => {
  // A 44px touch target keeps the row toggle and the workspace header's fold
  // control in the same column, so the glyph is the only thing separating them.
  const originalSessions = mockRemote.sessions;
  const originalWorkspaces = mockRemote.workspaces;
  const iconsIn = (node: ReactTestRenderer["root"], name: string) =>
    node.findAll(candidate => candidate.type === name);
  try {
    mockRemote.workspaces = [{ id: "glyph-workspace", name: "Glyph project", path: "/tmp/glyph" }];
    const common = { mode: tab, workspaceId: "glyph-workspace", streaming: false };
    mockRemote.sessions = [
      { ...common, sessionId: "glyph-parent", threadId: "glyph-thread", title: "Glyph parent" },
      { ...common, sessionId: "glyph-child", threadId: "glyph-thread-child", title: "Glyph child", parentSessionId: "glyph-parent" },
    ];
    act(() => tree.update(createElement(SessionList, {
      tab,
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));

    const parentRow = sessionBody("Glyph parent").parent!;
    expect(iconsIn(parentRow, "Plus")).toHaveLength(1);
    expect(iconsIn(parentRow, "ChevronRight")).toHaveLength(0);
    // The workspace header keeps its chevron; only rows fold with +/−.
    const headerChevrons = iconsIn(tree.root, "ChevronDown")
      .filter(node => !iconsIn(parentRow, "ChevronDown").includes(node));
    if (tab === "workspace") expect(headerChevrons).toHaveLength(1);

    act(() => button("sessions.expandChildren").props.onPress());
    expect(iconsIn(parentRow, "Minus")).toHaveLength(1);
    expect(iconsIn(parentRow, "ChevronDown")).toHaveLength(0);
    act(() => button("sessions.collapseChildren").props.onPress());
  }
  finally {
    mockRemote.sessions = originalSessions;
    mockRemote.workspaces = originalWorkspaces;
    act(() => tree.update(createElement(SessionList, {
      tab,
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  }
});

test("a child in one workspace does not enlarge other workspace session gutters", () => {
  renderWorkspaceTab();
  mockRemote.workspaces = [...mockRemote.workspaces, { id: "w2", name: "Other", path: "/tmp/other" }];
  mockRemote.sessions = [...mockRemote.sessions, { sessionId: "w2a", threadId: "t2a", title: "Independent", mode: "workspace", workspaceId: "w2", streaming: false }];
  act(() => tree.update(createElement(SessionList, {
      tab: "workspace",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  expect(rowInset("Independent")).toBe(ROW_INSET);
  act(() => button("sessions.expandChildren").props.onPress());
  expect(rowInset("Independent")).toBe(ROW_INSET);
  act(() => button("sessions.collapseChildren").props.onPress());
});

test.each(["workspace", "chat"] as const)("a parent's title column is its rows' start for children (%s tab)", tab => {
  // The desktop rule, mirrored: a row is [toggle 16][gap 4][title], so a child
  // row starts exactly on its parent's title column and the two titles line up.
  const originalSessions = mockRemote.sessions;
  const originalWorkspaces = mockRemote.workspaces;
  try {
    mockRemote.workspaces = [{ id: "align-workspace", name: "Align project", path: "/tmp/align" }];
    const common = { mode: tab, workspaceId: "align-workspace", streaming: false };
    mockRemote.sessions = [
      { ...common, sessionId: "align-parent", threadId: "align-thread", title: "Align parent" },
      { ...common, sessionId: "align-child", threadId: "align-thread-child", title: "Align child", parentSessionId: "align-parent" },
    ];
    act(() => tree.update(createElement(SessionList, {
      tab,
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
    act(() => button("sessions.expandChildren").props.onPress());

    const parentInset = rowInset("Align parent");
    const childInset = rowInset("Align child");
    expect(parentInset).toBe(ROW_INSET);
    expect(childInset - parentInset).toBe(TOGGLE_WIDTH + TOGGLE_GAP);
    // Leaf rows carry no toggle column of their own.
    expect(StyleSheet.flatten(sessionBody("Align parent").parent!.props.style).paddingLeft).toBe(parentInset);

    // The toggle stays 16px wide for layout, and hitSlop restores 44x44 without
    // reaching into the title column.
    const toggle = button("sessions.collapseChildren");
    expect(StyleSheet.flatten(toggle.props.style)).toMatchObject({ width: TOGGLE_WIDTH, marginRight: TOGGLE_GAP });
    expect(toggle.props.hitSlop).toEqual({ left: TOGGLE_SLOP_LEFT, right: TOGGLE_SLOP_RIGHT });
    expect(TOGGLE_WIDTH + TOGGLE_SLOP_LEFT + TOGGLE_SLOP_RIGHT).toBe(44);
    expect(TOGGLE_WIDTH + TOGGLE_GAP + TOGGLE_SLOP_RIGHT).toBe(TOGGLE_WIDTH + TOGGLE_GAP + 4);
    act(() => button("sessions.collapseChildren").props.onPress());
  }
  finally {
    mockRemote.sessions = originalSessions;
    mockRemote.workspaces = originalWorkspaces;
    act(() => tree.update(createElement(SessionList, {
      tab,
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  }
});

test("the workspace header's name sits on its rows' title column", () => {
  renderWorkspaceTab();
  const headerBody = tree.root.findAllByType(Text)
    .find(node => node.props.children === "Project")!
    .parent!;
  // 16px folder icon + 4px gap, matching the row toggle column.
  expect(StyleSheet.flatten(headerBody.props.style)).toMatchObject({ gap: TOGGLE_GAP });
});

test.each(["workspace", "chat"] as const)("independent %s sessions keep their gutter when a neighboring tree changes", tab => {
  const originalSessions = mockRemote.sessions;
  const originalWorkspaces = mockRemote.workspaces;
  const update = () => act(() => tree.update(createElement(SessionList, {
      tab,
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  try {
    mockRemote.workspaces = [{ id: "gutter-workspace", name: "Gutter project", path: "/tmp/gutter" }];
    const common = { mode: tab, workspaceId: "gutter-workspace", streaming: false };
    const roots = [
      { ...common, sessionId: "gutter-independent", threadId: "gutter-thread-independent", title: "Independent root" },
      { ...common, sessionId: "gutter-parent", threadId: "gutter-thread-parent", title: "Neighboring parent" },
    ];
    mockRemote.sessions = roots;
    update();
    expect(rowInset("Independent root")).toBe(ROW_INSET);
    expect(rowInset("Neighboring parent")).toBe(ROW_INSET);

    mockRemote.sessions = [...roots, {
      ...common, sessionId: "gutter-child", threadId: "gutter-thread-child", title: "Nested child", parentSessionId: "gutter-parent",
    }];
    update();
    expect(rowInset("Independent root")).toBe(ROW_INSET);
    expect(StyleSheet.flatten(button("sessions.expandChildren").props.style)).toMatchObject({ width: TOGGLE_WIDTH, marginRight: TOGGLE_GAP });

    act(() => button("sessions.expandChildren").props.onPress());
    expect(rowInset("Independent root")).toBe(ROW_INSET);
    expect(rowInset("Nested child")).toBe(ROW_INSET + TOGGLE_WIDTH + TOGGLE_GAP);
    act(() => button("sessions.collapseChildren").props.onPress());
    expect(rowInset("Independent root")).toBe(ROW_INSET);

    act(() => button("sessions.search").props.onPress());
    act(() => tree.root.findByType(TextInput).props.onChangeText("Independent"));
    expect(rowInset("Independent root")).toBe(ROW_INSET);
    act(() => button("chat.cancel").props.onPress());
    expect(rowInset("Independent root")).toBe(ROW_INSET);

    // Selection mode prepends the 44px checkbox and shifts every row by it —
    // the toggle keeps its own width, so parent/child stay in step.
    act(() => button("sessions.select").props.onPress());
    expect(rowInset("Independent root")).toBe(ROW_INSET);
    act(() => button("sessions.expandChildren").props.onPress());
    expect(rowInset("Independent root")).toBe(ROW_INSET);
    expect(rowInset("Nested child")).toBe(ROW_INSET + TOGGLE_WIDTH + TOGGLE_GAP);
    act(() => button("sessions.collapseChildren").props.onPress());
    act(() => button("chat.cancel").props.onPress());
    expect(rowInset("Independent root")).toBe(ROW_INSET);
  } finally {
    mockRemote.sessions = originalSessions;
    mockRemote.workspaces = originalWorkspaces;
  }
});

test("workspace menu offers the workspace actions and disables them offline", () => {
  renderWorkspaceTab();
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  expect(tree.root.findByType(ActionMenu).props.actions.map((action: { label: string }) => action.label)).toEqual([
    "sessions.new",
    "sessions.pin",
    "sessions.renameWorkspace",
    "sessions.selectWorkspaceSessions",
    "sessions.deleteWorkspace",
  ]);
  act(() => button("chat.cancel").props.onPress());
  machinesAnswer = false;
  renderWorkspaceTab();
  expect(button("sessions.workspaceActions:Project").props.disabled).toBe(true);
});

test("the workspace menu renames the group through a filled dialog and reports a refusal", async () => {
  renderWorkspaceTab();
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  const sheet = tree.root.findByType(ActionMenu).findByType(Modal);
  act(() => button("sessions.renameWorkspace").props.onPress());
  act(() => sheet.props.onDismiss());
  const dialog = tree.root.findByType(RenameModal);
  // Named after what it renames, and prefilled with the name in force.
  expect(dialog.props.titleKey).toBe("sessions.renameWorkspace");
  expect(dialog.props.renameOpen).toBe(true);
  expect(dialog.props.renameValue).toBe("Project");

  act(() => dialog.props.setRenameValue("Renamed"));
  // The box closes, then submits on dismissal — the same hedge the session
  // rename uses, so UIKit is never presenting two surfaces at once.
  act(() => button("chat.save").props.onPress());
  expect(activeCommands.renameWorkspace).not.toHaveBeenCalled();
  await act(async () => {
    tree.root.findByType(RenameModal).findByType(Modal).props.onDismiss();
    await Promise.resolve();
  });
  expect(activeCommands.renameWorkspace).toHaveBeenCalledWith("w1", "Renamed");
  expect(tree.root.findByType(RenameModal).props.renameOpen).toBe(false);

  // A refused rename says so, instead of leaving the draft looking saved.
  activeCommands.renameWorkspace.mockRejectedValueOnce(new Error("offline"));
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  const reopened = tree.root.findByType(ActionMenu).findByType(Modal);
  act(() => button("sessions.renameWorkspace").props.onPress());
  act(() => reopened.props.onDismiss());
  await act(async () => { await tree.root.findByType(RenameModal).props.submitRename(); });
  expect(dialogText()).toEqual(
    expect.arrayContaining(["common.error", "sessions.renameWorkspaceFailed"]),
  );
});

test("old desktops do not offer unsupported workspace pin commands", () => {
  activeCommands.capabilities.clear();
  renderWorkspaceTab();
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  const actions = tree.root.findByType(ActionMenu).props.actions.map((action: { label: string }) => action.label);
  expect(actions).not.toContain("sessions.pin");
  expect(actions).not.toContain("sessions.unpin");
  // Renaming is not behind a feature flag: the host routes `rename_workspace`
  // unconditionally, so every desktop that answers this list supports it.
  expect(actions).toContain("sessions.renameWorkspace");
  expect(actions).toContain("sessions.new");
  expect(activeCommands.setWorkspacePinned).not.toHaveBeenCalled();
});

test("the workspace menu pins a group and offers unpin once it is pinned", async () => {
  renderWorkspaceTab();
  pressWorkspaceMenu("sessions.pin");
  expect(activeCommands.setWorkspacePinned).toHaveBeenCalledWith("w1", true);

  mockRemote.workspaces = [{ ...mockRemote.workspaces[0]!, pinned: true }];
  act(() => tree.update(createElement(SessionList, {
      tab: "workspace",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  pressWorkspaceMenu("sessions.unpin");
  expect(activeCommands.setWorkspacePinned).toHaveBeenLastCalledWith("w1", false);

  // A refused pin reports the failure instead of leaving the row moved.
  activeCommands.setWorkspacePinned.mockRejectedValueOnce(new Error("offline"));
  pressWorkspaceMenu("sessions.unpin");
  await act(async () => {});
  expect(dialogText()).toEqual(expect.arrayContaining(["common.error"]));
});

test("new conversation uses the selected workspace and waits for sheet dismissal", async () => {
  renderWorkspaceTab();
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  const modal = tree.root.findByType(ActionMenu).findByType(Modal);
  act(() => button("sessions.new").props.onPress());
  expect(mockRemote.newConversation).not.toHaveBeenCalled();
  await act(async () => { modal.props.onDismiss(); });
  expect(mockRemote.newConversation).toHaveBeenCalledWith("workspace", "w1");
  // This machine's own group has nothing to switch to: the draft opens on the
  // connection the reader is already using, and nothing is re-read.
  expect(mockRemote.switchDesktop).not.toHaveBeenCalled();
  expect(mockRemote.refreshModels).not.toHaveBeenCalled();
  act(() => modal.props.onDismiss());
  expect(mockRemote.newConversation).toHaveBeenCalledTimes(1);
});

test("select all in a workspace selects nested sessions but not other chats", () => {
  renderWorkspaceTab();
  pressWorkspaceMenu("sessions.selectWorkspaceSessions");
  // Only the workspace's two sessions, nested one included — the count text
  // proves nothing else (e.g. the plain chat) was pulled in.
  expect(
    tree.root.findAll(node => node.props.children === "sessions.selectedCount:2").length,
  ).toBeGreaterThan(0);
  expect(button("Plan").props.accessibilityState.checked).toBe(true);
  // The nested follow-up is folded away yet still selected: a workspace-wide
  // select must not be limited to what happens to be expanded.
  expect(button("Follow-up")).toBeUndefined();
  expect(button("sessions.deleteSelected").props.disabled).toBe(false);
});

test("deleting a workspace confirms with its session count and calls the desktop once", async () => {
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    activeCommands.deleteWorkspace.mockResolvedValue(undefined);
    // A batch assembled over the workspace before it was removed: the host takes
    // its conversations with it, so those ticks must go too — a selection that
    // survives would offer to delete conversations the host no longer has.
    act(() => button("sessions.select").props.onPress());
    act(() => button("sessions.selectVisible").props.onPress());
    expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:1").length).toBeGreaterThan(0);
    pressWorkspaceMenu("sessions.deleteWorkspace");
    expect(dialogText()).toEqual(expect.arrayContaining(["sessions.deleteWorkspace", "sessions.deleteWorkspaceConfirm:Project"]));
    await act(async () => {
      confirmAlert();
      confirmAlert(); // A second tap while the request is in flight must not re-fire.
    });
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledWith("w1");
    // Everything the workspace held — the folded child included — left the batch
    // with it.
    expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:0").length).toBeGreaterThan(0);
  } finally {
    jest.useRealTimers();
  }
});

test("promoted workspace pins stay visible and openable but are left out of workspace selection", () => {
  renderWorkspaceTab();
  mockRemote.sessions = mockRemote.sessions.map(session => ({ ...session, pinned: session.sessionId === "w1b" }));
  act(() => tree.update(createElement(SessionList, {
      tab: "workspace",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  const rows = tree.root.findByType(FlatList).props.data;
  expect(rows.map((row: { key: string }) => row.key)).toEqual(["w1b", "workspace:w1", "w1a"]);
  expect(rows[1].count).toBe(2);
  act(() => button("Project").props.onPress());
  expect(tree.root.findByType(FlatList).props.data.map((row: { key: string }) => row.key)).toEqual(["w1b", "workspace:w1"]);
  act(() => sessionBody("Follow-up").props.onPress());
  expect(mockRemote.selectSession).toHaveBeenCalledWith("w1b");
  pressWorkspaceMenu("sessions.selectWorkspaceSessions");
  // The shortcut row above the group is skipped; only the group's own session
  // joins the batch, even though the workspace header still counts the pin.
  expect(button("Follow-up")).toBeUndefined();
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:1").length).toBeGreaterThan(0);
  act(() => button("Project").props.onPress());
  expect(button("Plan").props.accessibilityState.checked).toBe(true);
  // Pressing the pinned shortcut's row cannot put it in the batch either.
  act(() => sessionBody("Follow-up").props.onPress());
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:1").length).toBeGreaterThan(0);
});

test("a failed workspace delete surfaces the workspace error instead of the generic one", async () => {
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    activeCommands.deleteWorkspace.mockRejectedValue(new Error("offline"));
    pressWorkspaceMenu("sessions.deleteWorkspace");
    await act(async () => {
      confirmAlert();
    });
    expect(dialogText()).toEqual(expect.arrayContaining(["common.error", "sessions.deleteWorkspaceFailed"]));
  } finally {
    jest.useRealTimers();
  }
});

test("a workspace menu cannot outlive the screen that opened it", () => {
  renderWorkspaceTab();
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(true);
  // Leaving the list (entering a chat, or a pairing change) unmounts the page
  // but the menu it opened lives in a native Modal: it has to be torn down with
  // the screen rather than left floating over whatever comes next.
  act(() => tree.update(createElement(SessionList, {
      tab: "workspace",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
      active: false,
    })));
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(false);
  act(() => tree.update(createElement(SessionList, {
      tab: "workspace",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
      active: true,
    })));
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(false);
});

test("system back leaves batch selection before it leaves the list", () => {
  renderChatTab();
  // The listener is registered when selection opens, so the spy has to exist
  // first or there is nothing to capture.
  const subscribe = jest.spyOn(BackHandler, "addEventListener");
  act(() => button("sessions.select").props.onPress());
  act(() => button("First").props.onPress());
  expect(button("First").props.accessibilityState.checked).toBe(true);
  const back = subscribe.mock.calls.at(-1)![1];
  act(() => { expect(back({} as never)).toBe(true); });
  // The toolbar is back and the batch is empty, so a stray back cannot delete.
  expect(button("sessions.select")).toBeDefined();
  expect(button("First")).toBeUndefined();
});

test("tapping a selected row removes it from the batch", () => {
  renderChatTab();
  act(() => button("sessions.select").props.onPress());
  act(() => button("First").props.onPress());
  expect(button("First").props.accessibilityState.checked).toBe(true);
  expect(button("sessions.deleteSelected").props.disabled).toBe(false);
  // A second tap on the same row is how a user takes one back out: leaving it
  // checked would delete a conversation the user just unticked.
  act(() => button("First").props.onPress());
  expect(button("First").props.accessibilityState.checked).toBe(false);
  expect(button("sessions.deleteSelected").props.disabled).toBe(true);
});

test("deselect visible empties the batch that select visible made", () => {
  renderChatTab();
  act(() => button("sessions.select").props.onPress());
  act(() => button("sessions.selectVisible").props.onPress());
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:2").length).toBeGreaterThan(0);
  act(() => button("sessions.deselectVisible").props.onPress());
  expect(button("sessions.deleteSelected").props.disabled).toBe(true);
  expect(button("sessions.selectVisible")).toBeDefined();
});

test("a batch tap with nothing selected, or while offline, sends nothing", () => {
  renderChatTab();
  const visibleDialog = () => tree.root.findAllByType(Modal).some(node => node.props.visible);
  act(() => button("sessions.select").props.onPress());
  // The button is disabled in both states; a tap that began before the disable
  // still reaches the handler, which is what the guard is for. Asserting only
  // that no request was sent would miss a confirmation that should never have
  // been offered — the mutant that drops this guard survives that assertion.
  act(() => button("sessions.deleteSelected").props.onPress());
  expect(activeCommands.deleteSession).not.toHaveBeenCalled();
  expect(visibleDialog()).toBe(false);
  act(() => button("First").props.onPress());
  machinesAnswer = false;
  act(() => tree.update(createElement(SessionList, {
      tab: "chat",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  act(() => button("sessions.deleteSelected").props.onPress());
  expect(activeCommands.deleteSession).not.toHaveBeenCalled();
  expect(visibleDialog()).toBe(false);
  machinesAnswer = true;
});

test("selecting a row inside a workspace group works the same as in a chat list", () => {
  renderWorkspaceTab();
  act(() => button("sessions.select").props.onPress());
  act(() => sessionBody("Plan").props.onPress());
  // Row taps route to the same toggle as the checkbox while selecting.
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:1").length).toBeGreaterThan(0);
  expect(button("Plan").props.accessibilityState.checked).toBe(true);
});

test("a workspace select-all with nothing to select is inert", () => {
  renderWorkspaceTab();
  // Every session in the group is a promoted pin, which carries no checkbox.
  mockRemote.sessions = mockRemote.sessions.map(session => ({ ...session, pinned: true }));
  act(() => tree.update(createElement(SessionList, {
      tab: "workspace",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  pressWorkspaceMenu("sessions.selectWorkspaceSessions");
  // The shortcut must not open an empty batch: the toolbar would then offer a
  // delete with nothing selected.
  expect(button("sessions.deleteSelected")).toBeUndefined();
  expect(button("sessions.select")).toBeDefined();
});

test("a workspace menu does not open while the desktop is unreachable", () => {
  renderWorkspaceTab();
  machinesAnswer = false;
  act(() => tree.update(createElement(SessionList, {
      tab: "workspace",
      empty: null,
      onMenu,
      onTabChange,
      commandsFor: testCommandsFor,
    })));
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  // Pin/rename/delete are all desktop writes; opening the sheet on a dead link
  // would offer actions that cannot run.
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(false);
  machinesAnswer = true;
});

test("a workspace cannot be deleted twice while the first request is in flight", async () => {
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    const pending = (() => {
      let resolve!: () => void;
      return { promise: new Promise<void>(done => { resolve = done; }), resolve: () => resolve() };
    })();
    activeCommands.deleteWorkspace.mockReturnValue(pending.promise);
    pressWorkspaceMenu("sessions.deleteWorkspace");
    await act(async () => { confirmAlert(); });
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
    // A tap on the workspace menu that began before the delete disabled it must
    // not queue a second destructive request.
    act(() => button("sessions.workspaceActions:Project").props.onPress());
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => { pending.resolve(); });
  } finally {
    jest.useRealTimers();
    activeCommands.deleteWorkspace.mockResolvedValue(undefined);
  }
});

test("a double tap on the delete confirmation cannot queue the batch twice", async () => {
  renderChatTab();
  // The desktop is slow to answer, so the first delete is still unresolved when
  // the second confirmation is confirmed.
  let resolveFirst!: () => void;
  activeCommands.deleteSession.mockReturnValueOnce(
    new Promise<void>(done => { resolveFirst = () => done(); }),
  );
  act(() => button("sessions.select").props.onPress());
  act(() => button("First").props.onPress());
  // Two confirmations are queued: the toolbar button stays enabled until the
  // delete actually starts, so a second tap re-arms it.
  act(() => {
    button("sessions.deleteSelected").props.onPress();
    button("sessions.deleteSelected").props.onPress();
  });
  await act(async () => { confirmAlert(); });
  expect(activeCommands.deleteSession).toHaveBeenCalledTimes(1);
  // The second confirmation is still on screen; confirming it must not delete
  // the same conversations again — a thread the desktop already removed would
  // answer with an error the user would see for an action that had succeeded.
  await act(async () => { confirmAlert(); });
  expect(activeCommands.deleteSession).toHaveBeenCalledTimes(1);
  await act(async () => { resolveFirst(); await Promise.resolve(); });
  expect(activeCommands.deleteSession).toHaveBeenCalledTimes(1);
  activeCommands.deleteSession.mockResolvedValue(undefined);
});

test("a double tap on the workspace confirmation deletes it once", async () => {
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    let resolveDelete!: () => void;
    activeCommands.deleteWorkspace.mockReturnValueOnce(
      new Promise<void>(done => { resolveDelete = () => done(); }),
    );
    // Two confirmations are queued for the same workspace: the menu can be
    // reopened while the first confirmation is still on screen (the delete has
    // not started), so both requests reach the user.
    pressWorkspaceMenu("sessions.deleteWorkspace");
    pressWorkspaceMenu("sessions.deleteWorkspace");
    await act(async () => { confirmAlert(); });
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
    // The first request is still in flight and the second confirmation is on
    // screen; confirming it must not queue the same destructive request again.
    await act(async () => { confirmAlert(); });
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => { resolveDelete(); await Promise.resolve(); });
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
  } finally {
    jest.useRealTimers();
    activeCommands.deleteWorkspace.mockResolvedValue(undefined);
  }
});

test("a stale workspace delete item cannot queue a second confirmation while the first is in flight", async () => {
  // The workspace sheet (ActionMenu) does not run an item when it is pressed: it
  // *queues* it and runs it from the Modal's `onDismiss`, and that flush clears
  // its own latch. So a press+dismiss pair captured while the sheet was usable
  // can be driven a second time after the sheet has closed, and the second round
  // lands on `confirmDeleteWorkspace` with `deletingRef.current` still true.
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    let resolveDelete!: () => void;
    activeCommands.deleteWorkspace.mockReturnValueOnce(
      new Promise<void>(done => { resolveDelete = () => done(); }),
    );

    act(() => button("sessions.workspaceActions:Project").props.onPress());
    const sheet = tree.root.findByType(ActionMenu).findByType(Modal);
    const press = button("sessions.deleteWorkspace").props.onPress as () => void;
    const flush = sheet.props.onDismiss as () => void;

    act(() => { press(); flush(); });
    expect(visibleDialogs()).toBe(1);
    await act(async () => { confirmAlert(); });
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledWith("w1");
    // The request really is in flight, so the guard's read at round 2 is a live
    // `true`; a delete that had already settled would make round 2 vacuous.
    expect(button("sessions.workspaceActions:Project").props.disabled).toBe(true);

    act(() => { press(); flush(); });
    // Nothing was re-queued. This count — not the request count, which would
    // only move if the stale confirmation were then tapped — is what fails when
    // the guard is dropped: round 2 would put a second confirmation on screen.
    expect(visibleDialogs()).toBe(0);
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);

    // Control, with the same captured pair: settle the delete, so the ref reads
    // false again, and drive it a third time. A confirmation *is* queued now —
    // which is what makes round 2's zero a statement about the guard rather than
    // about a closure that went dead when the sheet closed.
    await act(async () => { resolveDelete(); await Promise.resolve(); });
    expect(button("sessions.workspaceActions:Project").props.disabled).toBe(false);
    act(() => { press(); flush(); });
    expect(visibleDialogs()).toBe(1);
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
  } finally {
    jest.useRealTimers();
    activeCommands.deleteWorkspace.mockResolvedValue(undefined);
  }
});

test("a workspace cannot be deleted twice while the first request is in flight", async () => {
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    const pending = (() => {
      let resolve!: () => void;
      return { promise: new Promise<void>(done => { resolve = done; }), resolve: () => resolve() };
    })();
    activeCommands.deleteWorkspace.mockReturnValue(pending.promise);
    // Two requests reach the queue (the menu is re-openable, and a slow desktop
    // leaves the first one pending), so a second confirmation is on screen while
    // the first delete is still in flight.
    pressWorkspaceMenu("sessions.deleteWorkspace");
    pressWorkspaceMenu("sessions.deleteWorkspace");
    await act(async () => { confirmAlert(); });
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => { confirmAlert(); });
    // The second confirmation must not queue another destructive request.
    expect(activeCommands.deleteWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => { pending.resolve(); });
  } finally {
    jest.useRealTimers();
    activeCommands.deleteWorkspace.mockResolvedValue(undefined);
  }
});

test("a queued second batch confirmation cannot delete while the first is in flight", async () => {
  jest.useFakeTimers();
  try {
    renderChatTab();
    const pending = (() => {
      let resolve!: () => void;
      return { promise: new Promise<void>(done => { resolve = done; }), resolve: () => resolve() };
    })();
    activeCommands.deleteSession.mockReturnValue(pending.promise);
    act(() => button("sessions.select").props.onPress());
    act(() => button("First").props.onPress());
    act(() => button("sessions.deleteSelected").props.onPress());
    act(() => button("sessions.deleteSelected").props.onPress());
    await act(async () => { confirmAlert(); });
    expect(activeCommands.deleteSession).toHaveBeenCalledTimes(1);
    await act(async () => { confirmAlert(); });
    // A batch delete is a sequence of desktop writes; a second confirmation
    // while the first runs would interleave them and delete twice.
    expect(activeCommands.deleteSession).toHaveBeenCalledTimes(1);
    await act(async () => { pending.resolve(); });
  } finally {
    jest.useRealTimers();
    activeCommands.deleteSession.mockResolvedValue(undefined);
  }
});

test("a second tap on the same batch-delete confirm cannot start a second round", async () => {
  // The neighbouring test re-finds the dialog for its second tap, so it would
  // pass even if the guard were missing — after the first flush the dialog is
  // invisible and the second lookup finds nothing. This one captures the
  // dismiss/flush pair once and drives it twice: `useAppDialog` queues the
  // action on press and runs it on `onDismiss`, and `flush` resets its own
  // latch, so both rounds reach the guarded closure. The first sets the flag
  // synchronously; the second must see it and not start a second round.
  jest.useFakeTimers();
  try {
    renderChatTab();
    const pending = (() => {
      let resolve!: () => void;
      return { promise: new Promise<void>(done => { resolve = done; }), resolve: () => resolve() };
    })();
    activeCommands.deleteSession.mockReturnValue(pending.promise);
    act(() => button("sessions.select").props.onPress());
    act(() => button("First").props.onPress());
    act(() => button("sessions.deleteSelected").props.onPress());
    const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
    const press = modal.findAllByType(Button).find(node => node.props.variant === "danger")!.props.onPress;
    const flush = modal.props.onDismiss;
    act(() => { press(); flush(); });
    expect(activeCommands.deleteSession).toHaveBeenCalledTimes(1);
    // Same captured closures, second round: the request is still in flight.
    await act(async () => { press(); flush(); });
    expect(activeCommands.deleteSession).toHaveBeenCalledTimes(1);
    await act(async () => { pending.resolve(); });
  } finally {
    jest.useRealTimers();
    activeCommands.deleteSession.mockResolvedValue(undefined);
  }
});

test("a refused workspace conversation reports the failure instead of a stale screen", async () => {
  renderWorkspaceTab();
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  const modal = tree.root.findByType(ActionMenu).findByType(Modal);
  mockRemote.newConversation.mockRejectedValueOnce(new Error("offline"));
  act(() => button("sessions.new").props.onPress());
  await act(async () => { modal.props.onDismiss(); });
  expect(mockRemote.newConversation).toHaveBeenCalledWith("workspace", "w1");
  expect(dialogText()).toEqual(expect.arrayContaining(["common.error"]));
});

test("a long press on a row opens the menu only outside selection mode", () => {
  renderChatTab();
  act(() => sessionBody("First").props.onLongPress());
  expect(onMenu).toHaveBeenCalledWith(mockRemote.sessions[0], null);
  onMenu.mockClear();
  act(() => button("sessions.select").props.onPress());
  // Inside the batch a long press is a no-op: the row already has a checkbox.
  act(() => sessionBody("First").props.onLongPress());
  expect(onMenu).not.toHaveBeenCalled();
  expect(button("First").props.accessibilityState.checked).toBe(false);
});

test("pressing and releasing a row leaves no stuck pressed state", () => {
  renderChatTab();
  const rowStyle = () => StyleSheet.flatten(sessionBody("First").parent!.props.style);
  expect(rowStyle().backgroundColor).toBeUndefined();
  act(() => sessionBody("First").props.onPressIn());
  expect(rowStyle().backgroundColor).toBeDefined();
  // A finger lifting off another row must not clear the one still pressed.
  act(() => sessionBody("Second").props.onPressOut());
  expect(rowStyle().backgroundColor).toBeDefined();
  act(() => sessionBody("First").props.onPressOut());
  expect(rowStyle().backgroundColor).toBeUndefined();
});

/**
 * A merged row's tap is routed to the machine that owns it.
 *
 * The phone holds one active connection, so selecting another desktop's row
 * through it names a conversation that desktop has never heard of — and the
 * host used to answer that by *creating* a conversation of its own, which is
 * how a follow-up written inside one conversation appeared under a brand new
 * chat. The active desktop's own row stays the plain selection it always was.
 */
test("a merged row from another desktop is opened on that desktop", () => {
  const merged = [
    {
      key: "desk-a::s1",
      desktopId: "desk-a",
      lastMessageAt: 2,
      session: mockRemote.sessions[0]!,
    },
    {
      key: "desk-b::s9",
      desktopId: "desk-b",
      lastMessageAt: 1,
      session: { sessionId: "s9", threadId: "t9", title: "Elsewhere", streaming: false },
    },
  ];
  act(() => tree.update(createElement(SessionList, {
    tab: "chat",
    empty: null,
    onMenu,
    onTabChange,
    commandsFor: testCommandsFor,
    merged,
    activeDesktopId: "desk-a",
    icons: new Map([["desk-a", "🖥️"], ["desk-b", "💻"]]),
  })));

  act(() => sessionBody("Elsewhere").props.onPress());
  expect(mockRemote.openSessionOnDesktop).toHaveBeenCalledWith("desk-b", "s9");
  expect(mockRemote.selectSession).not.toHaveBeenCalled();

  // The active desktop's own row is the plain selection it always was.
  act(() => sessionBody("First").props.onPress());
  expect(mockRemote.selectSession).toHaveBeenCalledWith("s1");
  expect(mockRemote.openSessionOnDesktop).toHaveBeenCalledTimes(1);
});

/**
 * The merged list keeps the hierarchy the single-machine list has.
 *
 * A workspace group is keyed by `(machine, workspace id)`: a workspace called
 * "Project" can exist on two machines at once, and one heading for both would
 * file one machine's conversations under the other's. Every group fact rides on
 * its rows, which is how a heading knows a peer's workspace name at all.
 */
function renderMerged(
  merged: {
    key: string;
    desktopId: string;
    lastMessageAt: number;
    session: RemoteSession;
  }[],
  activeDesktopId = "desk-a",
  tab: "workspace" | "chat" = "workspace",
): void {
  act(() => tree.update(createElement(SessionList, {
    tab,
    empty: null,
    onMenu,
    onTabChange,
    commandsFor: testCommandsFor,
    merged,
    activeDesktopId,
    icons: new Map([["desk-a", "🖥️"], ["desk-b", "💻"]]),
  })));
}

const mergedRow = (
  desktopId: string,
  sessionId: string,
  title: string,
  extra: Partial<RemoteSession> = {},
  lastMessageAt = 1,
) => ({
  key: `${desktopId}::${sessionId}`,
  desktopId,
  lastMessageAt,
  session: {
    sessionId,
    threadId: `t-${sessionId}`,
    title,
    streaming: false,
    mode: "workspace" as const,
    workspaceId: "w1",
    workspaceName: "Project",
    ...extra,
  } as RemoteSession,
});

const mergedTitles = (): string[] =>
  tree.root.findByType(FlatList).props.data
    .filter((row: { kind: string }) => row.kind === "session")
    .map((row: { session: RemoteSession }) => row.session.title);

const headingNames = (): string[] =>
  tree.root.findByType(FlatList).props.data
    .filter((row: { kind: string }) => row.kind === "workspace")
    .map((row: { workspace: RemoteWorkspace }) => row.workspace.name);

/**
 * How many times a machine's glyph is drawn inside one rendered node.
 *
 * A source icon is the one text whose accessibility label is a machine id, and
 * the heading's and the row's are the same element — which is exactly what
 * makes "named once" countable.
 */
const glyphsIn = (node: ReactTestInstance, desktopId: string): number =>
  node.findAllByType(Text).filter(child => child.props.accessibilityLabel === desktopId).length;
/** The glyphs drawn for a machine anywhere on the page. */
const sourceGlyphs = (desktopId: string): number => glyphsIn(tree.root, desktopId);
/** The glyphs drawn on a row, found by its conversation's title. */
const rowGlyphs = (title: string, desktopId: string): number =>
  glyphsIn(sessionBody(title), desktopId);
/** The glyphs drawn on a workspace heading, found by its name. */
const headingGlyphs = (name: string, desktopId: string): number =>
  glyphsIn(button(name), desktopId);

test("the merged list files each machine's conversations under their workspace headings", () => {
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);

  expect(headingNames()).toEqual(["Project", "Review"]);
  expect(mergedTitles()).toEqual(["Local plan", "Remote review"]);
});

/**
 * A machine is named on the heading that owns the group, not on every line of
 * it.
 *
 * A workspace heading carries its machine's glyph, so a conversation under that
 * heading drawing it again is the same glyph on every row of the group — and it
 * is the width those titles need, on a rail that has none to spare. The rule is
 * the desktop rail's, which passes the group's answer down to its rows.
 */
test("a workspace heading names its machine, and its conversations do not repeat it", () => {
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote plan", { workspaceId: "w9", workspaceName: "Review" }),
    mergedRow("desk-b", "b2", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);

  expect(headingGlyphs("Project", "desk-a")).toBe(1);
  expect(headingGlyphs("Review", "desk-b")).toBe(1);
  // Two conversations on that machine, in one group: the glyph stays on the
  // heading rather than appearing three times down the group.
  expect(rowGlyphs("Remote plan", "desk-b")).toBe(0);
  expect(rowGlyphs("Remote review", "desk-b")).toBe(0);
  expect(rowGlyphs("Local plan", "desk-a")).toBe(0);
  expect(sourceGlyphs("desk-b")).toBe(1);
});

/**
 * A conversation nothing above it names still carries its own machine's glyph.
 *
 * Two places have no heading over them: the conversations page, which is flat,
 * and a pinned conversation, which is a shortcut promoted above the groups.
 * Losing the glyph there would leave a merged row indistinguishable from one of
 * the active machine's.
 */
test("a conversation with no heading above it keeps its machine's glyph", () => {
  renderMerged(
    [
      mergedRow("desk-a", "c1", "Local chat", { mode: "chat", workspaceId: undefined, workspaceName: undefined }),
      mergedRow("desk-b", "c2", "Remote chat", { mode: "chat", workspaceId: undefined, workspaceName: undefined }),
    ],
    "desk-a",
    "chat",
  );
  expect(rowGlyphs("Remote chat", "desk-b")).toBe(1);

  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote plan", { workspaceId: "w9", workspaceName: "Review" }),
    mergedRow("desk-b", "b2", "Pinned elsewhere", {
      workspaceId: "w9",
      workspaceName: "Review",
      pinned: true,
    }),
  ]);
  // Above the groups: no heading carries it, so the row does.
  expect(rowGlyphs("Pinned elsewhere", "desk-b")).toBe(1);
  // Under the heading: it does not.
  expect(rowGlyphs("Remote plan", "desk-b")).toBe(0);
});

/**
 * A page holds its own kind of conversation, exactly as the single-machine list
 * does: the workspaces page the ones that live in a workspace, the conversations
 * page the ones that live in none. Filing both on both pages put a machine's
 * chats under the workspace groups — where the list has no such section — and
 * made the two pages the same list.
 */
test("each page of the merged list holds its own kind of conversation", () => {
  const rows = [
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-a", "c1", "Local chat", { mode: "chat", workspaceId: undefined, workspaceName: undefined }),
    // A host older than the field: no `mode` at all, which is a chat.
    mergedRow("desk-b", "b2", "Remote chat", { mode: undefined, workspaceId: undefined, workspaceName: undefined }),
  ];
  renderMerged(rows, "desk-a", "workspace");
  expect(headingNames()).toEqual(["Project"]);
  expect(mergedTitles()).toEqual(["Local plan"]);

  renderMerged(rows, "desk-a", "chat");
  expect(headingNames()).toEqual([]);
  expect(mergedTitles()).toEqual(["Local chat", "Remote chat"]);
});

/**
 * A heading on another machine offers that machine's actions, and they land
 * there.
 *
 * Rename, pin and delete are commands to the machine that owns the workspace —
 * the one this phone holds a connection to for that machine's rows — so the
 * sheet is willing to offer them, and says which machine they address before its
 * first destructive item. Starting a conversation is that machine's too: the
 * queue is the active connection's, so the sheet switches the phone to the
 * machine that will run it instead of leaving the item out.
 */
test("a heading on another machine offers that machine's actions", async () => {
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);

  // The sheet is the same sheet wherever the group's workspace lives: one item
  // set, not the active machine's with "new conversation" in it and a peer's
  // without. Everything else about that action is below; the pin is here to
  // prove the other items still address the peer's own connection.
  act(() => button("sessions.workspaceActions:Review").props.onPress());
  expect(tree.root.findByType(ActionMenu).props.actions.map((a: { label: string }) => a.label))
    .toEqual([
      "sessions.actionsOnDesktop",
      "sessions.new",
      "sessions.pin",
      "sessions.renameWorkspace",
      "sessions.selectWorkspaceSessions",
      "sessions.deleteWorkspace",
    ]);
  await act(async () => {
    button("sessions.pin").props.onPress();
    await tree.root.findByType(ActionMenu).findByType(Modal).props.onDismiss();
  });
  expect(peerCommands.setWorkspacePinned).toHaveBeenCalledWith("w9", true);
  expect(activeCommands.setWorkspacePinned).not.toHaveBeenCalled();

  // Starting one there reaches that machine first: the draft belongs to the
  // connection that will carry the first prompt into the workspace.
  act(() => button("sessions.workspaceActions:Review").props.onPress());
  await act(async () => {
    button("sessions.new").props.onPress();
    await tree.root.findByType(ActionMenu).findByType(Modal).props.onDismiss();
  });
  expect(mockRemote.switchDesktop).toHaveBeenCalledWith("desk-b");
  // The machine it moved onto is re-read before the draft: a freshly connected
  // machine's catalogue starts empty, and its models are what the draft defaults
  // to. Same as the creation dialog's own move onto another machine.
  expect(mockRemote.refreshWorkspaces).toHaveBeenCalled();
  expect(mockRemote.refreshModels).toHaveBeenCalled();
  expect(mockRemote.newConversation).toHaveBeenCalledWith("workspace", "w9");
  expect(mockRemote.switchDesktop.mock.invocationCallOrder[0]!)
    .toBeLessThan(mockRemote.newConversation.mock.invocationCallOrder[0]!);
  expect(mockRemote.refreshModels.mock.invocationCallOrder[0]!)
    .toBeLessThan(mockRemote.newConversation.mock.invocationCallOrder[0]!);

  // A machine the phone cannot reach offers nothing: the heading's ⋯ is shown
  // unavailable, as the active machine's is when its connection is down.
  unreachableMachines.add("desk-b");
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);
  expect(button("sessions.workspaceActions:Review").props.disabled).toBe(true);
  expect(button("sessions.workspaceActions:Project").props.disabled).toBe(false);
  unreachableMachines.clear();
});

/**
 * The item list a group's sheet holds, by its heading's name.
 *
 * Read as one string per item so a difference between two groups' sheets is
 * visible as the two lists side by side.
 */
const sheetItems = (heading: string): string[] => {
  act(() => button(`sessions.workspaceActions:${heading}`).props.onPress());
  const items = tree.root
    .findByType(ActionMenu)
    .props.actions.map((action: { label: string }) => action.label);
  act(() => tree.root.findByType(ActionMenu).findByType(Modal).props.onDismiss());
  return items;
};

/**
 * Every group offers "new conversation", whichever machine the phone is on.
 *
 * The sheet used to hold it only for the machine the connection was on, so the
 * same list gave two answers: the group the reader had just come back from had
 * the item, every other group did not — and opening a conversation on the other
 * machine moved it across, because that is what switches the connection.
 */
test("both machines' groups offer a new conversation, whichever one is connected", () => {
  const rows = [
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ];
  const expectBoth = () => {
    expect(sheetItems("Project")).toContain("sessions.new");
    expect(sheetItems("Review")).toContain("sessions.new");
  };

  renderMerged(rows, "desk-a");
  expectBoth();
  // The same two groups after the reader opened one of the other machine's
  // conversations, which is what moves the connection.
  renderMerged(rows, "desk-b");
  expectBoth();
});

test("renaming and deleting a peer workspace name it on its own machine", async () => {
  renderMerged([
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);

  pressWorkspaceAction("Review", "sessions.renameWorkspace");
  const dialog = tree.root.findByType(RenameModal);
  // Prefilled with the name in force, then named on the machine that owns it —
  // by the workspace's own id, not the list's machine-qualified fold key.
  expect(dialog.props.renameValue).toBe("Review");
  act(() => dialog.props.setRenameValue("Reviewed"));
  await act(async () => {
    await tree.root.findByType(RenameModal).props.submitRename();
  });
  expect(peerCommands.renameWorkspace).toHaveBeenCalledWith("w9", "Reviewed");
  expect(activeCommands.renameWorkspace).not.toHaveBeenCalled();

  pressWorkspaceAction("Review", "sessions.deleteWorkspace");
  await act(async () => { confirmAlert(); });
  expect(peerCommands.deleteWorkspace).toHaveBeenCalledWith("w9");
  expect(activeCommands.deleteWorkspace).not.toHaveBeenCalled();
});

test("every conversation row offers its own machine's actions", () => {
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);

  // The row says which machine its actions address: the phone runs one
  // connection, so a rename here is a command to the desktop that owns the
  // conversation, and it is offered whenever that machine can take one.
  act(() => button("sessions.actions:Remote review").props.onPress());
  expect(onMenu).toHaveBeenCalledWith(
    expect.objectContaining({ sessionId: "b1" }),
    "desk-b",
  );
  expect(button("sessions.actions:Remote review").props.disabled).toBe(false);

  // A machine the phone has not reached cannot answer: the row shows the action
  // as unavailable rather than sending a command that would fail.
  unreachableMachines.add("desk-b");
  renderMerged([mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" })]);
  expect(button("sessions.actions:Remote review").props.disabled).toBe(true);
  unreachableMachines.clear();
});

/**
 * One batch can hold several machines' conversations, and each delete goes to
 * the machine that owns its row.
 *
 * A selection is keyed by row, not by session id: the same session id exists on
 * two machines, and a batch keyed by id would tick one machine's row when the
 * user ticked the other's.
 */
test("a batch deletes each machine's conversations through its own connection", async () => {
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);
  act(() => button("sessions.select").props.onPress());
  act(() => button("sessions.selectVisible").props.onPress());
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:2").length).toBeGreaterThan(0);
  expect(button("Remote review").props.accessibilityRole).toBe("checkbox");

  act(() => button("sessions.deleteSelected").props.onPress());
  const sheet = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  act(() => sheet.findAllByType(Button).find(node => node.props.variant === "danger")!.props.onPress());
  await act(async () => { sheet.props.onDismiss(); });

  expect(activeCommands.deleteSession).toHaveBeenCalledWith("a1", "t-a1");
  expect(peerCommands.deleteSession).toHaveBeenCalledWith("b1", "t-b1");
  expect(peerCommands.deleteSession).toHaveBeenCalledTimes(1);
});

test("a machine that cannot answer keeps its rows selected for retry", async () => {
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);
  act(() => button("sessions.select").props.onPress());
  act(() => button("sessions.selectVisible").props.onPress());
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:2").length).toBeGreaterThan(0);

  // The other machine drops after the batch was assembled. The reachable one's
  // rows are still deleted — the batch is offered while one of its machines
  // answers — and the unreachable one's keep their ticks, so the retry is one
  // tap rather than a re-selection.
  unreachableMachines.add("desk-b");
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);
  act(() => button("sessions.deleteSelected").props.onPress());
  const sheet = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  act(() => sheet.findAllByType(Button).find(node => node.props.variant === "danger")!.props.onPress());
  await act(async () => { sheet.props.onDismiss(); });

  expect(activeCommands.deleteSession).toHaveBeenCalledWith("a1", "t-a1");
  expect(peerCommands.deleteSession).not.toHaveBeenCalled();
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:1").length).toBeGreaterThan(0);
  expect(dialogText()).toEqual(
    expect.arrayContaining(["common.error", "sessions.deletePartialFailure:1"]),
  );
  unreachableMachines.clear();
});

test("selecting a heading's conversations takes only that machine's", async () => {
  // Two machines call the workspace the same thing, and both have a "w9": the
  // group's own machine is what decides which rows a "select all" takes, so one
  // heading cannot pull in the other machine's conversations.
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan", { workspaceId: "w9", workspaceName: "Review" }),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);
  expect(headingNames()).toEqual(["Review", "Review"]);

  // The heading's own ⋯: both machines call the workspace "Review", so the
  // peer's is the second one on screen (the active machine's leads).
  const headingMenus = tree.root.findAll(node =>
    node.props.accessibilityLabel === "sessions.workspaceActions:Review"
    && typeof node.props.onPress === "function");
  expect(headingMenus).toHaveLength(2);
  act(() => headingMenus[1]!.props.onPress());
  const menu = tree.root.findByType(ActionMenu);
  const selectAll = menu.props.actions.find(
    (action: { label: string }) => action.label === "sessions.selectWorkspaceSessions",
  );
  expect(selectAll.disabled).toBe(false);
  act(() => button("sessions.selectWorkspaceSessions").props.onPress());
  await act(async () => { menu.findByType(Modal).props.onDismiss(); });

  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:1").length).toBeGreaterThan(0);
  expect(button("Remote review").props.accessibilityState.checked).toBe(true);
  act(() => button("chat.cancel").props.onPress());
});

test("a batch with no machine left to answer offers no confirmation", () => {  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);
  act(() => button("sessions.select").props.onPress());
  act(() => button("sessions.selectVisible").props.onPress());
  machinesAnswer = false;
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);

  // The button is disabled, and a tap that began before the disable still
  // reaches the handler — which must not put a confirmation on screen for a
  // delete no machine could carry out.
  expect(button("sessions.deleteSelected").props.disabled).toBe(true);
  act(() => button("sessions.deleteSelected").props.onPress());
  expect(tree.root.findAllByType(Modal).some(node => node.props.visible)).toBe(false);
  machinesAnswer = true;
});

test("two machines' same-named workspace folds one at a time", () => {
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote plan"),
  ]);
  // Both machines call the workspace "Project": one heading each, and the peer's
  // key carries its machine so folding it leaves the local one open.
  expect(headingNames()).toEqual(["Project", "Project"]);
  const headings = tree.root.findAll(node =>
    node.props.accessibilityRole === "button"
    && typeof node.props.onPress === "function"
    && node.findAllByType(Text).some(text => text.props.children === "Project"));
  act(() => headings[headings.length - 1]!.props.onPress());

  expect(mergedTitles()).toEqual(["Local plan"]);
});

test("a row's own machine is named only when several machines are in the list", () => {
  renderMerged([
    mergedRow("desk-a", "a1", "Local plan"),
    mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" }),
  ]);
  const glyphs = () => tree.root.findAll(
    node => typeof node.props.accessibilityLabel === "string"
      && node.props.accessibilityLabel.startsWith("desk-")
      && typeof node.props.children === "string",
  ).map(node => node.props.children);
  expect(glyphs()).toContain("💻");

  // One machine: the selector above the list already names it, and every row
  // repeating its glyph would take the width the title needs.
  renderMerged(
    [mergedRow("desk-b", "b1", "Remote review", { workspaceId: "w9", workspaceName: "Review" })],
    "desk-b",
  );
  expect(glyphs()).toEqual([]);
});

/** The `+`/`−` in front of a conversation, in list order. */
const foldToggles = () =>
  tree.root.findAll(node =>
    typeof node.props.onPress === "function"
    && (node.props.accessibilityLabel === "sessions.expandChildren"
      || node.props.accessibilityLabel === "sessions.collapseChildren"),
  );

/**
 * A row in these tests' own workspace.
 *
 * Workspace folds are persisted (in storage and in a module-level cache), so a
 * fold another test performed would hide this test's rows; a workspace of its
 * own is what keeps a tree assertion about order and indent independent of
 * which tests ran before it. `lastMessageAt` is explicit because a fork is
 * *newer* than the conversation it came from by construction — the fixture that
 * hid the desktop's version of this bug had it older.
 */
const forkRow = (
  desktopId: string,
  sessionId: string,
  title: string,
  extra: Partial<RemoteSession> = {},
  lastMessageAt = 1,
) => mergedRow(
  desktopId,
  sessionId,
  title,
  { workspaceId: "w-forks", workspaceName: "Forks", ...extra },
  lastMessageAt,
);

/**
 * The merged list draws a conversation's forks under it — on either machine.
 *
 * A merged list is not a list of roots: these rows carry the same parent here
 * as they do in the single-machine list, and drawing them flat lost the tree —
 * a fork could not be told from a conversation of its own, and a conversation
 * with thirty forks could not be folded away. Asserted by order and indent
 * rather than by "the row is somewhere on screen": a fork's own last message is
 * newer than its conversation's, so a list that ranked rows by time put the
 * fork above the row it came from, and pressing `+` looked like it did nothing
 * because the row had been on screen all along.
 */
test("the merged list draws each machine's forks under the conversation they came from", () => {
  renderMerged([
    forkRow("desk-a", "a1", "Local parent", {}, 5),
    forkRow("desk-b", "b1", "Remote parent", {}, 4),
    forkRow("desk-a", "a2", "Local fork", { parentSessionId: "a1" }, 9),
    forkRow("desk-b", "b2", "Remote fork", { parentSessionId: "b1" }, 8),
  ]);

  // Folded by default, exactly as in the single-machine list: the two
  // conversations, each with the `+` that unfolds its own forks.
  expect(mergedTitles()).toEqual(["Local parent", "Remote parent"]);
  expect(rowInset("Local parent")).toBe(ROW_INSET);
  expect(foldToggles()).toHaveLength(2);

  act(() => foldToggles()[0]!.props.onPress());
  expect(mergedTitles()).toEqual(["Local parent", "Local fork", "Remote parent"]);

  act(() => foldToggles()[1]!.props.onPress());
  expect(mergedTitles()).toEqual(["Local parent", "Local fork", "Remote parent", "Remote fork"]);
  // One level in, so a fork's title starts on the column the conversation's
  // title does — the geometry both lists and the desktop app use.
  expect(rowInset("Local fork")).toBe(ROW_INSET + TOGGLE_WIDTH + TOGGLE_GAP);
  expect(rowInset("Remote fork")).toBe(ROW_INSET + TOGGLE_WIDTH + TOGGLE_GAP);
});

/**
 * A fork belongs to the machine that issued its conversation's id.
 *
 * A session id is unique per machine, not globally: two machines can each hold
 * a conversation called `m1`, and both can have a fork of it. The tree is built
 * one machine at a time for the same reason a row is addressed by
 * `(machine, id)` — building it over the merged rows as one list handed one
 * machine's fork to the other's conversation, and the fold follows: unfolding
 * the peer's conversation leaves this machine's shut, exactly as the merged
 * list's workspace headings keep one machine's fold off another's.
 */
test("a fork is not adopted by another machine's conversation with the same id", () => {
  renderMerged([
    forkRow("desk-a", "m1", "Local one", {}, 5),
    forkRow("desk-b", "m1", "Remote one", {}, 4),
    forkRow("desk-a", "m2", "Local fork", { parentSessionId: "m1" }, 3),
    forkRow("desk-b", "m2", "Remote fork", { parentSessionId: "m1" }, 2),
  ]);

  // Each machine's `m1` carries exactly its own fork.
  const rowFolds = (title: string) =>
    ["Plus", "Minus"].flatMap(name =>
      sessionBody(title).parent!.findAll(node => node.type === name));
  expect(rowFolds("Local one")).toHaveLength(1);
  expect(rowFolds("Remote one")).toHaveLength(1);
  expect(mergedTitles()).toEqual(["Local one", "Remote one"]);

  act(() => foldToggles()[1]!.props.onPress());
  expect(mergedTitles()).toEqual(["Local one", "Remote one", "Remote fork"]);
  act(() => foldToggles()[0]!.props.onPress());
  expect(mergedTitles()).toEqual(["Local one", "Local fork", "Remote one", "Remote fork"]);
});

/**
 * A fork follows its conversation into the section it was filed in.
 *
 * A pinned conversation is a shortcut promoted above the groups, and its forks
 * are part of it: filed by the fork's own fields they would sit under the
 * workspace heading, away from the `+` the reader just pressed, while the same
 * list on one machine shows them directly underneath. A fork is a
 * conversation's, so it takes that conversation's section — here and on the
 * desktop app alike.
 */
test("a fork stays beside a pinned conversation in the shortcut section", () => {
  renderMerged([
    forkRow("desk-a", "p1", "Pinned parent", { pinned: true }, 5),
    forkRow("desk-a", "p2", "Fork of pinned", { parentSessionId: "p1" }, 9),
    forkRow("desk-b", "q1", "Remote parent", {}, 4),
  ]);

  expect(mergedTitles()).toEqual(["Pinned parent", "Remote parent"]);
  act(() => button("sessions.expandChildren").props.onPress());
  expect(mergedTitles()).toEqual(["Pinned parent", "Fork of pinned", "Remote parent"]);
  expect(rowInset("Fork of pinned")).toBe(ROW_INSET + TOGGLE_WIDTH + TOGGLE_GAP);
});
