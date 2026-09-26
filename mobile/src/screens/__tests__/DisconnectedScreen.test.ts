import { createElement } from "react";
import { ActivityIndicator } from "react-native";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Button } from "../../components/Button";
import { useRemote } from "../../remote/RemoteContext";
import { DisconnectedScreen } from "../DisconnectedScreen";
import i18n from "../../i18n";

jest.mock("../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("lucide-react-native", () => ({ Unplug: () => null }));

const useRemoteMock = useRemote as jest.MockedFunction<typeof useRemote>;

/** The smallest remote surface the screen reads, with a chosen presentation. */
function remoteFor(overrides: Record<string, unknown> = {}) {
  return {
    connectionPresentation: {
      level: "disconnected",
      customerState: "disconnected",
      action: "reconnect",
      supportCode: null,
      titleKey: "connection.disconnected",
      hintKey: "connection.offlineHint",
    },
    error: null,
    presence: undefined,
    ...overrides,
  } as unknown as ReturnType<typeof useRemote>;
}

let tree: ReactTestRenderer;
afterEach(() => { if (tree) act(() => tree.unmount()); });

function render(props: Partial<Parameters<typeof DisconnectedScreen>[0]> = {}) {
  act(() => {
    tree = create(createElement(DisconnectedScreen, {
      onReconnect: jest.fn(), onUnpair: jest.fn(), ...props,
    }));
  });
}
function painted() {
  return tree.root
    .findAll(node => typeof node.type === "string" && node.props.children !== undefined)
    .map(node => Array.isArray(node.props.children)
      ? node.props.children.filter((part: unknown) => typeof part === "string").join("")
      : String(node.props.children));
}

test("an ordinary disconnect explains the cause and offers a reconnect", () => {
  const onReconnect = jest.fn();
  useRemoteMock.mockReturnValue(remoteFor({ presence: { reason: "app_exit" } }));
  render({ onReconnect });
  const button = tree.root.findByType(Button);
  expect(button.props.label).toBe(i18n.t("connection.reconnect"));
  expect(button.props.loading).toBe(false);
  act(() => button.props.onPress());
  expect(onReconnect).toHaveBeenCalledTimes(1);
  // The screen explains the cause and what to do, not just that it failed.
  expect(painted()).toContain(i18n.t("connection.disconnectDetails.exit.reason"));
  expect(painted()).toContain(i18n.t("connection.disconnectDetails.exit.solution"));
});

test("a revoked pairing offers re-pairing instead of a reconnect that cannot work", () => {
  const onUnpair = jest.fn();
  const onReconnect = jest.fn();
  useRemoteMock.mockReturnValue(remoteFor({
    connectionPresentation: {
      action: "pairAgain", customerState: "pairingExpired", hintKey: null,
      level: "disconnected", supportCode: "PA001", titleKey: "connection.pairingExpired",
    },
  }));
  render({ onReconnect, onUnpair });
  expect(painted()).toContain(i18n.t("connection.pairingExpired"));
  const button = tree.root.findByType(Button);
  expect(button.props.label).toBe(i18n.t("sessions.unpair"));
  act(() => button.props.onPress());
  expect(onUnpair).toHaveBeenCalledTimes(1);
  expect(onReconnect).not.toHaveBeenCalled();
});

test("a support code is shown beside the reason so the user can quote it", () => {
  useRemoteMock.mockReturnValue(remoteFor({
    connectionPresentation: {
      action: "reconnect", customerState: "networkUnavailable", hintKey: "connection.offlineHint",
      level: "disconnected", supportCode: "NW002", titleKey: "connection.networkUnavailable",
    },
  }));
  render();
  expect(painted()).toContain(`${i18n.t("connection.disconnectDetails.timeout.reason")} (NW002)`);
  // The two codes that share the network state must not read the same.
  expect(`${i18n.t("connection.disconnectDetails.timeout.reason")} (NW002)`)
    .not.toContain("NW001");
});

test("while a reconnect is running the screen waits instead of offering a second attempt", () => {
  const onReconnect = jest.fn();
  useRemoteMock.mockReturnValue(remoteFor());
  render({ onReconnect, reconnecting: true });
  // A spinner replaces the unplug glyph and the button cannot be pressed again.
  expect(tree.root.findAllByType(ActivityIndicator).length).toBeGreaterThan(0);
  const button = tree.root.findByType(Button);
  expect(button.props.loading).toBe(true);
  expect(painted()).toContain(i18n.t("connection.connecting"));
  expect(painted()).toContain(i18n.t("connection.statusDetails.connecting.reason"));
  expect(painted()).toContain(i18n.t("connection.statusDetails.connecting.solution"));
  expect(onReconnect).not.toHaveBeenCalled();
});

test("a reconnecting screen never offers the destructive unpair, even for a revoked pairing", () => {
  useRemoteMock.mockReturnValue(remoteFor({
    connectionPresentation: {
      action: "pairAgain", customerState: "pairingExpired", hintKey: null,
      level: "disconnected", supportCode: "PA001", titleKey: "connection.pairingExpired",
    },
  }));
  render({ reconnecting: true });
  expect(tree.root.findByType(Button).props.label).toBe(i18n.t("connection.reconnect"));
  expect(painted()).toContain(i18n.t("connection.connecting"));
  expect(painted()).not.toContain(i18n.t("connection.pairingExpired"));
});
