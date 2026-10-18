import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text } from "react-native";
import { DesktopPicker } from "../DesktopPicker";
import type { MachineSummary } from "../../remote/desktopStatus";
import type { PairedDesktop } from "../../remote/types";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => ({
      "desktops.pickerLabel": "Desktops",
      "desktops.filterAll": "All desktops",
      "desktops.manage": "Manage paired desktops…",
      "common.close": "Close",
      "connection.takeControl": "Take over",
      "connection.machineStandbyHint": "Another device is in control",
      "connection.machine.offline": "Offline",
    } as Record<string, string>)[key] ?? key,
  }),
}));
jest.mock("lucide-react-native", () => ({
  Check: "Check", ChevronDown: "ChevronDown", Layers: "Layers", Monitor: "Monitor", X: "X",
}));
jest.mock("../../remote/peerIcons", () => ({ iconGlyph: () => "" }));
jest.mock("../../components/DialogSurface", () => ({ DialogSurface: "DialogSurface" }));

const desktops: PairedDesktop[] = [
  { desktopId: "desk_a", pairId: "pair_a", name: "Lab MacBook Pro", icon: "laptop" },
  { desktopId: "desk_b", pairId: "pair_b", name: "Office host", icon: "desktop" },
] as PairedDesktop[];

function render(machines: MachineSummary[], onTakeControl?: (id: string) => Promise<void>) {
  let tree!: ReactTestRenderer;
  act(() => {
    tree = create(createElement(DesktopPicker, {
      filter: { kind: "all" },
      onChange: () => undefined,
      onManage: () => undefined,
      desktops,
      machines,
      ...(onTakeControl ? { onTakeControl } : {}),
    }));
  });
  return tree;
}

const openPicker = (tree: ReactTestRenderer) => {
  const trigger = tree.root.findAll(node => node.props.accessibilityRole === "button"
    && node.props.accessibilityLabel === "Desktops")[0]!;
  act(() => trigger.props.onPress());
};

const painted = (tree: ReactTestRenderer) =>
  tree.root.findAllByType(Text).map(node => node.props.children);

describe("the desktop picker", () => {
  it("says which machine is in standby and offers to take it over there", async () => {
    const take = jest.fn().mockResolvedValue(undefined);
    const tree = render([
      { desktopId: "desk_a", label: "Lab MacBook Pro", active: true, status: "connected", control: "standby" },
      { desktopId: "desk_b", label: "Office host", active: false, status: "connected", control: "active" },
    ], take);
    openPicker(tree);

    // Under the name: "pick this machine" and "take it over" are different
    // answers and must not read as the same kind of row.
    expect(painted(tree)).toContain("Another device is in control");
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
    expect(painted(tree)).not.toContain("Another device is in control");
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
});
