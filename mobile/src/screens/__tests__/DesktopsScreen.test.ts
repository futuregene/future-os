import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Alert, Modal, Text, TextInput } from "react-native";
import { Button } from "../../components/Button";
import type { PairedDesktop } from "../../remote/types";
import { DesktopsScreen } from "../DesktopsScreen";

const mockRemote: {
  credentials: { pairId: string };
  desktops: PairedDesktop[];
  switchDesktop: jest.Mock;
  renameDesktop: jest.Mock;
  removeDesktop: jest.Mock;
} = {
  credentials: { pairId: "pair-1" },
  desktops: [
    { desktopId: "desktop-1", pairId: "pair-1" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ],
  switchDesktop: jest.fn(),
  renameDesktop: jest.fn(),
  removeDesktop: jest.fn(),
};
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("lucide-react-native", () => ({
  ArrowLeft: "ArrowLeft",
  Plus: "Plus",
  Check: "Check",
  Monitor: "Monitor",
  Pencil: "Pencil",
  Trash2: "Trash2",
}));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let tree: ReactTestRenderer;
const onBack = jest.fn();
const onAdd = jest.fn();

async function render(): Promise<void> {
  await act(async () => {
    tree = create(createElement(DesktopsScreen, { onBack, onAdd }));
  });
}

function pressables(accessibilityLabel: string) {
  return tree.root.findAll((node) =>
    typeof node.props.onPress === "function" && node.props.accessibilityLabel === accessibilityLabel,
  );
}

const selectors = () =>
  tree.root.findAll((node) =>
    typeof node.props.onPress === "function" && node.props.accessibilityState?.selected !== undefined,
  );

const button = (label: string) =>
  tree.root.findAllByType(Button).find((node) => node.props.label === label)!;

const texts = () => tree.root.findAllByType(Text).map((node) => node.props.children).flat().filter(
  (child): child is string => typeof child === "string",
);

beforeEach(async () => {
  jest.clearAllMocks();
  mockRemote.desktops = [
    { desktopId: "desktop-1", pairId: "pair-1" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ];
  mockRemote.switchDesktop.mockResolvedValue(undefined);
  mockRemote.renameDesktop.mockResolvedValue(undefined);
  mockRemote.removeDesktop.mockResolvedValue(undefined);
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
  await render();
});
afterEach(() => act(() => tree.unmount()));

test("lists both desktops and marks the current one", () => {
  expect(selectors()).toHaveLength(2);
  expect(selectors()[0]!.props.accessibilityState.selected).toBe(true);
  expect(selectors()[1]!.props.accessibilityState.selected).toBe(false);
});

test("selects a saved desktop without scanning again", async () => {
  await act(async () => selectors()[1]!.props.onPress());
  expect(mockRemote.switchDesktop).toHaveBeenCalledWith("desktop-2");
  expect(onBack).toHaveBeenCalled();
});

test("supports a startup picker without a back action", async () => {
  act(() => tree.unmount());
  await act(async () => {
    tree = create(createElement(DesktopsScreen, { onAdd }));
  });
  expect(pressables("common.back")).toHaveLength(0);
  await act(async () => selectors()[1]!.props.onPress());
  expect(mockRemote.switchDesktop).toHaveBeenCalledWith("desktop-2");
  expect(onBack).not.toHaveBeenCalled();
});

test("keeps the list in a centered column with side padding", () => {
  const column = tree.root.findAll(
    (node) => node.props.style?.alignSelf === "center" && node.props.style?.maxWidth > 0,
  )[0]!;
  expect(column.props.style.alignSelf).toBe("center");
  expect(column.props.style.paddingHorizontal).toBeGreaterThan(0);
});

test("opens the add-by-QR flow", () => {
  act(() => button("desktops.add").props.onPress());
  expect(onAdd).toHaveBeenCalled();
});

test("keeps the picker open and displays a failed switch", async () => {
  mockRemote.switchDesktop.mockRejectedValueOnce(new Error("storage unavailable"));
  await act(async () => selectors()[1]!.props.onPress());
  expect(onBack).not.toHaveBeenCalled();
  expect(tree.root.findAll((node) => node.props.accessibilityRole === "alert").length).toBeGreaterThan(0);
});

test("falls back to the desktop id until the user names it", async () => {
  expect(texts()).toEqual(expect.arrayContaining(["desktop-1", "desktop-2"]));
});

test("shows the chosen name above the id", async () => {
  mockRemote.desktops = [
    { desktopId: "desktop-1", pairId: "pair-1", name: "Studio Mac" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ];
  await render();
  expect(texts()).toEqual(expect.arrayContaining(["Studio Mac", "desktop-1", "desktop-2"]));
});

test("renames only the chosen desktop", async () => {
  act(() => pressables("desktops.rename")[1]!.props.onPress());
  act(() => tree.root.findByType(TextInput).props.onChangeText("Office PC"));
  await act(async () => button("chat.save").props.onPress());
  expect(mockRemote.renameDesktop).toHaveBeenCalledWith("desktop-2", "Office PC");
});

test("a blank name clears the custom name", async () => {
  mockRemote.desktops = [
    { desktopId: "desktop-1", pairId: "pair-1", name: "Studio Mac" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ];
  await render();
  act(() => pressables("desktops.rename")[0]!.props.onPress());
  act(() => tree.root.findByType(TextInput).props.onChangeText("   "));
  await act(async () => button("chat.save").props.onPress());
  expect(mockRemote.renameDesktop).toHaveBeenCalledWith("desktop-1", "");
});

test("saving an unchanged name does not rewrite storage", async () => {
  mockRemote.desktops = [
    { desktopId: "desktop-1", pairId: "pair-1", name: "Studio Mac" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ];
  await render();
  act(() => pressables("desktops.rename")[0]!.props.onPress());
  await act(async () => button("chat.save").props.onPress());
  expect(mockRemote.renameDesktop).not.toHaveBeenCalled();
});

test("reports a failed rename", async () => {
  mockRemote.renameDesktop.mockRejectedValueOnce(new Error("storage full"));
  act(() => pressables("desktops.rename")[1]!.props.onPress());
  act(() => tree.root.findByType(TextInput).props.onChangeText("Office PC"));
  await act(async () => button("chat.save").props.onPress());
  expect(tree.root.findAll((node) => node.props.accessibilityRole === "alert").length).toBeGreaterThan(0);
});

test("confirms removal of only the chosen desktop", async () => {
  act(() => pressables("sessions.unpair")[1]!.props.onPress());
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  act(() => modal.findAllByType(Button).find(node => node.props.variant === "danger")!.props.onPress());
  expect(mockRemote.removeDesktop).not.toHaveBeenCalled();
  await act(async () => { modal.props.onDismiss(); });
  expect(mockRemote.removeDesktop).toHaveBeenCalledWith("desktop-2");
});
