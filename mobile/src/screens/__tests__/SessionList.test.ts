import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Alert, TextInput } from "react-native";
import { SessionList } from "../SessionList";

const mockRemote = {
  sessions: [
    { sessionId: "s1", threadId: "t1", title: "First", streaming: false },
    { sessionId: "s2", threadId: "t2", title: "Second", streaming: false },
    { sessionId: "child", threadId: "tc", title: "Child", parentSessionId: "s1", streaming: false },
  ],
  workspaces: [],
  unreadSessions: new Set<string>(),
  desktopOnline: true,
  deleteSession: jest.fn(),
  selectSession: jest.fn(),
};
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
beforeEach(() => {
  jest.clearAllMocks();
  mockRemote.desktopOnline = true;
  mockRemote.deleteSession.mockResolvedValue(undefined);
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
  act(() => {
    tree = create(createElement(SessionList, { tab: "chat", empty: null, onMenu }));
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
