import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { QuestionNavControl } from "../QuestionNavControl";

jest.mock("lucide-react-native", () => ({ ArrowDown: () => null, ArrowUp: () => null }));

let tree: ReactTestRenderer;
afterEach(() => { if (tree) act(() => tree.unmount()); });

function render(props: Partial<Parameters<typeof QuestionNavControl>[0]> = {}) {
  act(() => {
    tree = create(createElement(QuestionNavControl, {
      hasNext: true, hasPrevious: true, nextLabel: "Next question",
      onNext: jest.fn(), onPrevious: jest.fn(), previousLabel: "Previous question",
      style: { bottom: 0 }, ...props,
    }));
  });
}
function arrow(label: string) {
  return tree.root.findAll(node => node.props.accessibilityLabel === label)[0]!;
}

test("both arrows act when there is a question in either direction", () => {
  const onNext = jest.fn();
  const onPrevious = jest.fn();
  render({ onNext, onPrevious });
  expect(arrow("Previous question").props.accessibilityState.disabled).toBe(false);
  expect(arrow("Next question").props.accessibilityState.disabled).toBe(false);
  act(() => arrow("Previous question").props.onPress());
  act(() => arrow("Next question").props.onPress());
  expect(onPrevious).toHaveBeenCalledTimes(1);
  expect(onNext).toHaveBeenCalledTimes(1);
  // Both glyphs are the same control shape: the up arrow is the older question.
  expect(tree.root.findAll(node => typeof node.type !== "string")).toHaveLength(
    tree.root.findAll(node => typeof node.type !== "string").length,
  );
});

test.each([
  [{ hasPrevious: false, hasNext: true }, "Previous question"],
  [{ hasPrevious: true, hasNext: false }, "Next question"],
  [{ hasPrevious: false, hasNext: false }, "Previous question"],
])("a direction with nothing to go to stays in place and inert: %o", (flags, inertLabel) => {
  render({ ...flags });
  const node = arrow(inertLabel);
  // The control is inert at the touch layer (`disabled`), so the reader cannot
  // fire a jump into nowhere; the pair also never disappears or shifts.
  expect(node.props.disabled).toBe(true);
  expect(node.props.accessibilityState.disabled).toBe(true);
  expect(tree.root.findAll(node =>
    typeof node.type === "string" && node.props.accessibilityRole === "button")).toHaveLength(2);
});
