import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
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
import type { RemoteSession, RemoteWorkspace } from "../../remote/types";
import { SessionList } from "../SessionList";
import { ActionMenu } from "../../components/ActionMenu";
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView", useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }) }));

const mockRemote: {
  sessions: RemoteSession[];
  workspaces: RemoteWorkspace[];
  unreadSessions: Set<string>;
  desktopOnline: boolean;
  capabilities: Set<string>;
  deleteSession: jest.Mock;
  deleteWorkspace: jest.Mock;
  setWorkspacePinned: jest.Mock;
  selectSession: jest.Mock;
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
  deleteSession: jest.fn(),
  deleteWorkspace: jest.fn(),
  setWorkspacePinned: jest.fn(),
  selectSession: jest.fn(),
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
      createElement(SessionList, { tab, empty: null, onMenu, onTabChange: tabChange }),
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
  mockRemote.desktopOnline = true;
  mockRemote.capabilities = new Set(["workspace_pinning_v1"]);
  mockRemote.deleteSession.mockResolvedValue(undefined);
  mockRemote.setWorkspacePinned.mockResolvedValue(undefined);
  mockRemote.newConversation.mockResolvedValue(undefined);
  await act(async () => {
    tree = create(createElement(SessionList, { tab: "chat", empty: null, onMenu, onTabChange }));
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
  expect(onMenu).toHaveBeenCalledWith(mockRemote.sessions[0]);
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(SessionList, { tab: "chat", empty: null, onMenu, onTabChange })));
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
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(SessionList, { tab: "chat", empty: null, onMenu, onTabChange })));
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
  expect(mockRemote.deleteSession).not.toHaveBeenCalled();
});

test("a successful batch exits selection and duplicate confirmation cannot delete twice", async () => {
  act(() => button("sessions.select").props.onPress());
  act(() => button("First").props.onPress());
  act(() => button("sessions.deleteSelected").props.onPress());
  const modal = activeDialog();
  const confirm = modal.findAllByType(Button).find(node => node.props.variant === "danger")!;
  act(() => { confirm.props.onPress(); confirm.props.onPress(); });
  expect(mockRemote.deleteSession).not.toHaveBeenCalled();
  await act(async () => { modal.props.onDismiss(); modal.props.onDismiss(); });
  expect(mockRemote.deleteSession).toHaveBeenCalledTimes(1);
  expect(button("sessions.select")).toBeDefined();
});

test("batch deletion confirms exact visible selection, preserves hidden children, and retains failures for retry", async () => {
  mockRemote.deleteSession.mockImplementation(async (id: string) => {
    if (id === "s2") throw new Error("offline");
  });
  act(() => button("sessions.select").props.onPress());
  // A collapsed child must never be implicitly selected by Select visible.
  act(() => button("sessions.selectVisible").props.onPress());
  act(() => button("sessions.deleteSelected").props.onPress());
  expect(mockRemote.deleteSession).not.toHaveBeenCalled();
  expect(dialogText()).toEqual(expect.arrayContaining(["sessions.deleteSelected", "sessions.deleteSelectedConfirm:2"]));
  await act(async () => { confirmAlert(); });
  expect(mockRemote.deleteSession.mock.calls).toEqual([
    ["s1", "t1"],
    ["s2", "t2"],
  ]);
  expect(button("First").props.accessibilityState.checked).toBe(false);
  expect(button("Second").props.accessibilityState.checked).toBe(true);
  expect(dialogText()).toEqual(expect.arrayContaining(["common.error", "sessions.deletePartialFailure:1"]));
});

test("select visible skips pinned chats, which carry no checkbox", () => {
  mockRemote.sessions = mockRemote.sessions.map(session => ({ ...session, pinned: session.sessionId === "s2" }));
  act(() => tree.update(createElement(SessionList, { tab: "chat", empty: null, onMenu, onTabChange })));
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
    tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu, onTabChange }));
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
  act(() => tree.update(createElement(SessionList, { tab: "chat", empty: null, onMenu, onTabChange })));
}

/** Select a real app sheet row and finish its iOS dismissal before navigation. */
function pressWorkspaceMenu(action: string): void {
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  const menu = tree.root.findByType(ActionMenu);
  expect(menu.props.actions.map((item: { label: string }) => item.label)).toContain(action);
  const modal = menu.findByType(Modal);
  act(() => button(action).props.onPress());
  act(() => modal.props.onDismiss());
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

/** The destructive button of the *confirmation dialog* (not a menu item), with
 * its handler left un-invoked so a double tap can land on it twice before the
 * surface is gone. */
function dangerPress(): () => void {
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible && node.findAllByType(DialogSurface).length > 0)!;
  const button = modal.findAllByType(Button).find(node => node.props.variant === "danger")!;
  return button.props.onPress;
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
    act(() => tree.update(createElement(SessionList, { tab, empty: null, onMenu, onTabChange })));

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
    act(() => tree.update(createElement(SessionList, { tab, empty: null, onMenu, onTabChange })));
  }
});

test("a child in one workspace does not enlarge other workspace session gutters", () => {
  renderWorkspaceTab();
  mockRemote.workspaces = [...mockRemote.workspaces, { id: "w2", name: "Other", path: "/tmp/other" }];
  mockRemote.sessions = [...mockRemote.sessions, { sessionId: "w2a", threadId: "t2a", title: "Independent", mode: "workspace", workspaceId: "w2", streaming: false }];
  act(() => tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu, onTabChange })));
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
    act(() => tree.update(createElement(SessionList, { tab, empty: null, onMenu, onTabChange })));
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
    act(() => tree.update(createElement(SessionList, { tab, empty: null, onMenu, onTabChange })));
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
  const update = () => act(() => tree.update(createElement(SessionList, { tab, empty: null, onMenu, onTabChange })));
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
    "sessions.selectWorkspaceSessions",
    "sessions.deleteWorkspace",
  ]);
  act(() => button("chat.cancel").props.onPress());
  mockRemote.desktopOnline = false;
  renderWorkspaceTab();
  expect(button("sessions.workspaceActions:Project").props.disabled).toBe(true);
});

test("old desktops do not offer unsupported workspace pin commands", () => {
  mockRemote.capabilities.clear();
  renderWorkspaceTab();
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  const actions = tree.root.findByType(ActionMenu).props.actions.map((action: { label: string }) => action.label);
  expect(actions).not.toContain("sessions.pin");
  expect(actions).not.toContain("sessions.unpin");
  expect(actions).toContain("sessions.new");
  expect(mockRemote.setWorkspacePinned).not.toHaveBeenCalled();
});

test("the workspace menu pins a group and offers unpin once it is pinned", async () => {
  renderWorkspaceTab();
  pressWorkspaceMenu("sessions.pin");
  expect(mockRemote.setWorkspacePinned).toHaveBeenCalledWith("w1", true);

  mockRemote.workspaces = [{ ...mockRemote.workspaces[0]!, pinned: true }];
  act(() => tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu, onTabChange })));
  pressWorkspaceMenu("sessions.unpin");
  expect(mockRemote.setWorkspacePinned).toHaveBeenLastCalledWith("w1", false);

  // A refused pin reports the failure instead of leaving the row moved.
  mockRemote.setWorkspacePinned.mockRejectedValueOnce(new Error("offline"));
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
    mockRemote.deleteWorkspace.mockResolvedValue(undefined);
    pressWorkspaceMenu("sessions.deleteWorkspace");
    expect(dialogText()).toEqual(expect.arrayContaining(["sessions.deleteWorkspace", "sessions.deleteWorkspaceConfirm:Project"]));
    await act(async () => {
      confirmAlert();
      confirmAlert(); // A second tap while the request is in flight must not re-fire.
    });
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledWith("w1");
  } finally {
    jest.useRealTimers();
  }
});

test("promoted workspace pins stay visible and openable but are left out of workspace selection", () => {
  renderWorkspaceTab();
  mockRemote.sessions = mockRemote.sessions.map(session => ({ ...session, pinned: session.sessionId === "w1b" }));
  act(() => tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu, onTabChange })));
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
    mockRemote.deleteWorkspace.mockRejectedValue(new Error("offline"));
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
  act(() => tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu, onTabChange, active: false })));
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(false);
  act(() => tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu, onTabChange, active: true })));
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
  expect(mockRemote.deleteSession).not.toHaveBeenCalled();
  expect(visibleDialog()).toBe(false);
  act(() => button("First").props.onPress());
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(SessionList, { tab: "chat", empty: null, onMenu, onTabChange })));
  act(() => button("sessions.deleteSelected").props.onPress());
  expect(mockRemote.deleteSession).not.toHaveBeenCalled();
  expect(visibleDialog()).toBe(false);
  mockRemote.desktopOnline = true;
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
  act(() => tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu, onTabChange })));
  pressWorkspaceMenu("sessions.selectWorkspaceSessions");
  // The shortcut must not open an empty batch: the toolbar would then offer a
  // delete with nothing selected.
  expect(button("sessions.deleteSelected")).toBeUndefined();
  expect(button("sessions.select")).toBeDefined();
});

test("a workspace menu does not open while the desktop is unreachable", () => {
  renderWorkspaceTab();
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu, onTabChange })));
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  // Pin/rename/delete are all desktop writes; opening the sheet on a dead link
  // would offer actions that cannot run.
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(false);
  mockRemote.desktopOnline = true;
});

test("a workspace cannot be deleted twice while the first request is in flight", async () => {
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    const pending = (() => {
      let resolve!: () => void;
      return { promise: new Promise<void>(done => { resolve = done; }), resolve: () => resolve() };
    })();
    mockRemote.deleteWorkspace.mockReturnValue(pending.promise);
    pressWorkspaceMenu("sessions.deleteWorkspace");
    await act(async () => { confirmAlert(); });
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
    // A tap on the workspace menu that began before the delete disabled it must
    // not queue a second destructive request.
    act(() => button("sessions.workspaceActions:Project").props.onPress());
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => { pending.resolve(); });
  } finally {
    jest.useRealTimers();
    mockRemote.deleteWorkspace.mockResolvedValue(undefined);
  }
});

test("a double tap on the delete confirmation cannot queue the batch twice", async () => {
  renderChatTab();
  // The desktop is slow to answer, so the first delete is still unresolved when
  // the second confirmation is confirmed.
  let resolveFirst!: () => void;
  mockRemote.deleteSession.mockReturnValueOnce(
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
  expect(mockRemote.deleteSession).toHaveBeenCalledTimes(1);
  // The second confirmation is still on screen; confirming it must not delete
  // the same conversations again — a thread the desktop already removed would
  // answer with an error the user would see for an action that had succeeded.
  await act(async () => { confirmAlert(); });
  expect(mockRemote.deleteSession).toHaveBeenCalledTimes(1);
  await act(async () => { resolveFirst(); await Promise.resolve(); });
  expect(mockRemote.deleteSession).toHaveBeenCalledTimes(1);
  mockRemote.deleteSession.mockResolvedValue(undefined);
});

test("a double tap on the workspace confirmation deletes it once", async () => {
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    let resolveDelete!: () => void;
    mockRemote.deleteWorkspace.mockReturnValueOnce(
      new Promise<void>(done => { resolveDelete = () => done(); }),
    );
    // Two confirmations are queued for the same workspace: the menu can be
    // reopened while the first confirmation is still on screen (the delete has
    // not started), so both requests reach the user.
    pressWorkspaceMenu("sessions.deleteWorkspace");
    pressWorkspaceMenu("sessions.deleteWorkspace");
    await act(async () => { confirmAlert(); });
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
    // The first request is still in flight and the second confirmation is on
    // screen; confirming it must not queue the same destructive request again.
    await act(async () => { confirmAlert(); });
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => { resolveDelete(); await Promise.resolve(); });
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
  } finally {
    jest.useRealTimers();
    mockRemote.deleteWorkspace.mockResolvedValue(undefined);
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
    mockRemote.deleteWorkspace.mockReturnValueOnce(
      new Promise<void>(done => { resolveDelete = () => done(); }),
    );

    act(() => button("sessions.workspaceActions:Project").props.onPress());
    const sheet = tree.root.findByType(ActionMenu).findByType(Modal);
    const press = button("sessions.deleteWorkspace").props.onPress as () => void;
    const flush = sheet.props.onDismiss as () => void;

    act(() => { press(); flush(); });
    expect(visibleDialogs()).toBe(1);
    await act(async () => { confirmAlert(); });
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledWith("w1");
    // The request really is in flight, so the guard's read at round 2 is a live
    // `true`; a delete that had already settled would make round 2 vacuous.
    expect(button("sessions.workspaceActions:Project").props.disabled).toBe(true);

    act(() => { press(); flush(); });
    // Nothing was re-queued. This count — not the request count, which would
    // only move if the stale confirmation were then tapped — is what fails when
    // the guard is dropped: round 2 would put a second confirmation on screen.
    expect(visibleDialogs()).toBe(0);
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);

    // Control, with the same captured pair: settle the delete, so the ref reads
    // false again, and drive it a third time. A confirmation *is* queued now —
    // which is what makes round 2's zero a statement about the guard rather than
    // about a closure that went dead when the sheet closed.
    await act(async () => { resolveDelete(); await Promise.resolve(); });
    expect(button("sessions.workspaceActions:Project").props.disabled).toBe(false);
    act(() => { press(); flush(); });
    expect(visibleDialogs()).toBe(1);
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
  } finally {
    jest.useRealTimers();
    mockRemote.deleteWorkspace.mockResolvedValue(undefined);
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
    mockRemote.deleteWorkspace.mockReturnValue(pending.promise);
    // Two requests reach the queue (the menu is re-openable, and a slow desktop
    // leaves the first one pending), so a second confirmation is on screen while
    // the first delete is still in flight.
    pressWorkspaceMenu("sessions.deleteWorkspace");
    pressWorkspaceMenu("sessions.deleteWorkspace");
    await act(async () => { confirmAlert(); });
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => { confirmAlert(); });
    // The second confirmation must not queue another destructive request.
    expect(mockRemote.deleteWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => { pending.resolve(); });
  } finally {
    jest.useRealTimers();
    mockRemote.deleteWorkspace.mockResolvedValue(undefined);
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
    mockRemote.deleteSession.mockReturnValue(pending.promise);
    act(() => button("sessions.select").props.onPress());
    act(() => button("First").props.onPress());
    act(() => button("sessions.deleteSelected").props.onPress());
    act(() => button("sessions.deleteSelected").props.onPress());
    await act(async () => { confirmAlert(); });
    expect(mockRemote.deleteSession).toHaveBeenCalledTimes(1);
    await act(async () => { confirmAlert(); });
    // A batch delete is a sequence of desktop writes; a second confirmation
    // while the first runs would interleave them and delete twice.
    expect(mockRemote.deleteSession).toHaveBeenCalledTimes(1);
    await act(async () => { pending.resolve(); });
  } finally {
    jest.useRealTimers();
    mockRemote.deleteSession.mockResolvedValue(undefined);
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
    mockRemote.deleteSession.mockReturnValue(pending.promise);
    act(() => button("sessions.select").props.onPress());
    act(() => button("First").props.onPress());
    act(() => button("sessions.deleteSelected").props.onPress());
    const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
    const press = modal.findAllByType(Button).find(node => node.props.variant === "danger")!.props.onPress;
    const flush = modal.props.onDismiss;
    act(() => { press(); flush(); });
    expect(mockRemote.deleteSession).toHaveBeenCalledTimes(1);
    // Same captured closures, second round: the request is still in flight.
    await act(async () => { press(); flush(); });
    expect(mockRemote.deleteSession).toHaveBeenCalledTimes(1);
    await act(async () => { pending.resolve(); });
  } finally {
    jest.useRealTimers();
    mockRemote.deleteSession.mockResolvedValue(undefined);
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
  expect(onMenu).toHaveBeenCalledWith(mockRemote.sessions[0]);
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
