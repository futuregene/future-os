import { createElement, type ComponentProps } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, StyleSheet, View } from "react-native";
import { SessionUsageSheet } from "../components/SessionUsageSheet";

jest.mock("lucide-react-native", () => ({ X: "X" }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));

const usage = {
  inputTokens: 1_000_000,
  outputTokens: 100_000,
  cacheReadTokens: 200_000,
  cacheWriteTokens: 50_000,
  costCny: 1.2345,
  costInputCny: 0.75,
  costOutputCny: 0.4,
  costCacheReadCny: 0.08,
  costCacheWriteCny: 0.0045,
};
const props = {
  title: "Priced conversation",
  usage,
  visible: true,
  onClose: jest.fn(),
  t: ((key: string) => key) as unknown as ComponentProps<typeof SessionUsageSheet>["t"],
};
let tree: ReactTestRenderer;
beforeEach(() => {
  jest.clearAllMocks();
  act(() => { tree = create(createElement(SessionUsageSheet, props)); });
});
afterEach(() => act(() => tree.unmount()));
const press = (label: string) =>
  tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!;
/** Every string rendered by the sheet, in tree order. */
const texts = () => tree.root.findAll(node => typeof node.children[0] === "string").map(node => node.children[0] as string);
// The sheet formats token counts in the runtime's default locale, so assert
// against the same formatter rather than hard-coding a grouping separator.
const count = (value: number) => new Intl.NumberFormat(undefined).format(value);

test("splits the tokens by category and names the conversation it accounts for", () => {
  const rendered = texts();
  // The non-cached input remainder is what the input row bills.
  expect(rendered).toContain(count(750_000));
  expect(rendered).toContain("¥0.75");
  expect(rendered).toContain(count(100_000));
  expect(rendered).toContain("¥0.4");
  expect(rendered).toContain("¥0.08");
  expect(rendered).toContain("¥0.0045");
  expect(rendered).toContain("¥1.2345");
  expect(rendered).toContain("chat.usageHintPriced");
  expect(rendered).toContain("Priced conversation");

  // Only the two accounting actions: closing the sheet, and nothing that
  // edits the conversation (renaming stays in the top bar).
  const actions = tree.root.findAll(node => typeof node.props.onPress === "function" && node.props.accessibilityRole === "button");
  expect(actions.map(node => node.props.accessibilityLabel)).toEqual(["common.close"]);
  act(() => press("common.close").props.onPress());
  expect(props.onClose).toHaveBeenCalledTimes(1);
});

test("shows tokens only for a model with no prices, and still shows what was billed", () => {
  act(() => {
    tree.update(createElement(SessionUsageSheet, {
      ...props,
      usage: { ...usage, costInputCny: 0, costOutputCny: 0, costCacheReadCny: 0, costCacheWriteCny: 0 },
    }));
  });
  const rendered = texts();
  // One dash per priced column, never a fabricated ¥0 row.
  expect(rendered.filter(value => value === "—")).toHaveLength(4);
  expect(rendered).not.toContain("¥0.75");
  expect(rendered).toContain("¥1.2345");
  expect(rendered).toContain("chat.usageHintUnpriced");
});

test("reports missing usage instead of an empty table", () => {
  act(() => { tree.update(createElement(SessionUsageSheet, { ...props, usage: null })); });
  const rendered = texts();
  expect(rendered).toContain("chat.usageUnavailable");
  expect(rendered).not.toContain("chat.usageTotal");
});

test("stays a bounded, dismissible modal sheet", () => {
  expect(tree.root.findByType(Modal).props.animationType).toBe("slide");
  const surface = tree.root.findAllByType(View).find(node => node.props.accessibilityViewIsModal)!;
  expect(StyleSheet.flatten(surface.props.style)).toMatchObject({ maxWidth: 560, maxHeight: "85%" });
  act(() => tree.root.findByType(Modal).props.onRequestClose());
  expect(props.onClose).toHaveBeenCalledTimes(1);
});
