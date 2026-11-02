import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text, TextInput } from "react-native";
import { Button } from "../../components/Button";
import { peerIconId, PEER_ICONS } from "../../remote/peerIcons";
import type { PairedDesktop } from "../../remote/types";
import { DesktopLabelDialog } from "../DesktopLabelDialog";

const mockRemote: { labelDesktop: jest.Mock } = {
  labelDesktop: jest.fn(async () => {}),
};
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../../remote/PeerIconGlyph", () => ({ PeerIconGlyph: "PeerIconGlyph" }));
jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let tree: ReactTestRenderer;
const onClose = jest.fn();
let desktop: PairedDesktop;

const nameField = () => tree.root.findAllByType(TextInput)[0]!;
const iconOption = (label: string) => tree.root.findAll(node =>
  node.props.accessibilityLabel === label && node.props.accessibilityRole === "radio")[0]!;
const button = (label: string) => tree.root.findAllByType(Button).find(node => node.props.label === label)!;
const texts = () => tree.root.findAllByType(Text).map(node => node.props.children).flat()
  .filter((child): child is string => typeof child === "string");

async function mount(overrides: Partial<PairedDesktop> = {}): Promise<void> {
  desktop = { desktopId: "desktop", pairId: "pair", name: "Work computer", ...overrides };
  await act(async () => {
    tree = create(createElement(DesktopLabelDialog, { desktop, onClose }));
  });
}

beforeEach(async () => {
  jest.clearAllMocks();
  mockRemote.labelDesktop.mockResolvedValue(undefined);
  await mount();
});
afterEach(() => act(() => tree.unmount()));

test("opens on the label the phone already holds", () => {
  expect(nameField().props.value).toBe("Work computer");
  expect(nameField().props.placeholder).toBe("desktop");
  expect(iconOption("desktops.icons.desktop").props.accessibilityState.checked).toBe(true);
  expect(iconOption("desktops.icons.rocket").props.accessibilityState.checked).toBe(false);
});

test("a machine with no label of its own opens on its id and the default mark", async () => {
  act(() => tree.unmount());
  await mount({ name: undefined });
  expect(nameField().props.value).toBe("");
  expect(iconOption(`desktops.icons.${PEER_ICONS[0]!.labelKey}`).props.accessibilityState.checked).toBe(true);
});

/**
 * One patch, carrying only what changed: the store treats an omitted key as
 * "leave it", so sending both every time would rewrite a value that did not
 * need it.
 */
test("writes the typed name alone when the icon was left as it was", async () => {
  await act(async () => nameField().props.onChangeText("Studio"));
  await act(async () => button("desktops.save").props.onPress());
  expect(mockRemote.labelDesktop).toHaveBeenCalledWith("desktop", { name: "Studio" });
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("writes the picked icon alone when the name was left as it was", async () => {
  await act(async () => iconOption("desktops.icons.rocket").props.onPress());
  await act(async () => button("desktops.save").props.onPress());
  expect(mockRemote.labelDesktop).toHaveBeenCalledWith("desktop", { icon: "rocket" });
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("a blank name clears the label rather than storing spaces", async () => {
  await act(async () => nameField().props.onChangeText("   "));
  await act(async () => button("desktops.save").props.onPress());
  expect(mockRemote.labelDesktop).toHaveBeenCalledWith("desktop", { name: "" });
});

test("nothing changed writes nothing, and the way out is still there", async () => {
  expect(button("desktops.save").props.disabled).toBe(true);
  await act(async () => button("desktops.save").props.onPress());
  expect(mockRemote.labelDesktop).not.toHaveBeenCalled();
  act(() => button("desktops.cancel").props.onPress());
  expect(onClose).toHaveBeenCalledTimes(1);
});

/**
 * The book is local and can still refuse (read-only, locked). Closing on that
 * would report success for a label that was never stored, so the dialog stays
 * open with what the user typed and says what failed.
 */
test("a refused write is reported instead of closing on it", async () => {
  mockRemote.labelDesktop.mockRejectedValueOnce(new Error("storage full"));
  await act(async () => nameField().props.onChangeText("Studio"));
  await act(async () => button("desktops.save").props.onPress());
  expect(onClose).not.toHaveBeenCalled();
  expect(texts()).toContain("desktops.actionFailed");
  expect(nameField().props.value).toBe("Studio");
  // Retrying is still offered once the refusal is on screen.
  await act(async () => button("desktops.save").props.onPress());
  expect(mockRemote.labelDesktop).toHaveBeenCalledTimes(2);
});

/**
 * The grid has no "none" entry: the default mark is drawn by the default id, so
 * picking it is how an explicitly chosen icon is dropped, and the stored label
 * just says which id to draw.
 */
test("the default mark is one of the picks, and it can replace another", async () => {
  act(() => tree.unmount());
  await mount({ icon: "rocket" });
  expect(peerIconId("rocket")).toBe("rocket");
  expect(iconOption("desktops.icons.rocket").props.accessibilityState.checked).toBe(true);
  expect(iconOption(`desktops.icons.${PEER_ICONS[0]!.labelKey}`).props.accessibilityState.checked).toBe(false);
  await act(async () => iconOption(`desktops.icons.${PEER_ICONS[0]!.labelKey}`).props.onPress());
  await act(async () => button("desktops.save").props.onPress());
  expect(mockRemote.labelDesktop).toHaveBeenCalledWith("desktop", { icon: PEER_ICONS[0]!.id });
});

// The keyboard's "done" key is the way out of a one-field dialog on a phone;
// it saves exactly what the button does, and a dialog that only wired the
// button would leave that key doing nothing.
test("saving from the keyboard writes the same label as the button", async () => {
  await act(async () => nameField().props.onChangeText("Bench"));
  await act(async () => { await nameField().props.onSubmitEditing(); });
  expect(mockRemote.labelDesktop).toHaveBeenCalledWith("desktop", { name: "Bench" });
  expect(onClose).toHaveBeenCalled();
});
