import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, ScrollView, StyleSheet, Text, View } from "react-native";
import { ConnectionBadge } from "../ConnectionBadge";
import { Button } from "../Button";
import { connectionPresentation } from "../../remote/connectionPresentation";

let mockDimensions = { width: 320, height: 640, scale: 1, fontScale: 1 };
jest.mock("react-native", () => {
  const actual = jest.requireActual("react-native");
  return new Proxy(actual, {
    get: (target, key) => key === "useWindowDimensions" ? () => mockDimensions : Reflect.get(target, key),
  });
});
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("lucide-react-native", () => ({ X: "X" }));
jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));

let tree: ReactTestRenderer;
const onReconnect = jest.fn();
const onUnpair = jest.fn();
const ready = connectionPresentation({ phase: "ready", desktopOnline: true });
const trigger = () => tree.root.findAll(node => node.props.accessibilityHint === "connection.showDetails" && typeof node.props.onPress === "function")[0]!;
const open = () => {
  // The RN preset's native views have no-op measurement methods.
  for (const node of tree.root.findAll(node => jest.isMockFunction(node.instance?.measureInWindow))) {
    node.instance.measureInWindow.mockImplementation((callback: (x: number, y: number, width: number, height: number) => void) => callback(220, 32, 44, 44));
  }
  act(() => trigger().props.onPress());
};
const text = () => tree.root.findAllByType(Text).map(node => node.props.children);
const render = (presentation = ready, active = true) => {
  const element = createElement(ConnectionBadge, { presentation, onReconnect, onUnpair, active });
  act(() => {
    tree.update(element);
  });
};
beforeEach(() => {
  mockDimensions = { width: 320, height: 640, scale: 1, fontScale: 1 };
  jest.clearAllMocks();
  act(() => { tree = create(createElement(ConnectionBadge, { presentation: ready, onReconnect, onUnpair })); });
});
afterEach(() => act(() => tree.unmount()));

test("only the dot is visible, with a 44pt button and an accessible status", () => {
  render();
  expect(text()).toEqual([]);
  const button = trigger();
  expect(button.props.accessibilityRole).toBe("button");
  expect(button.props.accessibilityLabel).toContain("connection.connected");
  expect(button.props.accessibilityState.expanded).toBe(false);
  const style = StyleSheet.flatten(button.props.style({ pressed: false }));
  expect(style.width).toBeGreaterThanOrEqual(44);
  expect(style.height).toBeGreaterThanOrEqual(44);
  open();
  expect(tree.root.findByType(Modal).props.visible).toBe(true);
  expect(trigger().props.accessibilityState.expanded).toBe(true);
  expect(text()).toContain("connection.statusDetails.connected.reason");
  expect(tree.root.findAllByType(Button)).toHaveLength(0);
  expect(onReconnect).not.toHaveBeenCalled();
});

test.each([
  [{ phase: "connecting", desktopOnline: false }, "connection.connecting", "connection.statusDetails.connecting.reason"],
  [{ phase: "ready", desktopOnline: false }, "connection.waitingDesktop", "connection.statusDetails.waitingDesktop.reason"],
  [{ phase: "ready", desktopOnline: true, agentAvailable: false }, "connection.waitingDesktop", "connection.statusDetails.devicePreparing.reason"],
] as const)("explains %j without offering an inappropriate reconnect", (state, label, hint) => {
  render(connectionPresentation(state));
  open();
  expect(text()).toContain(label);
  expect(text()).toContain(hint);
  expect(tree.root.findAllByType(Button)).toHaveLength(0);
  expect(onReconnect).not.toHaveBeenCalled();
});

test("pairing expiry is preserved as a status and only offers explicit unpair", () => {
  render(connectionPresentation({ phase: "revoked", desktopOnline: false }));
  open();
  expect(text()).toContain("connection.pairingExpired");
  expect(text()).toContain("connection.disconnectDetails.pairing.reason (PA001)");
  const action = tree.root.findByType(Button);
  expect(action.props.label).toBe("sessions.unpair");
  act(() => action.props.onPress());
  expect(onUnpair).toHaveBeenCalledTimes(1);
  expect(onReconnect).not.toHaveBeenCalled();
});

test.each([
  [{ phase: "unpaired", desktopOnline: false }, "connection.reconnect"],
  [{ phase: "failed", desktopOnline: false, error: "timed out" }, "connection.reconnect"],
  [{ phase: "failed", desktopOnline: false, error: "503" }, "connection.reconnect"],
] as const)("reconnect/retry for %j requires the separate action button", (state, label) => {
  render(connectionPresentation(state));
  open();
  expect(onReconnect).not.toHaveBeenCalled();
  const action = tree.root.findByType(Button);
  expect(action.props.label).toBe(label);
  act(() => action.props.onPress());
  expect(onReconnect).toHaveBeenCalledTimes(1);
  expect(tree.root.findByType(Modal).props.visible).toBe(false);
});

test.each(["outside", "close", "back"])("dismisses via %s without reconnecting", (method) => {
  render();
  open();
  act(() => {
    if (method === "back") tree.root.findByType(Modal).props.onRequestClose();
    else tree.root.findAll(node => typeof node.props.onPress === "function" && (method === "outside" ? node.props.accessible === false : node.props.accessibilityLabel === "common.close"))[0]!.props.onPress();
  });
  expect(tree.root.findByType(Modal).props.visible).toBe(false);
  expect(onReconnect).not.toHaveBeenCalled();
});

test("an open bubble updates when the connection becomes ready", () => {
  render(connectionPresentation({ phase: "ready", desktopOnline: false }));
  open();
  render(ready);
  expect(text()).toContain("connection.connected");
  expect(text()).not.toContain("connection.waitingDesktop");
});

test("narrow-screen content stays within safe edges and large text can wrap/scroll", () => {
  mockDimensions = { ...mockDimensions, fontScale: 2 };
  render();
  open();
  const popover = tree.root.findAllByType(View).find(node => node.props.accessibilityViewIsModal)!;
  const style = StyleSheet.flatten(popover.props.style);
  expect(style.width + style.right).toBeLessThanOrEqual(320 - 16);
  expect(style.right).toBeGreaterThanOrEqual(16);
  expect(style.top).toBe(80);
  expect(style.maxHeight + style.top).toBeLessThanOrEqual(640 - 34);
  expect(tree.root.findAllByType(ScrollView)).toHaveLength(1);
  expect(tree.root.findAllByType(Text).every(node => node.props.numberOfLines === undefined)).toBe(true);
});

test("rotation and hiding the screen dismiss the bubble without reopening it later", () => {
  render();
  open();
  mockDimensions = { ...mockDimensions, width: 640, height: 320 };
  render();
  expect(tree.root.findByType(Modal).props.visible).toBe(false);
  open();
  render(ready, false);
  render(ready, true);
  expect(tree.root.findByType(Modal).props.visible).toBe(false);
});
