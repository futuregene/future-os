import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { StyleSheet } from "react-native";
import { MarkdownText } from "../MarkdownText";
import { ChoiceCardAnswerProvider, type ChoiceCardAnswer } from "../ChoiceCard";

// The icon set ships as an ES module, and these suites load the real
// `MarkdownText` (which a choice card pulls it into). Node 24+ can require it
// directly; older runtimes cannot, so the shapes are stubbed here.
jest.mock("lucide-react-native", () => ({ Check: () => null, ListChecks: () => null }));

jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "zh-CN" } }),
}));

const CARD = [
  "```futureos-choice",
  "title: 接下来做哪一件？",
  "mode: single",
  "option: forest | 重画森林图 | 按 Dreher 2025 的样本量重算置信区间",
  "option: flow | 补一张实验流程图",
  "```",
].join("\n");

/** A second question, with its own ids: one thread can carry several cards. */
const SECOND_CARD = [
  "```futureos-choice",
  "title: 要不要先喘口气？",
  "mode: single",
  "option: later | 稍后继续",
  "option: now | 现在继续",
  "```",
].join("\n");

describe("a futureos-choice card on the phone", () => {
  let renderer: ReactTestRenderer;
  afterEach(() => { act(() => renderer?.unmount()); });

  function render(text: string, answer?: ChoiceCardAnswer) {
    act(() => {
      renderer = create(
        createElement(ChoiceCardAnswerProvider, { value: answer }, createElement(MarkdownText, { text })),
      );
    });
    return renderer.root;
  }

  /**
   * The option rows, one per option, in card order.
   *
   * A rendered `Pressable` carries its props on a wrapper and on the views inside
   * it, so the first instance of each testID is the row itself.
   */
  function rows(root: ReactTestRenderer["root"]) {
    const seen = new Set<string>();
    return root.findAll(node => {
      const testID: unknown = node.props.testID;
      if (typeof testID !== "string" || !testID.startsWith("choice-option")) return false;
      if (node.props.accessibilityRole === undefined || seen.has(testID)) return false;
      seen.add(testID);
      return true;
    });
  }
  /**
   * The card's subtitle text(s). `t()` is stubbed to echo its key, so a card that
   * prints an answering hint reads `choice.hint.single` / `choice.hint.multi` and
   * a card that cannot answer reads `choice.hint.readonly`.
   *
   * Filtered to host nodes: a rendered `<Text>` also shows up as its composite
   * element with the same children, which would double every entry.
   */
  function hintKeys(root: ReactTestRenderer["root"]) {
    return root.findAll(node => typeof node.type === "string"
      && typeof node.props.children === "string"
      && /^choice\.hint\./.test(node.props.children))
      .map(node => node.props.children as string);
  }
  /** The style a row is actually drawn with (Pressable takes a function). */
  function drawnStyle(row: { props: { style?: unknown; }; }) {
    const style = row.props.style;
    return StyleSheet.flatten(typeof style === "function" ? style({ pressed: false }) : style);
  }

  test("draws the card, not the degraded list: a radio per option, 44pt tall", () => {
    const root = render(`先做这一步。\n\n${CARD}`);
    expect(root.findAllByProps({ testID: "choice-card" }).length).toBeGreaterThan(0);
    // The fence's own syntax never reaches the reader (§8's degraded path is for
    // clients that do not know the node, and this one does).
    expect(JSON.stringify(renderer.toJSON())).not.toContain("option:");
    const options = rows(root);
    expect(options).toHaveLength(2);
    for (const row of options) {
      expect(row.props.accessibilityRole).toBe("radio");
      // §5.7: a row is at least a 44pt target.
      expect(StyleSheet.flatten(drawnStyle(row)).minHeight).toBe(44);
    }
    const texts = root.findAll(node => typeof node.props.children === "string")
      .map(node => node.props.children);
    expect(texts).toContain("重画森林图");
    expect(texts).toContain("补一张实验流程图");
  });

  test("answers the last card only: the thread's earlier card is read-only", () => {
    const onPick = jest.fn();
    const answer: ChoiceCardAnswer = { onPick, selectedIds: [], title: "接下来做哪一件？" };
    // Two cards in one body: the first question is history, the second is live.
    const root = render(`${CARD}\n\n还有一件事：\n\n${SECOND_CARD}`, answer);
    const all = rows(root);
    expect(all).toHaveLength(4);
    expect(all[0]!.props.testID).toBe("choice-option-forest");
    expect(all[2]!.props.testID).toBe("choice-option-later");
    for (const historical of [all[0]!, all[1]!]) {
      expect(historical.props.disabled).toBe(true);
      expect(historical.props.onPress).toBeUndefined();
    }
    const lastRow = all[3]!;
    expect(lastRow.props["aria-checked"]).toBe(false);
    expect(lastRow.props.disabled).toBe(false);
    act(() => lastRow.props.onPress());
    expect(onPick).toHaveBeenCalledTimes(1);
    // The card travels with the pick, so the draft can carry the whole question.
    expect(onPick.mock.calls[0]![0]).toMatchObject({ type: "choiceCard", title: "要不要先喘口气？" });
    expect(onPick.mock.calls[0]![1]).toBe("now");
  });

  /**
   * The subtitle is the one place a card promises an action, so the earlier card
   * must not print the answering one: a card whose rows cannot be tapped must not
   * say "pick one and it lands in the input". Two cards in one body, one
   * answering hint, one that says it cannot be picked.
   */
  test("the earlier card says it cannot be picked instead of promising a pick", () => {
    const answer: ChoiceCardAnswer = { onPick: jest.fn(), selectedIds: [], title: "接下来做哪一件？" };
    const root = render(`${CARD}\n\n还有一件事：\n\n${SECOND_CARD}`, answer);
    expect(hintKeys(root)).toEqual(["choice.hint.readonly", "choice.hint.single"]);
  });

  test("a card read-only for lack of an answer surface says so too", () => {
    const root = render(CARD);
    expect(hintKeys(root)).toEqual(["choice.hint.readonly"]);
  });

  test("without an answer surface every card is read-only", () => {
    const root = render(CARD);
    const options = rows(root);
    expect(options).toHaveLength(2);
    for (const row of options) {
      expect(row.props.disabled).toBe(true);
      expect(row.props.onPress).toBeUndefined();
    }
  });

  test("the draft's pick marks the row it belongs to and adds the footer", () => {
    const answer: ChoiceCardAnswer = {
      onPick: jest.fn(),
      selectedIds: ["forest"],
      title: "接下来做哪一件？",
    };
    const root = render(CARD, answer);
    const [first, second] = rows(root);
    expect(first!.props["aria-checked"]).toBe(true);
    expect(second!.props["aria-checked"]).toBe(false);
    expect(root.findAllByProps({ testID: "choice-card-footer" }).length).toBeGreaterThan(0);
  });

  test("an answer to another question does not light this card up", () => {
    const answer: ChoiceCardAnswer = {
      onPick: jest.fn(),
      selectedIds: ["forest"],
      title: "另一个问题？",
    };
    const root = render(CARD, answer);
    for (const row of rows(root)) expect(row.props["aria-checked"]).toBe(false);
    expect(root.findAllByProps({ testID: "choice-card-footer" })).toHaveLength(0);
  });

  test("a multi-choice card offers checkboxes and keeps several picks marked", () => {
    const text = CARD.replace("mode: single", "mode: multi");
    const answer: ChoiceCardAnswer = {
      onPick: jest.fn(),
      selectedIds: ["forest", "flow"],
      title: "接下来做哪一件？",
    };
    const root = render(text, answer);
    const options = rows(root);
    for (const row of options) {
      expect(row.props.accessibilityRole).toBe("checkbox");
      expect(row.props["aria-checked"]).toBe(true);
    }
  });
});
