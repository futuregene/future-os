import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ActionSheetIOS, Alert, TextInput } from "react-native";
import type { RemoteSession, RemoteWorkspace } from "../../remote/types";
import { SessionList } from "../SessionList";

const mockRemote: {
  sessions: RemoteSession[];
  workspaces: RemoteWorkspace[];
  unreadSessions: Set<string>;
  desktopOnline: boolean;
  deleteSession: jest.Mock;
  deleteWorkspace: jest.Mock;
  selectSession: jest.Mock;
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
      "Search",
      "Trash2",
      "X",
    ].map(name => [name, name]),
  ),
);
jest.mock("../../remote/RemoteContext", () => ({ useRemote: () => mockRemote }));
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
const button = (label: string) =>
  tree.root.findAll(
    node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function",
  )[0]!;
beforeEach(async () => {
  jest.clearAllMocks();
  mockRemote.desktopOnline = true;
  mockRemote.deleteSession.mockResolvedValue(undefined);
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
  await act(async () => {
    tree = create(createElement(SessionList, { tab: "chat", empty: null, onMenu }));
    // Let the persisted-folds read land inside act() so it cannot update the
    // tree after the test body has finished.
    await Promise.resolve();
  });
});
afterEach(() => {
  act(() => tree.unmount());
  jest.restoreAllMocks();
});

test("explicit row action opens rename/pin/delete menu; offline management is disabled", () => {
  act(() => button("sessions.actions:First").props.onPress());
  expect(onMenu).toHaveBeenCalledWith(mockRemote.sessions[0]);
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(SessionList, { tab: "chat", empty: null, onMenu })));
  expect(button("sessions.actions:First").props.disabled).toBe(true);
  expect(button("sessions.select").props.disabled).toBe(true);
});

test("search finds a collapsed child without changing folds", () => {
  expect(button("sessions.actions:Child")).toBeUndefined();
  act(() => tree.root.findByType(TextInput).props.onChangeText("Child"));
  expect(button("sessions.actions:Child")).toBeDefined();
  act(() => button("sessions.clearSearch").props.onPress());
  expect(button("sessions.actions:Child")).toBeUndefined();
});

test("cancelling deletion does not send any requests", () => {
  act(() => button("sessions.select").props.onPress());
  act(() => button("sessions.selectVisible").props.onPress());
  act(() => button("sessions.deleteSelected").props.onPress());
  const cancel = jest
    .mocked(Alert.alert)
    .mock.calls[0]![2]!.find(action => action.style === "cancel")!;
  act(() => cancel.onPress?.());
  expect(mockRemote.deleteSession).not.toHaveBeenCalled();
});

test("a successful batch exits selection and duplicate confirmation cannot delete twice", async () => {
  act(() => button("sessions.select").props.onPress());
  act(() => button("First").props.onPress());
  act(() => button("sessions.deleteSelected").props.onPress());
  const confirm = jest
    .mocked(Alert.alert)
    .mock.calls[0]![2]!.find(action => action.style === "destructive")!;
  await act(async () => {
    confirm.onPress?.();
    confirm.onPress?.();
  });
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
  expect(Alert.alert).toHaveBeenCalledWith(
    "sessions.deleteSelected",
    "sessions.deleteSelectedConfirm:2",
    expect.any(Array),
  );
  const confirm = jest
    .mocked(Alert.alert)
    .mock.calls[0]![2]!.find(action => action.style === "destructive")!;
  await act(async () => {
    confirm.onPress?.();
  });
  expect(mockRemote.deleteSession.mock.calls).toEqual([
    ["s1", "t1"],
    ["s2", "t2"],
  ]);
  expect(button("First").props.accessibilityState.checked).toBe(false);
  expect(button("Second").props.accessibilityState.checked).toBe(true);
  expect(Alert.alert).toHaveBeenLastCalledWith("common.error", "sessions.deletePartialFailure:1");
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
    tree.update(createElement(SessionList, { tab: "workspace", empty: null, onMenu }));
  });
}

/** Answer the next native action sheet with `index`; read the options it was
given afterwards (the returned getter sees the values the sheet received). */
function answerSheet(index: number): () => string[] {
  let options: string[] = [];
  jest
    .spyOn(ActionSheetIOS, "showActionSheetWithOptions")
    .mockImplementation((config, callback) => {
      options = config.options as string[];
      callback(index);
    });
  return () => options;
}

/** Press the workspace menu and run the iOS dismissal hop before the alert. */
function pressWorkspaceMenu(index: number): void {
  answerSheet(index);
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  // confirmDeleteWorkspace is deferred past UIKit's sheet dismissal (350ms).
  act(() => {
    jest.advanceTimersByTime(400);
  });
}

function confirmAlert(): void {
  const buttons = jest.mocked(Alert.alert).mock.calls.at(-1)![2]!;
  act(() => buttons.find(action => action.style === "destructive")!.onPress?.());
}

test("workspace menu offers the workspace actions and disables them offline", () => {
  renderWorkspaceTab();
  const options = answerSheet(2); // cancel
  act(() => button("sessions.workspaceActions:Project").props.onPress());
  expect(options()).toEqual([
    "sessions.selectWorkspaceSessions",
    "sessions.deleteWorkspace",
    "chat.cancel",
  ]);
  mockRemote.desktopOnline = false;
  renderWorkspaceTab();
  expect(button("sessions.workspaceActions:Project").props.disabled).toBe(true);
});

test("select all in a workspace selects nested sessions but not other chats", () => {
  renderWorkspaceTab();
  answerSheet(0);
  act(() => button("sessions.workspaceActions:Project").props.onPress());
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
    pressWorkspaceMenu(1);
    expect(Alert.alert).toHaveBeenLastCalledWith(
      "sessions.deleteWorkspace",
      "sessions.deleteWorkspaceConfirm:Project",
      expect.any(Array),
    );
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

test("a failed workspace delete surfaces the workspace error instead of the generic one", async () => {
  jest.useFakeTimers();
  try {
    renderWorkspaceTab();
    mockRemote.deleteWorkspace.mockRejectedValue(new Error("offline"));
    pressWorkspaceMenu(1);
    await act(async () => {
      confirmAlert();
    });
    expect(Alert.alert).toHaveBeenLastCalledWith("common.error", "sessions.deleteWorkspaceFailed");
  } finally {
    jest.useRealTimers();
  }
});
