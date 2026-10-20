import { createElement } from "react";
import { Text } from "react-native";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Button } from "../../components/Button";
import i18n from "../../i18n";
import { StandbyScreen } from "../StandbyScreen";

jest.mock("lucide-react-native", () => ({ MonitorOff: () => null }));

let tree: ReactTestRenderer;
afterEach(() => { if (tree) act(() => tree.unmount()); });

function render(props: Partial<Parameters<typeof StandbyScreen>[0]> = {}) {
  act(() => {
    tree = create(createElement(StandbyScreen, {
      onTakeControl: jest.fn(async () => {}),
      ...props,
    }));
  });
}

/** Every string the page paints, joined per text node. */
function painted() {
  return tree.root.findAllByType(Text)
    .map(node => node.props.children)
    .flat()
    .filter((part): part is string => typeof part === "string");
}

test("a held desktop names who has it and offers the takeover", async () => {
  const onTakeControl = jest.fn(async () => {});
  render({ holder: { name: "Lab MacBook Air" }, onTakeControl });
  expect(painted()).toContain(i18n.t("connection.controlHeldBy", { name: "Lab MacBook Air" }));
  expect(painted()).toContain(i18n.t("connection.standbyPageHint"));
  // The page says what the action costs before it is taken.
  expect(painted()).toContain(i18n.t("connection.takeControlHint", { name: "Lab MacBook Air" }));
  const button = tree.root.findByType(Button);
  expect(button.props.label).toBe(i18n.t("connection.takeControl"));
  await act(async () => { button.props.onPress(); });
  expect(onTakeControl).toHaveBeenCalledTimes(1);
});

test("a holder that declared no name is named neutrally rather than invented", () => {
  render({ holder: null });
  expect(painted()).toContain(i18n.t("connection.controlHeldByUnknown"));
  expect(painted()).toContain(i18n.t("connection.takeControlHintUnknown"));
});

test("a refused takeover says so and leaves the button ready for another try", async () => {
  const onTakeControl = jest.fn()
    .mockRejectedValueOnce(new Error("takeover_failed"))
    .mockResolvedValueOnce(undefined);
  render({ holder: { name: "Office Mac" }, onTakeControl });

  await act(async () => { tree.root.findByType(Button).props.onPress(); });
  expect(painted()).toContain(i18n.t("connection.takeControlFailed"));
  const button = tree.root.findByType(Button);
  expect(button.props.loading).toBe(false);

  await act(async () => { button.props.onPress(); });
  expect(onTakeControl).toHaveBeenCalledTimes(2);
  expect(painted()).not.toContain(i18n.t("connection.takeControlFailed"));
});

test("a takeover in flight cannot be asked for twice", async () => {
  let release: () => void = () => {};
  const onTakeControl = jest.fn(() => new Promise<void>((resolve) => { release = resolve; }));
  render({ onTakeControl });

  act(() => { tree.root.findByType(Button).props.onPress(); });
  expect(tree.root.findByType(Button).props.loading).toBe(true);
  act(() => tree.root.findByType(Button).props.onPress());
  expect(onTakeControl).toHaveBeenCalledTimes(1);

  await act(async () => {
    release();
    await Promise.resolve();
  });
  expect(tree.root.findByType(Button).props.loading).toBe(false);
});
