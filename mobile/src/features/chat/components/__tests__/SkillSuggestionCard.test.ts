import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { SkillSuggestionCard } from "../SkillSuggestionCard";

jest.mock("lucide-react-native", () => ({ Lightbulb: () => null, X: () => null }));
const t = ((key: string) => key) as never;

const suggestion = {
  draft: "please search the web for this",
  skill: { name: "future-web", description: "search the open web" },
};

let tree: ReactTestRenderer;
afterEach(() => { if (tree) act(() => tree.unmount()); });

function render(props: Partial<Parameters<typeof SkillSuggestionCard>[0]> = {}) {
  act(() => {
    tree = create(createElement(SkillSuggestionCard, {
      installing: false, onDismiss: jest.fn(), onInstall: jest.fn(), suggestion, t, ...props,
    }));
  });
}
function hasText(text: string) {
  // A template with an interpolation (`/{name}`) reaches the host as an array
  // of strings, so children are flattened before comparing.
  return painted().includes(text);
}
/** Every string a host text node paints, in render order. */
function painted() {
  return tree.root
    .findAll(node => typeof node.type === "string" && node.props.children !== undefined)
    .map(node => Array.isArray(node.props.children)
      ? node.props.children.filter((part: unknown) => typeof part === "string").join("")
      : String(node.props.children));
}
/** The two decision buttons, as the touch layer sees them (host nodes only). */
function decisions() {
  return tree.root.findAll(node =>
    typeof node.type === "string" && node.props.accessibilityRole === "button");
}
/** The pressable with this screen-reader label. */
function button(text: string) {
  return tree.root.findAll(node =>
    typeof node.props.onPress === "function"
    && node.findAll(inner => inner.props.children === text).length > 0).at(-1)!;
}

test("the card names the skill and its description, and offers both decisions", () => {
  const onInstall = jest.fn();
  const onDismiss = jest.fn();
  render({ onDismiss, onInstall });
  expect(hasText("chat.skillRecommend")).toBe(true);
  // The slash form is what the user would type, so the card shows it verbatim.
  expect(hasText("/future-web")).toBe(true);
  expect(hasText("search the open web")).toBe(true);
  expect(hasText("chat.skillInstallAndUse")).toBe(true);
  act(() => button("chat.skillInstallAndUse").props.onPress());
  expect(onInstall).toHaveBeenCalledTimes(1);
  // Dismissing means "send what I typed", never "install".
  act(() => button("chat.skillDismissAndSend").props.onPress());
  expect(onDismiss).toHaveBeenCalledTimes(1);
  expect(onInstall).toHaveBeenCalledTimes(1);
});

test("a skill with no description still renders one line of decisions", () => {
  render({ suggestion: { draft: "x", skill: { name: "future-paper", description: "" } } });
  expect(hasText("/future-paper")).toBe(true);
  expect(tree.root.findAll(node => node.props.numberOfLines === 3)).toHaveLength(0);
});

test("while the install is in flight both decisions are locked and the wait is visible", () => {
  const onInstall = jest.fn();
  const onDismiss = jest.fn();
  render({ installing: true, onDismiss, onInstall });
  const buttons = decisions();
  expect(buttons).toHaveLength(2);
  for (const node of buttons) {
    expect(node.props.accessibilityState?.disabled).toBe(true);
  }
  // The install button also reports its busy state to the screen reader.
  expect(buttons.some(node => node.props.accessibilityState?.busy === true)).toBe(true);
  // The spinner replaces the label so the card cannot be mistaken for idle.
  expect(hasText("chat.skillInstallAndUse")).toBe(false);
});
