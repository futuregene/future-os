import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text } from "react-native";
import {
  ConnectionStatusBlock,
  connectionState,
  connectionStateColors,
  machineDotColor,
} from "../ConnectionStatus";
import { Button } from "../Button";
import { connectionPresentation } from "../../remote/connectionPresentation";
import type { MachineSummary } from "../../remote/desktopStatus";
import { lightColors } from "../../theme/tokens";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("lucide-react-native", () => ({ X: "X" }));

const onReconnect = jest.fn();
const onUnpair = jest.fn();
const ready = connectionPresentation({ phase: "ready", desktopOnline: true });

let tree: ReactTestRenderer | null = null;
const text = () => tree!.root.findAllByType(Text).map(node => node.props.children);
const render = (
  presentation = ready,
  machines?: MachineSummary[],
  standby = false,
  holder: { name?: string } | null = null,
) => {
  act(() => {
    tree = create(createElement(ConnectionStatusBlock, {
      presentation, machines, standby, holder, onReconnect, onUnpair,
    }));
  });
};
beforeEach(() => jest.clearAllMocks());
afterEach(() => act(() => tree?.unmount()));

const machines = (...statuses: [string, MachineSummary["status"]][]): MachineSummary[] =>
  statuses.map(([desktopId, status], index) => ({
    desktopId,
    label: desktopId,
    active: index === 0,
    status,
  }));

test("a healthy connection says so and still offers the manual reconnect", () => {
  render();
  expect(text()).toContain("connection.connected");
  expect(text()).toContain("connection.statusDetails.connected.reason");
  const action = tree!.root.findByType(Button);
  expect(action.props.label).toBe("connection.reconnect");
  // Secondary: a healthy connection must not push the escape hatch forward.
  expect(action.props.variant).toBe("secondary");
  act(() => action.props.onPress());
  expect(onReconnect).toHaveBeenCalledTimes(1);
  expect(onUnpair).not.toHaveBeenCalled();
});

test.each([
  [{ phase: "ready", desktopOnline: false }, "connection.waitingDesktop", "connection.statusDetails.waitingDesktop.reason"],
  [{ phase: "ready", desktopOnline: true, agentAvailable: false }, "connection.waitingDesktop", "connection.statusDetails.devicePreparing.reason (LC003)"],
] as const)("explains %j without offering an inappropriate reconnect", (state, label, hint) => {
  render(connectionPresentation(state));
  expect(text()).toContain(label);
  expect(text()).toContain(hint);
  // The action is the desktop's ("open it", "restart it"), not one this phone
  // can perform from here.
  expect(tree!.root.findAllByType(Button)).toHaveLength(0);
});

test("pairing expiry is preserved as a status and only offers explicit unpair", () => {
  render(connectionPresentation({ phase: "revoked", desktopOnline: false }));
  expect(text()).toContain("connection.pairingExpired");
  expect(text()).toContain("connection.disconnectDetails.pairing.reason (PA001)");
  const action = tree!.root.findByType(Button);
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
  expect(onReconnect).not.toHaveBeenCalled();
  const action = tree!.root.findByType(Button);
  expect(action.props.label).toBe(label);
  // A disconnected connection leads with it.
  expect(action.props.variant).toBe("primary");
  act(() => action.props.onPress());
  expect(onReconnect).toHaveBeenCalledTimes(1);
});

test("a standby connection says so instead of claiming to be simply connected", () => {
  render(ready, [{ desktopId: "a", label: "a", active: true, status: "connected", control: "standby" }], true, {
    name: "Lab MacBook Air",
  });
  expect(text()).toContain("connection.connectedStandby");
  // …and names who has it, so the user knows what taking it back would cost.
  expect(text()).toContain("connection.controlHeldBy");
});

test("the block answers for the set, not for the open machine", () => {
  // The reported case: three desktops, two of them connected, and the machine
  // whose conversation is open is the one that is down. Its own state — here an
  // unclassified failure, LC999 — must not be laid under a list that says two
  // machines are connected.
  render(
    connectionPresentation({ phase: "failed", desktopOnline: false, error: "whatever it said" }),
    machines(["a", "offline"], ["b", "connected"], ["c", "connected"]),
  );
  expect(text()).toContain("connection.machinesPartial");
  expect(text()).toContain("connection.machinesSummary");
  expect(text()).toContain("connection.machinesPartialHint");
  expect(text()).not.toContain("connection.disconnected");
  expect(text()).not.toContain("connection.disconnectDetails.failed.reason (LC999)");
  // Reaching every machine is the offer, and the only one.
  expect(tree!.root.findByType(Button).props.label).toBe("connection.reconnectAll");
});

test("a machine that answers is not reported as the phone being down", () => {
  render(ready, machines(["a", "connected"], ["b", "offline"]));
  expect(text()).toContain("connection.machinesPartial");
  expect(text()).not.toContain("connection.connected");
});

test("with every machine connected the block keeps the connection's own words", () => {
  render(ready, machines(["a", "connected"], ["b", "connected"]));
  expect(text()).toContain("connection.connected");
  expect(text()).not.toContain("connection.machinesPartial");
});

test("with nothing answering the cause is the open machine's, code and all", () => {
  render(
    connectionPresentation({ phase: "failed", desktopOnline: false, error: "whatever it said" }),
    machines(["a", "offline"], ["b", "offline"]),
  );
  expect(text()).toContain("connection.disconnectDetails.failed.reason (LC999)");
  expect(text()).not.toContain("connection.machinesPartial");
});

test("an expired pairing is offered as an unpair, not as the set's state", () => {
  render(
    connectionPresentation({ phase: "revoked", desktopOnline: false }),
    machines(["a", "offline"], ["b", "connected"]),
  );
  expect(text()).toContain("connection.pairingExpired");
  expect(text()).not.toContain("connection.machinesPartial");
  expect(tree!.root.findByType(Button).props.label).toBe("sessions.unpair");
});

test("the dot is green only when every machine is connected and ours to drive", () => {
  const state = (list: MachineSummary[] | undefined, standby = false) =>
    connectionState({ machines: list, presentation: ready, standby });

  expect(state(machines(["a", "connected"], ["b", "connected"]))).toBe("up");
  // Yellow, not the text-tuned amber: that one is orange enough at this size to
  // be taken for the red the state below wears, and "one machine is down" must
  // not read as "all".
  expect(state(machines(["a", "connected"], ["b", "offline"]))).toBe("partial");
  // Nothing answers at all: red, whatever the remaining one is still trying.
  expect(state(machines(["a", "offline"], ["b", "connecting"]))).toBe("down");
  expect(state(machines(["a", "offline"], ["b", "offline"]))).toBe("down");
  // With no machine list the dot goes back to answering for the one connection
  // the app renders from.
  expect(state([])).toBe("up");
  expect(state(undefined, true)).toBe("partial");
});

test("a connected machine another device holds turns the whole set yellow", () => {
  // Connected is not the same as usable. Painting a standby machine green told
  // the user everything was fine while every command came back refused.
  const held: MachineSummary[] = [{
    desktopId: "a",
    label: "a",
    active: true,
    status: "connected",
    control: "standby",
  }];
  expect(connectionState({ machines: held, presentation: ready })).toBe("partial");
  // A machine we may drive does not paint the held one green: the dot answers
  // for the set, and the list the reader is looking at contains a machine this
  // phone cannot use.
  expect(connectionState({
    machines: [...held, { desktopId: "b", label: "b", active: false, status: "connected", control: "active" }],
    presentation: ready,
  })).toBe("partial");
});

test("the partial hue is yellow for the dot and readable amber for the text", () => {
  const { dot, text: title } = connectionStateColors(lightColors).partial;
  // The title is a line of text on a light surface, where yellow has no contrast
  // left to read it by: it keeps the darker amber for the same state.
  expect(dot).toBe(lightColors.warningDot);
  expect(title).toBe(lightColors.warning);
});

test("a machine's own dot wears the same palette, and standby is never green", () => {
  expect(machineDotColor({ status: "connected", control: "active" }, lightColors)).toBe(lightColors.success);
  expect(machineDotColor({ status: "connected", control: "standby" }, lightColors)).toBe(lightColors.warningDot);
  expect(machineDotColor({ status: "connecting" }, lightColors)).toBe(lightColors.warningDot);
  expect(machineDotColor({ status: "preparing" }, lightColors)).toBe(lightColors.warningDot);
  expect(machineDotColor({ status: "offline" }, lightColors)).toBe(lightColors.danger);
});
