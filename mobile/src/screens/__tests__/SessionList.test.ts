import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, BackHandler, FlatList, StyleSheet, Text, TextInput } from "react-native";
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
  deleteSession: jest.Mock;
  deleteWorkspace: jest.Mock;
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
  deleteSession: jest.fn(),
  deleteWorkspace: jest.fn(),
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
const button = (label: string) =>
  tree.root.findAll(
    node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function",
  )[0]!;
const sessionBody = (title: string) => tree.root.findAll(node =>
  node.props.accessibilityRole === "button" && typeof node.props.onLongPress === "function",
).find(node => node.findAllByType(Text).some(text => text.props.children === title))!;
beforeEach(async () => {
  jest.clearAllMocks();
  mockRemote.desktopOnline = true;
  mockRemote.deleteSession.mockResolvedValue(undefined);
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

/** Select a real app sheet row and finish its iOS dismissal before navigation. */
function pressWorkspaceMenu(index: number): void {
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  const menu = tree.root.findByType(ActionMenu);
  const label = menu.props.actions[index].label;
  const modal = menu.findByType(Modal);
  act(() => button(label).props.onPress());
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
function confirmAlert(): void {
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible);
  if (!modal) return;
  act(() => modal.findAllByType(Button).find(node => node.props.variant === "danger")!.props.onPress());
  act(() => modal.props.onDismiss());
}

test("a child in one workspace does not enlarge other workspace session gutters", () => {
  renderWorkspaceTab();
  mockRemote.workspaces = [...mockRemote.workspaces, { id: "w2", name: "Other", path: "/tmp/other" }];
  mockRemote.sessions = [...mockRemote.sessions, { sessionId: "w2a", threadId: "t2a", title: "Independent", mode: "workspace", workspaceId: "w2", streaming: false }];
  act(() => tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu, onTabChange })));
  const row = sessionBody("Independent").parent!;
  const gutter = row.findByProps({ testID: "session-expander-space" });
  expect(StyleSheet.flatten(gutter.props.style).width).toBe(12);
  act(() => button("sessions.expandChildren").props.onPress());
  expect(StyleSheet.flatten(sessionBody("Independent").parent!.findByProps({ testID: "session-expander-space" }).props.style).width).toBe(12);
  act(() => button("sessions.collapseChildren").props.onPress());
});

test.each(["workspace", "chat"] as const)("independent %s sessions keep their gutter when a neighboring tree changes", tab => {
  const originalSessions = mockRemote.sessions;
  const originalWorkspaces = mockRemote.workspaces;
  const update = () => act(() => tree.update(createElement(SessionList, { tab, empty: null, onMenu, onTabChange })));
  const gutterWidth = (title: string) => StyleSheet.flatten(
    sessionBody(title).parent!.findByProps({ testID: "session-expander-space" }).props.style,
  ).width;
  const rowIndent = (title: string) => StyleSheet.flatten(sessionBody(title).parent!.props.style).marginLeft;
  try {
    mockRemote.workspaces = [{ id: "gutter-workspace", name: "Gutter project", path: "/tmp/gutter" }];
    const common = { mode: tab, workspaceId: "gutter-workspace", streaming: false };
    const roots = [
      { ...common, sessionId: "gutter-independent", threadId: "gutter-thread-independent", title: "Independent root" },
      { ...common, sessionId: "gutter-parent", threadId: "gutter-thread-parent", title: "Neighboring parent" },
    ];
    mockRemote.sessions = roots;
    update();
    expect(gutterWidth("Independent root")).toBe(12);
    expect(rowIndent("Independent root")).toBe(0);

    mockRemote.sessions = [...roots, {
      ...common, sessionId: "gutter-child", threadId: "gutter-thread-child", title: "Nested child", parentSessionId: "gutter-parent",
    }];
    update();
    expect(gutterWidth("Independent root")).toBe(12);
    expect(rowIndent("Independent root")).toBe(0);
    expect(StyleSheet.flatten(button("sessions.expandChildren").props.style)).toMatchObject({ width: 44, minHeight: 44 });

    act(() => button("sessions.expandChildren").props.onPress());
    expect(gutterWidth("Independent root")).toBe(12);
    expect(rowIndent("Independent root")).toBe(0);
    expect(gutterWidth("Nested child")).toBe(44);
    expect(rowIndent("Nested child")).toBe(12);
    act(() => button("sessions.collapseChildren").props.onPress());
    expect(gutterWidth("Independent root")).toBe(12);

    act(() => button("sessions.search").props.onPress());
    act(() => tree.root.findByType(TextInput).props.onChangeText("Independent"));
    expect(gutterWidth("Independent root")).toBe(12);
    act(() => button("chat.cancel").props.onPress());
    expect(gutterWidth("Independent root")).toBe(12);

    act(() => button("sessions.select").props.onPress());
    expect(gutterWidth("Independent root")).toBe(0);
    act(() => button("sessions.expandChildren").props.onPress());
    expect(gutterWidth("Independent root")).toBe(0);
    expect(gutterWidth("Nested child")).toBe(44);
    act(() => button("sessions.collapseChildren").props.onPress());
    act(() => button("chat.cancel").props.onPress());
    expect(gutterWidth("Independent root")).toBe(12);

    mockRemote.sessions = roots;
    update();
    expect(gutterWidth("Independent root")).toBe(12);
    expect(gutterWidth("Neighboring parent")).toBe(12);
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
    "sessions.selectWorkspaceSessions",
    "sessions.deleteWorkspace",
  ]);
  act(() => button("chat.cancel").props.onPress());
  mockRemote.desktopOnline = false;
  renderWorkspaceTab();
  expect(button("sessions.workspaceActions:Project").props.disabled).toBe(true);
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
  pressWorkspaceMenu(1);
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
    pressWorkspaceMenu(2);
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

test("promoted workspace pins remain visible, openable and included in workspace selection", () => {
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
  pressWorkspaceMenu(1);
  expect(button("Follow-up").props.accessibilityState.checked).toBe(true);
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:2").length).toBeGreaterThan(0);
});

test("a failed workspace delete surfaces the workspace error instead of the generic one", async () => {
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    mockRemote.deleteWorkspace.mockRejectedValue(new Error("offline"));
    pressWorkspaceMenu(2);
    await act(async () => {
      confirmAlert();
    });
    expect(dialogText()).toEqual(expect.arrayContaining(["common.error", "sessions.deleteWorkspaceFailed"]));
  } finally {
    jest.useRealTimers();
  }
});
