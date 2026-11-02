import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Platform } from "react-native";
import { ActionMenu } from "../../components/ActionMenu";
import { showToast } from "../../features/chat/utils";
import type { MachineStatus, MachineSummary } from "../../remote/desktopStatus";
import type { RemoteSession } from "../../remote/types";
import { ShareIntakeMenu } from "../ShareIntakeMenu";

const mockRemote = {
  credentials: { expectedDesktopId: "d" } as { expectedDesktopId: string } | null,
  desktops: [{ desktopId: "d", pairId: "p1", name: "Laptop" }] as {
    desktopId: string;
    pairId?: string;
    name?: string;
    icon?: string;
  }[],
  presence: null as null | { control?: "active" | "standby"; holder?: { name?: string } | null },
  connectionPresentation: { level: "connected" } as { level: "connected" | "disconnected" },
  workspaces: [{ id: "w", name: "Project" }] as { id: string; name: string }[],
  sessions: [] as RemoteSession[],
  catalogs: {
    machines: [] as MachineSummary[],
    catalogs: [] as {
      desktopId: string;
      label: string;
      status: MachineStatus;
      sessions: RemoteSession[];
    }[],
    readWorkspaces: jest.fn(async (_desktopId: string): Promise<{ id: string; name: string }[]> => []),
  },
};
const mockIntake = {
  pending: {} as object | null,
  dismiss: jest.fn(),
  chooseDestination: jest.fn(),
};
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../useShareIntake", () => ({ useShareIntake: () => mockIntake }));
jest.mock("../../features/chat/utils", () => ({ showToast: jest.fn() }));
jest.mock("../../remote/PeerIconGlyph", () => ({ PeerIconGlyph: "PeerIconGlyph" }));
jest.mock("lucide-react-native", () => Object.fromEntries(["ArrowLeft", "Folder", "MessageCircle", "Search", "X"].map(name => [name, name])));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, values?: { name?: string; title?: string }) =>
      [key, values?.name, values?.title].filter(Boolean).join(":"),
  }),
}));
const readWorkspaces = mockRemote.catalogs.readWorkspaces as jest.MockedFunction<
  typeof mockRemote.catalogs.readWorkspaces
>;
const toast = showToast as jest.MockedFunction<typeof showToast>;
let tree: ReactTestRenderer;
const menu = () => tree.root.findByType(ActionMenu);
const labels = () => menu().props.actions.map((action: { label: string }) => action.label);
const tap = (label: string) => {
  tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!.props.onPress();
};
const press = (label: string) => act(() => { tap(label); });
const pressAsync = async (label: string) => {
  await act(async () => { tap(label); });
};
const update = () => act(() => tree.update(createElement(ShareIntakeMenu)));
const dismissModal = () => act(() => tree.root.findByType(Modal).props.onDismiss());
const systemBack = () => act(() => tree.root.findByType(Modal).props.onRequestClose());
/** A machine as the provider's catalogue reports it. */
const peer = (desktopId: string, status: MachineStatus = "connected", label = desktopId): MachineSummary =>
  ({ desktopId, label, active: false, status });
beforeEach(() => {
  jest.clearAllMocks();
  Platform.OS = "ios";
  mockIntake.pending = {};
  mockIntake.dismiss.mockImplementation(() => { mockIntake.pending = null; });
  mockRemote.credentials = { expectedDesktopId: "d" };
  mockRemote.desktops = [{ desktopId: "d", pairId: "p1", name: "Laptop" }];
  mockRemote.presence = null;
  mockRemote.connectionPresentation = { level: "connected" };
  mockRemote.sessions = [];
  mockRemote.catalogs.machines = [];
  mockRemote.catalogs.catalogs = [];
  readWorkspaces.mockResolvedValue([]);
  act(() => { tree = create(createElement(ShareIntakeMenu)); });
});
afterEach(() => { act(() => tree.unmount()); Platform.OS = "ios"; jest.useRealTimers(); });

test("first asks new or existing, then offers new-chat destinations without dismissing the share", () => {
  expect(menu().props.visible).toBe(true);
  expect(menu().props.title).toBe("share.chooseDestination");
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
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("d", "workspace", "w");
});

test("a new non-workspace chat remains available", () => {
  press("share.newConversation");
  press("share.chat");
  dismissModal();
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("d", "chat");
});

test("existing mode files workspace sessions under their workspace, chats as roots", () => {
  mockRemote.sessions = [
    { sessionId: "chat", threadId: "t1", title: "Recent chat", streaming: false },
    { sessionId: "work", threadId: "t2", title: "Task", mode: "workspace", workspaceId: "w", streaming: true },
    { sessionId: "untitled", threadId: "t3", title: "  ", streaming: false },
  ];
  update();
  press("share.existingConversation");
  expect(labels()).toEqual(["common.back", "Recent chat", "sessions.unnamed", "Project", "Task"]);
  expect(menu().props.actions[3].heading).toBe(true);
  expect(menu().props.actions[4].nested).toBe(true);
  expect(mockIntake.dismiss).not.toHaveBeenCalled();
  press("Task");
  dismissModal();
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("d", "session", "work");
});

test("searching flattens the tree into results that name their workspace", () => {
  mockRemote.sessions = [
    { sessionId: "chat", threadId: "t1", title: "Recent chat", streaming: false },
    { sessionId: "work", threadId: "t2", title: "Task", mode: "workspace", workspaceId: "w", streaming: true },
  ];
  update();
  press("share.existingConversation");
  act(() => menu().props.search.onChangeText("task"));
  expect(labels()).toEqual(["common.back", "share.existingWorkspace:Project:Task"]);
  expect(menu().props.actions.some((action: { heading?: boolean }) => action.heading)).toBe(false);
  press("share.existingWorkspace:Project:Task");
  dismissModal();
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("d", "session", "work");
});

test("a query that matches nothing says so instead of showing an empty sheet", () => {
  mockRemote.sessions = [{ sessionId: "work", threadId: "t2", title: "Task", mode: "workspace", workspaceId: "w", streaming: false }];
  update();
  press("share.existingConversation");
  act(() => menu().props.search.onChangeText("zzz"));
  expect(labels()).toEqual(["common.back", "sessions.noResults"]);
});

test("system back clears a search before it leaves the picker", () => {
  mockRemote.sessions = [{ sessionId: "work", threadId: "t2", title: "Task", mode: "workspace", workspaceId: "w", streaming: false }];
  update();
  press("share.existingConversation");
  act(() => menu().props.search.onChangeText("task"));
  systemBack();
  expect(menu().props.search.value).toBe("");
  expect(labels()).toEqual(["common.back", "Project", "Task"]);
  expect(mockIntake.dismiss).not.toHaveBeenCalled();
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
  systemBack();
  expect(labels()).toEqual(["share.newConversation", "share.existingConversation"]);
  expect(mockIntake.dismiss).not.toHaveBeenCalled();
  expect(mockIntake.chooseDestination).not.toHaveBeenCalled();
  expect(menu().props.visible).toBe(true);
  systemBack();
  expect(mockIntake.dismiss).toHaveBeenCalledTimes(1);
});

test("cancelling the second step discards the share and resets the next share to mode selection", () => {
  press("share.newConversation");
  press("chat.cancel");
  expect(mockIntake.dismiss).toHaveBeenCalledTimes(1);
  expect(mockIntake.chooseDestination).not.toHaveBeenCalled();
  update();
  expect(menu().props.visible).toBe(false);
  mockIntake.pending = {};
  update();
  expect(menu().props.visible).toBe(true);
  expect(labels()).toEqual(["share.newConversation", "share.existingConversation"]);
});

// ── which machine ───────────────────────────────────────────────────────────

/** Two paired machines, both able to take the share. */
function twoMachines(): void {
  mockRemote.desktops = [
    { desktopId: "d", pairId: "p1", name: "Laptop" },
    { desktopId: "o", pairId: "p2", name: "Office" },
  ];
  mockRemote.catalogs.machines = [peer("o", "connected", "Office")];
  update();
}

test("asks which machine first when more than one can take the share", async () => {
  twoMachines();
  expect(menu().props.title).toBe("share.chooseDevice");
  expect(labels()).toEqual(["Laptop", "Office"]);
  await pressAsync("Office");
  expect(readWorkspaces).toHaveBeenCalledWith("o");
  expect(menu().props.title).toBe("share.chooseDestination");
  // The mode step sits under the machine step, so it can go back to it.
  expect(labels()).toEqual(["common.back", "share.newConversation", "share.existingConversation"]);
  press("common.back");
  expect(menu().props.title).toBe("share.chooseDevice");
});

test("the machine step offers that machine's own workspaces", async () => {
  twoMachines();
  readWorkspaces.mockResolvedValue([{ id: "ow", name: "Office project" }]);
  await pressAsync("Office");
  press("share.newConversation");
  expect(labels()).toEqual(["common.back", "share.chat", "share.workspace:Office project"]);
  press("share.workspace:Office project");
  dismissModal();
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("o", "workspace", "ow");
});

test("the machine step offers that machine's own conversations", async () => {
  twoMachines();
  mockRemote.catalogs.catalogs = [{
    desktopId: "o",
    label: "Office",
    status: "connected",
    sessions: [{ sessionId: "s9", threadId: "t9", title: "Office chat", streaming: false }],
  }];
  await pressAsync("Office");
  press("share.existingConversation");
  expect(labels()).toEqual(["common.back", "Office chat"]);
  press("Office chat");
  dismissModal();
  expect(mockIntake.chooseDestination).toHaveBeenCalledWith("o", "session", "s9");
});

test("the machine being read needs no step of its own", async () => {
  twoMachines();
  await pressAsync("Laptop");
  expect(readWorkspaces).not.toHaveBeenCalled();
  expect(menu().props.title).toBe("share.chooseDestination");
  press("share.newConversation");
  expect(labels()).toEqual(["common.back", "share.chat", "share.workspace:Project"]);
});

test("a machine that cannot take the share is listed, disabled and inert", async () => {
  mockRemote.desktops = [
    { desktopId: "d", pairId: "p1", name: "Laptop" },
    { desktopId: "o", pairId: "p2", name: "Office" },
    { desktopId: "s", pairId: "p3", name: "Studio" },
  ];
  mockRemote.catalogs.machines = [peer("o", "offline", "Office"), peer("s", "connected", "Studio")];
  update();
  expect(labels()).toEqual(["Laptop", "Office", "Studio"]);
  expect(menu().props.actions[1].disabled).toBe(true);
  await pressAsync("Office");
  expect(readWorkspaces).not.toHaveBeenCalled();
  expect(menu().props.title).toBe("share.chooseDevice");
});

test("a machine that refuses the read says so and stays on the machine step", async () => {
  twoMachines();
  readWorkspaces.mockRejectedValue(new Error("machine_unavailable"));
  await pressAsync("Office");
  expect(toast).toHaveBeenCalledWith("common.error");
  expect(menu().props.title).toBe("share.chooseDevice");
});

test("a read that lands after the sheet was closed does not reopen the machine step", async () => {
  twoMachines();
  let release!: (workspaces: { id: string; name: string }[]) => void;
  readWorkspaces.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
  press("Office");
  press("chat.cancel");
  await act(async () => { release([{ id: "ow", name: "Office project" }]); });
  mockIntake.pending = {};
  update();
  // The next share starts at the machine step, with no machine carried over:
  // the read that landed after the sheet closed named nothing.
  expect(menu().props.title).toBe("share.chooseDevice");
});

test("a refused read that lands after the sheet was closed says nothing", async () => {
  twoMachines();
  let reject!: (error: Error) => void;
  readWorkspaces.mockImplementationOnce(() => new Promise((_resolve, fail) => { reject = fail; }));
  press("Office");
  press("chat.cancel");
  await act(async () => { reject(new Error("machine_unavailable")); });
  expect(toast).not.toHaveBeenCalled();
});

test("the machine step also opens when the only machine that can take the share is not the one being read", () => {
  mockRemote.connectionPresentation = { level: "disconnected" };
  mockRemote.desktops = [
    { desktopId: "d", pairId: "p1", name: "Laptop" },
    { desktopId: "o", pairId: "p2", name: "Office" },
  ];
  mockRemote.catalogs.machines = [peer("o", "connected", "Office")];
  update();
  expect(menu().props.title).toBe("share.chooseDevice");
  expect(menu().props.actions[0].disabled).toBe(true);
});

test("a machine that goes away while browsing disables the destinations it offered", async () => {
  twoMachines();
  readWorkspaces.mockResolvedValue([{ id: "ow", name: "Office project" }]);
  await pressAsync("Office");
  press("share.newConversation");
  mockRemote.desktops = [{ desktopId: "d", pairId: "p1", name: "Laptop" }];
  mockRemote.catalogs.machines = [];
  update();
  const destinations = menu().props.actions.filter(
    (action: { label: string }) => action.label !== "common.back",
  );
  expect(destinations.every((action: { disabled: boolean }) => action.disabled)).toBe(true);
});

test.each(["kind", "new", "existing"])("cannot import onto a machine that stopped answering at the %s step", step => {
  mockRemote.sessions = [{ sessionId: "chat", threadId: "t1", title: "Chat", streaming: false }];
  mockRemote.connectionPresentation = { level: "disconnected" };
  update();
  if (step !== "kind") press(step === "new" ? "share.newConversation" : "share.existingConversation");
  const destinations = menu().props.actions.filter((action: { label: string }) => action.label !== "common.back");
  expect(destinations.every((action: { disabled: boolean }) => action.disabled)).toBe(true);
});
