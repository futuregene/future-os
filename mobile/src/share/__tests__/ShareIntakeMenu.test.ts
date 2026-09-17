import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Platform } from "react-native";
import { ActionMenu } from "../../components/ActionMenu";
import type { RemoteSession } from "../../remote/types";
import { ShareIntakeMenu } from "../ShareIntakeMenu";
const mockRemote = {
  credentials: { expectedDesktopId: "d" },
  workspaces: [{ id: "w", name: "Project" }],
  sessions: [] as RemoteSession[],
};
const mockIntake = {
  pending: { desktopId: "d" } as { desktopId: string } | null,
  dismiss: jest.fn(),
  chooseDestination: jest.fn(),
};
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../useShareIntake", () => ({ useShareIntake: () => mockIntake }));
jest.mock("lucide-react-native", () => Object.fromEntries(["ArrowLeft", "Folder", "MessageCircle", "X"].map(name => [name, name])));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, values?: { name?: string; title?: string }) =>
      [key, values?.name, values?.title].filter(Boolean).join(":"),
  }),
}));
let tree: ReactTestRenderer;
const menu = () => tree.root.findByType(ActionMenu);
const labels = () => menu().props.actions.map((action: { label: string }) => action.label);
const press = (label: string) => act(() => {
  tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!.props.onPress();
});
const update = () => act(() => tree.update(createElement(ShareIntakeMenu)));
beforeEach(() => {
  jest.clearAllMocks();
  Platform.OS = "ios";
  mockIntake.pending = { desktopId: "d" };
  mockIntake.dismiss.mockImplementation(() => { mockIntake.pending = null; });
  mockRemote.credentials.expectedDesktopId = "d";
  mockRemote.sessions = [];
  act(() => { tree = create(createElement(ShareIntakeMenu)); });
});
afterEach(() => { act(() => tree.unmount()); Platform.OS = "ios"; jest.useRealTimers(); });

test("first asks new or existing, then offers new-chat destinations without dismissing the share", () => {
  expect(menu().props.visible).toBe(true);
  expect(labels()).toEqual(["share.newConversation", "share.existingConversation"]);
  expect(menu().props.actions[1].disabled).toBe(true);
  press("share.newConversation");
  expect(menu().props.title).toBe("share.newConversation");
  expect(labels()).toEqual(["common.back", "share.chat", "share.workspace:Project"]);
  expect(mockIntake.dismiss).not.toHaveBeenCalled();
  expect(mockIntake.chooseDestination).not.toHaveBeenCalled();
  expect(menu().props.visible).toBe(true);
});

test.each(["ios", "android"])("%s releases the modal only after the final destination is chosen", platform => {
  Platform.OS = platform as typeof Platform.OS;
  jest.useFakeTimers();
  press("share.newConversation");
  press("share.workspace:Project");
  expect(mockIntake.dismiss).toHaveBeenCalledTimes(1);
  expect(mockIntake.chooseDestination).not.toHaveBeenCalled();
  act(() => {
    if (platform === "ios") tree.root.findByType(Modal).props.onDismiss();
    else jest.runAllTimers();
  });
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("workspace", "w");
});

test("a new non-workspace chat remains available", () => {
  press("share.newConversation");
  press("share.chat");
  act(() => tree.root.findByType(Modal).props.onDismiss());
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("chat");
});

test("existing mode lists only existing chat and workspace sessions, including untitled sessions", () => {
  mockRemote.sessions = [
    { sessionId: "chat", threadId: "t1", title: "Recent chat", streaming: false },
    { sessionId: "work", threadId: "t2", title: "Task", mode: "workspace", workspaceId: "w", streaming: true },
    { sessionId: "untitled", threadId: "t3", title: "  ", streaming: false },
  ];
  update();
  press("share.existingConversation");
  expect(labels()).toEqual([
    "common.back", "share.existing:Recent chat", "share.existingWorkspace:Project:Task", "share.existing:sessions.unnamed",
  ]);
  expect(mockIntake.dismiss).not.toHaveBeenCalled();
  press("share.existingWorkspace:Project:Task");
  act(() => tree.root.findByType(Modal).props.onDismiss());
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("session", "work");
});

test("back changes mode without losing the pending content", () => {
  press("share.newConversation");
  press("common.back");
  expect(labels()).toEqual(["share.newConversation", "share.existingConversation"]);
  expect(mockIntake.dismiss).not.toHaveBeenCalled();
  expect(mockIntake.chooseDestination).not.toHaveBeenCalled();
});

test.each(["new", "existing"])("system back from %s preserves the share until back at the root", step => {
  mockRemote.sessions = [{ sessionId: "chat", threadId: "t", title: "Chat", streaming: false }];
  update();
  press(step === "new" ? "share.newConversation" : "share.existingConversation");
  act(() => tree.root.findByType(Modal).props.onRequestClose());
  expect(labels()).toEqual(["share.newConversation", "share.existingConversation"]);
  expect(mockIntake.dismiss).not.toHaveBeenCalled();
  expect(mockIntake.chooseDestination).not.toHaveBeenCalled();
  expect(menu().props.visible).toBe(true);
  act(() => tree.root.findByType(Modal).props.onRequestClose());
  expect(mockIntake.dismiss).toHaveBeenCalledTimes(1);
});

test.each(["kind", "new", "existing"])("cannot import into a different desktop at the %s step", step => {
  mockRemote.sessions = [{ sessionId: "chat", threadId: "t1", title: "Chat", streaming: false }];
  update();
  if (step !== "kind") press(step === "new" ? "share.newConversation" : "share.existingConversation");
  mockRemote.credentials.expectedDesktopId = "other";
  update();
  const destinations = menu().props.actions.filter((action: { label: string }) => action.label !== "common.back");
  expect(destinations.every((action: { disabled: boolean }) => action.disabled)).toBe(true);
});

test("cancelling the second step discards the share and resets the next share to mode selection", () => {
  press("share.newConversation");
  press("chat.cancel");
  expect(mockIntake.dismiss).toHaveBeenCalledTimes(1);
  expect(mockIntake.chooseDestination).not.toHaveBeenCalled();
  update();
  expect(menu().props.visible).toBe(false);
  mockIntake.pending = { desktopId: "d" };
  update();
  expect(menu().props.visible).toBe(true);
  expect(labels()).toEqual(["share.newConversation", "share.existingConversation"]);
});
