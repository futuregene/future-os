import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ActionMenu } from "../../components/ActionMenu";
import { ShareIntakeMenu } from "../ShareIntakeMenu";
const mockRemote = { credentials: { expectedDesktopId: "d" }, workspaces: [{ id: "w", name: "Project" }] };
const mockIntake = { pending: { desktopId: "d" }, dismiss: jest.fn(), chooseDestination: jest.fn() };
jest.mock("../../components/ActionMenu", () => ({ ActionMenu: "ActionMenu" }));
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../useShareIntake", () => ({ useShareIntake: () => mockIntake }));
jest.mock("lucide-react-native", () => ({ Folder: "Folder", MessageCircle: "MessageCircle" }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string, values?: { name: string }) => values ? `${key}:${values.name}` : key }) }));
let tree: ReactTestRenderer;
beforeEach(() => {
  jest.clearAllMocks();
  mockRemote.credentials.expectedDesktopId = "d";
  act(() => { tree = create(createElement(ShareIntakeMenu)); });
});
afterEach(() => act(() => tree.unmount()));
test("offers both a non-workspace conversation and each workspace", () => {
  const menu = tree.root.findByType(ActionMenu);
  expect(menu.props.visible).toBe(true);
  expect(menu.props.actions.map((action: { label: string }) => action.label)).toEqual(["share.chat", "share.workspace:Project"]);
  act(() => menu.props.actions[0].onPress());
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("chat");
  act(() => menu.props.actions[1].onPress());
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("workspace", "w");
});
test("cannot import into a different desktop while choosing", () => {
  mockRemote.credentials.expectedDesktopId = "other";
  act(() => tree.update(createElement(ShareIntakeMenu)));
  expect(tree.root.findByType(ActionMenu).props.actions.every((action: { disabled: boolean }) => action.disabled)).toBe(true);
});
