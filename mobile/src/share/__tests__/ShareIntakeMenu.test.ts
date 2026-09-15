import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ActionMenu } from "../../components/ActionMenu";
import type { RemoteSession } from "../../remote/types";
import { ShareIntakeMenu } from "../ShareIntakeMenu";
const mockRemote = {
  credentials: { expectedDesktopId: "d" },
  workspaces: [{ id: "w", name: "Project" }],
  sessions: [] as RemoteSession[],
};
const mockIntake = { pending: { desktopId: "d" }, dismiss: jest.fn(), chooseDestination: jest.fn() };
jest.mock("../../components/ActionMenu", () => ({ ActionMenu: "ActionMenu" }));
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../useShareIntake", () => ({ useShareIntake: () => mockIntake }));
jest.mock("lucide-react-native", () => ({ Folder: "Folder", MessageCircle: "MessageCircle" }));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, values?: { name?: string; title?: string }) =>
      [key, values?.name, values?.title].filter(Boolean).join(":"),
  }),
}));
let tree: ReactTestRenderer;
beforeEach(() => {
  jest.clearAllMocks();
  mockRemote.credentials.expectedDesktopId = "d";
  mockRemote.sessions = [];
  act(() => { tree = create(createElement(ShareIntakeMenu)); });
});
afterEach(() => act(() => tree.unmount()));
test("offers both a non-workspace conversation and each workspace with an empty catalog", () => {
  const menu = tree.root.findByType(ActionMenu);
  expect(menu.props.visible).toBe(true);
  expect(menu.props.title).toBe("share.chooseDestination");
  expect(menu.props.actions.map((action: { label: string }) => action.label)).toEqual(["share.chat", "share.workspace:Project"]);
  act(() => menu.props.actions[0].onPress());
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("chat");
  act(() => menu.props.actions[1].onPress());
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("workspace", "w");
});
test("offers existing chat and workspace sessions, including untitled sessions", () => {
  mockRemote.sessions = [
    { sessionId: "chat", threadId: "t1", title: "Recent chat", streaming: false },
    { sessionId: "work", threadId: "t2", title: "Task", mode: "workspace", workspaceId: "w", streaming: true },
    { sessionId: "untitled", threadId: "t3", title: "  ", streaming: false },
  ];
  act(() => tree.update(createElement(ShareIntakeMenu)));
  const menu = tree.root.findByType(ActionMenu);
  expect(menu.props.actions.map((action: { label: string }) => action.label)).toEqual([
    "share.chat",
    "share.existing:Recent chat",
    "share.existingWorkspace:Project:Task",
    "share.existing:sessions.unnamed",
    "share.workspace:Project",
  ]);
  for (const [index, session] of mockRemote.sessions.entries()) {
    act(() => menu.props.actions[index + 1].onPress());
    expect(mockIntake.chooseDestination).toHaveBeenLastCalledWith("session", session.sessionId);
  }
});
test("cannot import into a different desktop while choosing", () => {
  mockRemote.sessions = [{ sessionId: "chat", threadId: "t1", title: "Chat", streaming: false }];
  mockRemote.credentials.expectedDesktopId = "other";
  act(() => tree.update(createElement(ShareIntakeMenu)));
  expect(tree.root.findByType(ActionMenu).props.actions.every((action: { disabled: boolean }) => action.disabled)).toBe(true);
});
test("closing the menu discards the share without choosing a destination", () => {
  act(() => tree.root.findByType(ActionMenu).props.onClose());
  expect(mockIntake.dismiss).toHaveBeenCalledTimes(1);
  expect(mockIntake.chooseDestination).not.toHaveBeenCalled();
});
