import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Alert } from "react-native";
import { Button } from "../../components/Button";
import { DesktopsScreen } from "../DesktopsScreen";

const mockRemote = {
  credentials: { pairId: "pair-1" },
  desktops: [
    { desktopId: "desktop-1", pairId: "pair-1" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ],
  switchDesktop: jest.fn(),
  removeDesktop: jest.fn(),
};
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("lucide-react-native", () => ({ Check: "Check", Monitor: "Monitor", Trash2: "Trash2" }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let tree: ReactTestRenderer;
const onBack = jest.fn();
const onAdd = jest.fn();
beforeEach(async () => {
  jest.clearAllMocks();
  mockRemote.switchDesktop.mockResolvedValue(undefined);
  mockRemote.removeDesktop.mockResolvedValue(undefined);
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
  await act(async () => { tree = create(createElement(DesktopsScreen, { onBack, onAdd })); });
});
afterEach(() => act(() => tree.unmount()));

const selectors = () => tree.root.findAll((node) =>
  typeof node.props.onPress === "function" && node.props.accessibilityState?.selected !== undefined,
);

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

test("opens the add-by-QR flow", () => {
  act(() => tree.root.findAllByType(Button).find((node) => node.props.label === "desktops.add")!.props.onPress());
  expect(onAdd).toHaveBeenCalled();
});

test("keeps the picker open and displays a failed switch", async () => {
  mockRemote.switchDesktop.mockRejectedValueOnce(new Error("storage unavailable"));
  await act(async () => selectors()[1]!.props.onPress());
  expect(onBack).not.toHaveBeenCalled();
  expect(tree.root.findAll((node) => node.props.accessibilityRole === "alert").length).toBeGreaterThan(0);
});

test("confirms removal of only the chosen desktop", async () => {
  const remove = tree.root.findAll((node) =>
    typeof node.props.onPress === "function" && node.props.accessibilityLabel === "sessions.unpair",
  );
  act(() => remove[1]!.props.onPress());
  const buttons = jest.mocked(Alert.alert).mock.calls[0]![2]!;
  await act(async () => { buttons.find((button) => button.style === "destructive")!.onPress!(); });
  expect(mockRemote.removeDesktop).toHaveBeenCalledWith("desktop-2");
});
