import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text } from "react-native";
import { Button } from "../../../components/Button";
import { TimeField } from "../TimeField";

jest.mock("lucide-react-native", () => ({ ChevronDown: "ChevronDown" }));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) => (options ? `${key}:${JSON.stringify(options)}` : key),
  }),
}));

let tree: ReactTestRenderer;
let picked: string[];
/** The draft's time, as the form would hold it: `onChange` feeds the value back. */
let current: string;

function field(disabled = false) {
  return createElement(TimeField, {
    label: "tasks.form.time",
    value: current,
    disabled,
    onChange: (next: string) => {
      picked.push(next);
      current = next;
      // The form is controlled: the parent re-renders the field with the value
      // it just wrote, which is what keeps the two halves one value. This runs
      // inside the press's own `act`, so it must not open another one.
      tree.update(field(disabled));
    },
  });
}

async function mount(value: string, disabled = false) {
  picked = [];
  current = value;
  await act(async () => { tree = create(field(disabled)); });
}

const trigger = () => tree.root
  .findAll(node => node.props.accessibilityRole === "button" && node.props.accessibilityLabel === "tasks.form.time")
  .find(node => typeof node.props.onPress === "function")!;
const radio = (label: string) => tree.root
  .findAll(node => node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function")
  .find(node => node.props.accessibilityLabel === label);
const hour = (value: string) => radio(`tasks.form.hourOption:{"hour":"${value}"}`);
const minute = (value: string) => radio(`tasks.form.minuteOption:{"minute":"${value}"}`);
const texts = () => tree.root.findAllByType(Text).map(node => node.props.children).flat();

// The picker is closed until asked for: the form shows the stored time and
// nothing else, so six rows of chips do not push its own fields off the screen.
test("shows the stored time and stays closed until asked", async () => {
  await mount("18:30");
  expect(trigger().props.accessibilityValue).toEqual({ text: "18:30" });
  expect(trigger().props.accessibilityState.expanded).toBe(false);
  expect(hour("18")).toBeUndefined();
});

test("opens on the stored hour and minute", async () => {
  await mount("18:30");
  await act(async () => trigger().props.onPress());
  expect(trigger().props.accessibilityState.expanded).toBe(true);
  expect(hour("18")!.props.accessibilityState.selected).toBe(true);
  expect(minute("30")!.props.accessibilityState.selected).toBe(true);
  expect(hour("09")!.props.accessibilityState.selected).toBe(false);
});

// Choosing an hour keeps the minute and the other way round: the two strips are
// one value, and a pick of one half must not silently reset the other.
test("moves one half of the value at a time", async () => {
  await mount("18:30");
  await act(async () => trigger().props.onPress());
  await act(async () => hour("07")!.props.onPress());
  expect(picked).toEqual(["07:30"]);
  await act(async () => minute("05")!.props.onPress());
  expect(picked).toEqual(["07:30", "07:05"]);
  expect(texts()).toContain("07:05");
});

// Every value is padded to two digits, which is what the desktop stores and what
// its own time input emits.
test("pads both halves", async () => {
  await mount("00:00");
  await act(async () => trigger().props.onPress());
  await act(async () => minute("01")!.props.onPress());
  expect(picked).toEqual(["00:01"]);
  await act(async () => hour("23")!.props.onPress());
  expect(picked).toEqual(["00:01", "23:01"]);
});

// A task written by another client (or a truncated payload) can carry no time;
// the picker opens at a usable hour rather than an empty strip, and writes
// nothing until the user picks.
test("opens at 09:00 when the stored time is unreadable", async () => {
  await mount("");
  expect(trigger().props.accessibilityValue).toEqual({ text: "09:00" });
  await act(async () => trigger().props.onPress());
  expect(hour("09")!.props.accessibilityState.selected).toBe(true);
  expect(picked).toEqual([]);
});

test("closes from its own button, keeping the value it wrote", async () => {
  await mount("12:00");
  await act(async () => trigger().props.onPress());
  await act(async () => minute("45")!.props.onPress());
  const done = tree.root.findAllByType(Button).find(node => node.props.label === "tasks.form.done")!;
  await act(async () => done.props.onPress());
  expect(trigger().props.accessibilityState.expanded).toBe(false);
  expect(picked).toEqual(["12:45"]);
});

// A disabled form (a write in flight) must not take a pick it cannot send.
test("refuses every control while the form is busy", async () => {
  await mount("12:00", true);
  expect(trigger().props.accessibilityState.disabled).toBe(true);
  expect(trigger().props.disabled).toBe(true);
  expect(hour("09")).toBeUndefined();
});
