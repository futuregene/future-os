import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { StyleSheet, Text } from "react-native";
import { DesktopPicker } from "../DesktopPicker";
import type { MachineSummary } from "../../remote/desktopStatus";
import type { PairedDesktop } from "../../remote/types";
import { connectionPresentation } from "../../remote/connectionPresentation";
import { lightColors } from "../../theme/tokens";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { name?: string }) => ({
      "desktops.pickerLabel": "Desktops",
      "desktops.filterAll": "All desktops",
      "desktops.manage": "Manage paired desktops…",
      "common.close": "Close",
      "connection.showDetails": "Show connection details and available actions",
      "connection.takeControl": "Take over",
      "connection.takeControlHint": "Taking over disconnects {{name}}",
      "connection.machine.connected": "Connected",
      "connection.machine.preparing": "Preparing",
      "connection.machine.connecting": "Connecting",
      "connection.machine.offline": "Offline",
      "connection.machineStandby": "Connected · in standby",
      "connection.machineStandbyHint": "Another device is in control",
      "connection.connected": "Connected",
      "connection.machinesPartial": "Some desktops are not connected",
      "connection.machinesSummary": "{{connected}} of {{total}} desktops connected",
      "connection.machinesPartialHint": "Make the missing desktops online, then reconnect",
      "connection.reconnect": "Reconnect",
      "connection.reconnectAll": "Reconnect all",
      "connection.statusDetails.connected.reason": "The phone is connected to the desktop",
      "connection.statusDetails.connected.solution": "",
    } as Record<string, string>)[key] ?? key,
  }),
}));
jest.mock("lucide-react-native", () => ({
  Check: "Check", ChevronDown: "ChevronDown", Layers: "Layers", Monitor: "Monitor", X: "X",
}));
jest.mock("../../remote/PeerIconGlyph", () => ({ PeerIconGlyph: "PeerIconGlyph" }));
jest.mock("../../components/DialogSurface", () => ({ DialogSurface: "DialogSurface" }));

const desktops: PairedDesktop[] = [
  { desktopId: "desk_a", pairId: "pair_a", name: "Lab MacBook Pro", icon: "laptop" },
  { desktopId: "desk_b", pairId: "pair_b", name: "Office host", icon: "desktop" },
] as PairedDesktop[];

const ready = connectionPresentation({ phase: "ready", desktopOnline: true });

function render(
  machines: MachineSummary[],
  onTakeControl?: (id: string) => Promise<void>,
  extras: { onReconnect?: () => void; onUnpair?: () => void } = {},
) {
  let tree!: ReactTestRenderer;
  act(() => {
    tree = create(createElement(DesktopPicker, {
      filter: { kind: "all" },
      onChange: () => undefined,
      onManage: () => undefined,
      desktops,
      machines,
      connection: { presentation: ready, ...extras },
      ...(onTakeControl ? { onTakeControl } : {}),
    }));
  });
  return tree;
}

const trigger = (tree: ReactTestRenderer) => tree.root.findAll(node =>
  node.props.accessibilityRole === "button"
  && String(node.props.accessibilityLabel ?? "").startsWith("Desktops:"))[0]!;

const openPicker = (tree: ReactTestRenderer) => {
  act(() => trigger(tree).props.onPress());
};

const painted = (tree: ReactTestRenderer) =>
  tree.root.findAllByType(Text).map(node => node.props.children);

const dotColor = (tree: ReactTestRenderer) => {
  const dot = tree.root.findAll(node => node.props.testID === "connection-dot")[0]!;
  return StyleSheet.flatten(dot.props.style).backgroundColor;
};

const machineDotColor = (tree: ReactTestRenderer, desktopId: string) => {
  const dot = tree.root.findAll(node => node.props.testID === `machine-dot-${desktopId}`)[0]!;
  return StyleSheet.flatten(dot.props.style).backgroundColor;
};

describe("the picker's trigger", () => {
  it("keeps the machine's whole name on screen at the largest text size", () => {
    const tree = render([]);
    const label = trigger(tree).findAllByType(Text).find(node => node.props.children === "All desktops")!;
    // Two lines rather than one: this row shares its width with the list's own
    // tools and settings, and a name the reader chose can be longer than what is
    // left. Wrapping shows all of it; one line with an ellipsis showed only its
    // beginning.
    expect(label.props.numberOfLines).toBe(2);
    expect(StyleSheet.flatten(label.props.style)).toMatchObject({ flex: 1, minWidth: 0 });
    // The trigger itself stays a full touch target after losing width.
    const style = StyleSheet.flatten(trigger(tree).props.style({ pressed: false }));
    expect(style).toMatchObject({ minHeight: 44, flex: 1 });
    expect(style.flexDirection).toBe("row");
  });

  it("is the status dot: one control, one tap, and the state in its own name", () => {
    const tree = render([
      { desktopId: "desk_a", label: "Lab MacBook Pro", active: true, status: "connected", control: "active" },
    ]);
    // The dot lives on the trigger now. Tapping any part of it opens the one
    // sheet — there is no second control beside it to choose between.
    expect(dotColor(tree)).toBe(lightColors.success);
    // A screen reader cannot read a colour, so the state is part of the name.
    expect(trigger(tree).props.accessibilityLabel).toBe("Desktops: All desktops, Connected");
    expect(trigger(tree).props.accessibilityHint).toBe("Show connection details and available actions");
    expect(trigger(tree).props.accessibilityState.expanded).toBe(false);
    openPicker(tree);
    expect(trigger(tree).props.accessibilityState.expanded).toBe(true);
    expect(painted(tree)).toContain("Connected");
  });

  it("is yellow while one machine is down and red when none answers", () => {
    const down = render([
      { desktopId: "desk_a", label: "Lab MacBook Pro", active: true, status: "connected", control: "active" },
      { desktopId: "desk_b", label: "Office host", active: false, status: "offline" },
    ]);
    expect(dotColor(down)).toBe(lightColors.warningDot);
    const gone = render([
      { desktopId: "desk_a", label: "Lab MacBook Pro", active: true, status: "offline" },
      { desktopId: "desk_b", label: "Office host", active: false, status: "offline" },
    ]);
    expect(dotColor(gone)).toBe(lightColors.danger);
  });
});

describe("the device sheet", () => {
  it("explains the connection the dot was painting, and reconnects from there", () => {
    const onReconnect = jest.fn();
    const tree = render([], undefined, { onReconnect });
    openPicker(tree);

    expect(painted(tree)).toContain("Connected");
    expect(painted(tree)).toContain("The phone is connected to the desktop");
    const action = tree.root.findAll(node =>
      node.props.accessibilityRole === "button" && node.props.accessibilityLabel === "Reconnect")[0]!;
    act(() => action.props.onPress());
    expect(onReconnect).toHaveBeenCalledTimes(1);
  });

  it("reaches every machine at once when some of them are down", () => {
    const onReconnect = jest.fn();
    const tree = render([
      { desktopId: "desk_a", label: "Lab MacBook Pro", active: true, status: "connected", control: "active" },
      { desktopId: "desk_b", label: "Office host", active: false, status: "offline" },
    ], undefined, { onReconnect });
    openPicker(tree);

    expect(painted(tree)).toContain("Some desktops are not connected");
    const action = tree.root.findAll(node =>
      node.props.accessibilityRole === "button" && node.props.accessibilityLabel === "Reconnect all")[0]!;
    act(() => action.props.onPress());
    expect(onReconnect).toHaveBeenCalledTimes(1);
  });

  it("gives each machine its own dot and state, in the badge's palette", () => {
    const tree = render([
      { desktopId: "desk_a", label: "Lab MacBook Pro", active: true, status: "connected", control: "active" },
      { desktopId: "desk_b", label: "Office host", active: false, status: "offline" },
    ]);
    openPicker(tree);

    expect(machineDotColor(tree, "desk_a")).toBe(lightColors.success);
    expect(machineDotColor(tree, "desk_b")).toBe(lightColors.danger);
    expect(painted(tree)).toContain("Offline");
    // The merged row keeps its own meaning, and stays selectable: an offline
    // machine is not a removed one.
    expect(tree.root.findAll(node =>
      node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function")).toHaveLength(3);
  });

  it("says which machine is in standby and offers to take it over there", async () => {
    const take = jest.fn().mockResolvedValue(undefined);
    const tree = render([
      { desktopId: "desk_a", label: "Lab MacBook Pro", active: true, status: "connected", control: "standby" },
      { desktopId: "desk_b", label: "Office host", active: false, status: "connected", control: "active" },
    ], take);
    openPicker(tree);

    // Under the name: "pick this machine" and "take it over" are different
    // answers and must not read as the same kind of row.
    expect(painted(tree)).toContain("Connected · in standby");
    // A held machine answers every heartbeat and still cannot be used: its dot
    // is the yellow of "connected, but not yours", not the green of "usable".
    expect(machineDotColor(tree, "desk_a")).toBe(lightColors.warningDot);
    const button = tree.root.findAll(node =>
      node.props.accessibilityRole === "button"
      && String(node.props.accessibilityLabel ?? "").startsWith("Take over"))[0]!;
    expect(button).toBeDefined();

    await act(async () => {
      button.props.onPress();
    });
    expect(take).toHaveBeenCalledWith("desk_a");
  });

  it("offers nothing to take over on a machine this phone already drives", () => {
    const tree = render([
      { desktopId: "desk_a", label: "Lab MacBook Pro", active: true, status: "connected", control: "active" },
      { desktopId: "desk_b", label: "Office host", active: false, status: "connected", control: "active" },
    ], jest.fn());
    openPicker(tree);

    const buttons = tree.root.findAll(node =>
      node.props.accessibilityRole === "button"
      && String(node.props.accessibilityLabel ?? "").startsWith("Take over"));
    expect(buttons).toHaveLength(0);
    expect(painted(tree)).not.toContain("Connected · in standby");
  });

  it("marks an offline machine without pretending it can be driven", () => {
    const tree = render([
      { desktopId: "desk_a", label: "Lab MacBook Pro", active: true, status: "connected", control: "active" },
      { desktopId: "desk_b", label: "Office host", active: false, status: "offline" },
    ], jest.fn());
    openPicker(tree);

    expect(painted(tree)).toContain("Offline");
    const buttons = tree.root.findAll(node =>
      node.props.accessibilityRole === "button"
      && String(node.props.accessibilityLabel ?? "").startsWith("Take over"));
    expect(buttons).toHaveLength(0);
  });

  it("picking a machine still commits the choice and closes the sheet", () => {
    const onChange = jest.fn();
    let tree!: ReactTestRenderer;
    act(() => {
      tree = create(createElement(DesktopPicker, {
        filter: { kind: "all" },
        onChange,
        onManage: () => undefined,
        desktops,
        machines: [],
        connection: { presentation: ready },
      }));
    });
    openPicker(tree);
    const row = tree.root.findAll(node =>
      node.props.accessibilityRole === "radio"
      && typeof node.props.onPress === "function"
      && node.props.accessibilityState?.checked === false)[0]!;
    act(() => row.props.onPress());
    expect(onChange).toHaveBeenCalledWith({ kind: "desktop", desktopId: "desk_a" });
    expect(trigger(tree).props.accessibilityState.expanded).toBe(false);
  });
});
