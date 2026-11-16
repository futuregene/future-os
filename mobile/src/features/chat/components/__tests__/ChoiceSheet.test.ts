import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { StyleSheet } from "react-native";
import { choiceAnswerFromCard, pickChoiceOption, type ChoiceAnswer } from "../../choiceAnswer";
import { ChoiceSheet } from "../ChoiceSheet";

jest.mock("lucide-react-native", () => ({ Pencil: () => null, X: () => null }));
jest.mock("react-native-safe-area-context", () =>
  jest.requireActual("react-native-safe-area-context/jest/mock").default);

const ANSWER: ChoiceAnswer = pickChoiceOption(choiceAnswerFromCard({
  mode: "single",
  options: [
    { description: "按 Dreher 2025 的样本量重算置信区间", id: "forest", label: "重画森林图" },
    { id: "flow", label: "补一张实验流程图" },
  ],
  title: "接下来做哪一件？",
  type: "choiceCard",
}), "forest");

describe("the phone's change-answer sheet", () => {
  let renderer: ReactTestRenderer;
  afterEach(() => { act(() => renderer?.unmount()); });

  function render(props: Partial<Parameters<typeof ChoiceSheet>[0]> = {}) {
    act(() => {
      renderer = create(createElement(ChoiceSheet, {
        answer: ANSWER, onClose: jest.fn(), onPick: jest.fn(), open: true,
        t: ((key: string) => key) as never, ...props,
      }));
    });
    return renderer.root;
  }

  /** A rendered `Pressable` repeats its props on the views inside it. */
  function rows(root: ReactTestRenderer["root"]) {
    const seen = new Set<string>();
    return root.findAll(node => {
      const testID: unknown = node.props.testID;
      if (typeof testID !== "string" || !testID.startsWith("choice-sheet-option")) return false;
      if (node.props.accessibilityRole === undefined || seen.has(testID)) return false;
      seen.add(testID);
      return true;
    });
  }

  test("offers the same 44pt rows as the card, with the current pick marked", () => {
    const root = render();
    expect(root.findAllByProps({ testID: "choice-sheet" }).length).toBeGreaterThan(0);
    const options = rows(root);
    expect(options).toHaveLength(2);
    expect(options[0]!.props.testID).toBe("choice-sheet-option-forest");
    expect(options[0]!.props["aria-checked"]).toBe(true);
    expect(options[1]!.props["aria-checked"]).toBe(false);
    for (const row of options) {
      const style = row.props.style;
      expect(StyleSheet.flatten(typeof style === "function" ? style({ pressed: false }) : style).minHeight)
        .toBe(44);
    }
  });

  test("a pick is reported with the option's id, and closing is not the pick's business", () => {
    const onPick = jest.fn();
    const onClose = jest.fn();
    const root = render({ onPick, onClose });
    const options = rows(root);
    act(() => options[1]!.props.onPress());
    expect(onPick).toHaveBeenCalledWith("flow");
    expect(onClose).not.toHaveBeenCalled();
  });

  test("nothing is drawn while it is closed or when there is no answer", () => {
    expect(rows(render({ open: false }))).toHaveLength(0);
    act(() => renderer.unmount());
    expect(rows(render({ answer: null }))).toHaveLength(0);
  });
});
