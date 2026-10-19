import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, ScrollView, StyleSheet, Text, View } from "react-native";
import { ConnectionBadge } from "../ConnectionBadge";
import { Button } from "../Button";
import { connectionPresentation } from "../../remote/connectionPresentation";
import type { MachineSummary } from "../../remote/desktopStatus";
import { lightColors } from "../../theme/tokens";

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
const dotColor = () => {
  const dot = tree.root.findAll(node => node.props.testID === "connection-dot")[0]!;
  return StyleSheet.flatten(dot.props.style).backgroundColor;
};
const machineDotColor = (desktopId: string) => {
  const dot = tree.root.findAll(node => node.props.testID === `machine-dot-${desktopId}`)[0]!;
  return StyleSheet.flatten(dot.props.style).backgroundColor;
};
const titleColor = () => {
  // The popover's title is the one Text whose style overrides a single color —
  // its hue says which of the dot's states the connection is in.
  const title = tree.root.findAllByType(Text).find(node =>
    Array.isArray(node.props.style)
    && node.props.style.some((entry: { color?: string }) => entry?.color !== undefined))!;
  return StyleSheet.flatten(title.props.style).color;
};
const render = (
  presentation = ready,
  active = true,
  machines?: MachineSummary[],
  standby = false,
  holder: { name?: string } | null = null,
  onTakeControl?: (desktopId: string) => Promise<void>,
) => {
  const element = createElement(ConnectionBadge, {
    presentation, onReconnect, onUnpair, active, machines, standby, holder, onTakeControl,
  });
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
  expect(onReconnect).not.toHaveBeenCalled();
});

test.each([
  [{ phase: "ready", desktopOnline: true }, "connection.statusDetails.connected.reason"],
  [{ phase: "connecting", desktopOnline: false }, "connection.statusDetails.connecting.reason"],
] as const)("offers a manual reconnect while the status still reads healthy (%j)", (state, hint) => {
  render(connectionPresentation(state));
  open();
  expect(text()).toContain(hint);
  const action = tree.root.findByType(Button);
  expect(action.props.label).toBe("connection.reconnect");
  // Secondary: a healthy connection must not push the escape hatch forward.
  expect(action.props.variant).toBe("secondary");
  act(() => action.props.onPress());
  expect(onReconnect).toHaveBeenCalledTimes(1);
  expect(tree.root.findByType(Modal).props.visible).toBe(false);
});

test.each([
  [{ phase: "ready", desktopOnline: false }, "connection.waitingDesktop", "connection.statusDetails.waitingDesktop.reason"],
  [{ phase: "ready", desktopOnline: true, agentAvailable: false }, "connection.waitingDesktop", "connection.statusDetails.devicePreparing.reason (LC003)"],
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
  expect(action.props.variant).toBe("primary");
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

const machines = (...statuses: [string, MachineSummary["status"]][]): MachineSummary[] =>
  statuses.map(([desktopId, status], index) => ({
    desktopId,
    label: desktopId,
    active: index === 0,
    status,
  }));

test("the dot is green only when every machine is connected", () => {
  render(ready, true, machines(["a", "connected"], ["b", "connected"]));
  expect(dotColor()).toBe(lightColors.success);

  render(ready, true, machines(["a", "connected"], ["b", "offline"]));
  // Yellow, not the text-tuned amber: that one is orange enough at this size to
  // be taken for the red the state below wears, and "one machine is down" must
  // not read as "all".
  expect(dotColor()).toBe(lightColors.warningDot);

  render(ready, true, machines(["a", "offline"], ["b", "connecting"]));
  expect(dotColor()).toBe(lightColors.danger);

  // With no pairing the dot goes back to answering for the one connection the
  // app renders from.
  render(ready, true, []);
  expect(dotColor()).toBe(lightColors.success);
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
  render(ready, true, held);
  expect(dotColor()).toBe(lightColors.warningDot);

  // A machine we may drive does not paint the held one green: the dot answers
  // for the set, and the list the reader is looking at contains a machine this
  // phone cannot use. Which one it is, and the takeover, are the rows' job.
  render(ready, true, [...held, { desktopId: "b", label: "b", active: false, status: "connected", control: "active" }]);
  expect(dotColor()).toBe(lightColors.warningDot);
});

test("three machines up and the open one in standby is yellow, not green", () => {
  // The reported case: every desktop answered, so the dot was green — while the
  // machine the screen is about was held by another device, and nothing could be
  // done on it.
  render(
    ready,
    true,
    [
      { desktopId: "desk", label: "desk", active: true, status: "connected", control: "standby" },
      { desktopId: "phone", label: "phone", active: false, status: "connected", control: "active" },
      { desktopId: "home", label: "home", active: false, status: "connected", control: "active" },
    ],
    true,
    { name: "Studio iPad" },
  );

  expect(dotColor()).toBe(lightColors.warningDot);
  open();
  // The block's words belong to the open machine, so they say it is held —
  // naming the device this phone would displace, even with peers paired.
  expect(text()).toContain("connection.connectedStandby");
  expect(text()).toContain("connection.controlHeldBy");
  // The rows keep answering one machine each.
  expect(machineDotColor("desk")).toBe(lightColors.warningDot);
  expect(machineDotColor("phone")).toBe(lightColors.success);
});

test("a standby connection says so instead of claiming to be simply connected", () => {
  render(ready, true, [{ desktopId: "a", label: "a", active: true, status: "connected", control: "standby" }], true, {
    name: "Lab MacBook Air",
  });

  expect(dotColor()).toBe(lightColors.warningDot);
  open();
  expect(text()).toContain("connection.connectedStandby");
  // …and names who has it, so the user knows what taking it back would cost.
  expect(text()).toContain("connection.controlHeldBy");
});

test("the sheet explains a standby machine by name and offers the takeover there", () => {
  // The prompt used to sit on the list page; it belongs behind the dot, next to
  // the explanation of why nothing works.
  const take = jest.fn().mockResolvedValue(undefined);
  render(
    ready,
    true,
    [{ desktopId: "a", label: "Lab MacBook Air", active: true, status: "connected", control: "standby" }],
    true,
    { name: "Lab MacBook Air" },
    take,
  );
  open();

  // Passive voice, and the name is the one that would be disconnected.
  expect(text()).toContain("connection.controlHeldBy");
  const button = tree.root.findAll(node =>
    typeof node.props.label === "string" && node.props.label === "connection.takeControl")[0]!;
  expect(button).toBeDefined();
  act(() => button.props.onPress());
  expect(take).toHaveBeenCalledWith("a");
});

test("a machine this phone drives is not offered a takeover", () => {
  render(
    ready,
    true,
    [{ desktopId: "a", label: "Lab MacBook Air", active: true, status: "connected", control: "active" }],
    false,
    null,
    jest.fn(),
  );
  open();

  const buttons = tree.root.findAll(node =>
    typeof node.props.label === "string" && node.props.label === "connection.takeControl");
  expect(buttons).toHaveLength(0);
});

test("a standby machine's row wears yellow, and its takeover acts on that row", () => {
  const take = jest.fn().mockResolvedValue(undefined);
  render(
    ready,
    true,
    [
      { desktopId: "a", label: "a", active: true, status: "connected", control: "active" },
      { desktopId: "b", label: "b", active: false, status: "connected", control: "standby" },
    ],
    true,
    null,
    take,
  );
  open();

  expect(machineDotColor("b")).toBe(lightColors.warningDot);
  expect(machineDotColor("a")).toBe(lightColors.success);
  const buttons = tree.root.findAll(node =>
    typeof node.props.label === "string" && node.props.label === "connection.takeControl");
  expect(buttons).toHaveLength(1);
  act(() => buttons[0]!.props.onPress());
  // The row's own machine, not the one the phone happens to be reading.
  expect(take).toHaveBeenCalledWith("b");
});

test("the partial dot is yellow while the title under it stays readable", () => {
  render(ready, true, machines(["a", "connected"], ["b", "offline"]));
  open();

  expect(dotColor()).toBe(lightColors.warningDot);
  // The title is a line of text on a light surface, where yellow has no contrast
  // left to read it by: it keeps the darker amber for the same state.
  expect(titleColor()).toBe(lightColors.warning);
});

test("a machine's own row wears the badge's palette", () => {
  render(ready, true, machines(["a", "connected"], ["b", "connecting"], ["c", "offline"]));
  open();

  expect(machineDotColor("a")).toBe(lightColors.success);
  expect(machineDotColor("b")).toBe(lightColors.warningDot);
  expect(machineDotColor("c")).toBe(lightColors.danger);
});

test("a merged list names every machine and its own state, and reconnects them all", () => {
  render(ready, true, machines(["a", "connected"], ["b", "preparing"], ["c", "offline"]));
  open();

  expect(text()).toContain("connection.machine.connected");
  expect(text()).toContain("connection.machine.preparing");
  expect(text()).toContain("connection.machine.offline");
  expect(text()).toContain("connection.machinesSummary");
  const action = tree.root.findByType(Button);
  expect(action.props.label).toBe("connection.reconnectAll");
  act(() => action.props.onPress());
  expect(onReconnect).toHaveBeenCalledTimes(1);
});

test("one machine needs no list of its own", () => {
  // The picker above the list already names it, and the title below says how it
  // is doing: a one-row list would repeat both.
  render(ready, true, machines(["a", "connected"]));
  open();

  expect(text()).not.toContain("connection.machine.connected");
  expect(tree.root.findByType(Button).props.label).toBe("connection.reconnect");
});

test("the block under the rows answers for the set, not for the open machine", () => {
  // The reported case: three desktops, two of them connected, and the machine
  // whose conversation is open is the one that is down. Its own state — here an
  // unclassified failure, LC999 — must not be laid under a row list that says
  // two machines are connected.
  render(
    connectionPresentation({ phase: "failed", desktopOnline: false, error: "whatever it said" }),
    true,
    machines(["a", "offline"], ["b", "connected"], ["c", "connected"]),
  );
  open();

  expect(text()).toContain("connection.machinesPartial");
  expect(text()).toContain("connection.machinesSummary");
  expect(text()).toContain("connection.machinesPartialHint");
  expect(text()).not.toContain("connection.disconnected");
  expect(text()).not.toContain("connection.disconnectDetails.failed.reason (LC999)");
  // The count is one fact: it is the block's description rather than a caption
  // repeated a line above it.
  expect(text().filter(value => value === "connection.machinesSummary")).toHaveLength(1);
  // ...and reaching every machine is still the offer, now the only one.
  expect(tree.root.findByType(Button).props.label).toBe("connection.reconnectAll");
});

test("a machine that answers is not reported as the phone being down", () => {
  // The other way round: the open machine is fine and a peer is not. The dot and
  // the rows already said so, and the block must not say "connected" instead.
  render(ready, true, machines(["a", "connected"], ["b", "offline"]));
  open();

  expect(text()).toContain("connection.machinesPartial");
  expect(text()).not.toContain("connection.connected");
});

test("with every machine connected the block keeps the connection's own words", () => {
  render(ready, true, machines(["a", "connected"], ["b", "connected"]));
  open();

  expect(text()).toContain("connection.connected");
  expect(text()).not.toContain("connection.machinesPartial");
  // The count still captions the rows it is about.
  expect(text()).toContain("connection.machinesSummary");
});

test("with nothing answering the cause is the open machine's, code and all", () => {
  render(
    connectionPresentation({ phase: "failed", desktopOnline: false, error: "whatever it said" }),
    true,
    machines(["a", "offline"], ["b", "offline"]),
  );
  open();

  expect(text()).toContain("connection.disconnectDetails.failed.reason (LC999)");
  expect(text()).not.toContain("connection.machinesPartial");
});

test("an expired pairing is offered as an unpair, not as the set's state", () => {
  render(
    connectionPresentation({ phase: "revoked", desktopOnline: false }),
    true,
    machines(["a", "offline"], ["b", "connected"]),
  );
  open();

  expect(text()).toContain("connection.pairingExpired");
  expect(text()).not.toContain("connection.machinesPartial");
  expect(tree.root.findByType(Button).props.label).toBe("sessions.unpair");
});
