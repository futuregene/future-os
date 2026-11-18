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
   * The card's subtitle text(s). `t()` is stubbed to echo its key, so the only
   * key that can appear is `choice.hint.readonly`; the default single/multi
   * instructions were removed on 2026-11-18 and a model hint is literal text.
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

  test("draws the context above the subtitle, and nothing when the fence omitted it", () => {
    const withContext = [
      "```futureos-choice",
      "title: 接下来做哪一件？",
      "context: 森林图已按新样本量重画，图注还挂着旧的 42。",
      "option: forest | 重画森林图",
      "option: flow | 补一张实验流程图",
      "```",
    ].join("\n");
    const root = render(withContext);
    expect(root.findAllByProps({ testID: "choice-card-context" }).length).toBeGreaterThan(0);
    const texts = root.findAll(node => typeof node.props.children === "string")
      .map(node => node.props.children);
    expect(texts).toContain("森林图已按新样本量重画，图注还挂着旧的 42。");
    // A card without one draws no empty paragraph.
    act(() => renderer.unmount());
    expect(render(CARD).findAllByProps({ testID: "choice-card-context" })).toHaveLength(0);
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
   * A card that cannot be tapped says so; the live card says nothing extra about
   * how to answer any more — the mode label and the mark already say it.
   */
  test("only the earlier card carries a line, and it is the notice", () => {
    const answer: ChoiceCardAnswer = { onPick: jest.fn(), selectedIds: [], title: "接下来做哪一件？" };
    const root = render(`${CARD}\n\n还有一件事：\n\n${SECOND_CARD}`, answer);
    expect(hintKeys(root)).toEqual(["choice.hint.readonly"]);
  });

  test("a card read-only for lack of an answer surface says so too", () => {
    const root = render(CARD);
    expect(hintKeys(root)).toEqual(["choice.hint.readonly"]);
  });

  /**
   * History keeps what the reply said. The model's own `hint` is part of it, so a
   * past card prints it and then the notice — replacing it with "cannot be
   * picked" threw away the only line the reply wrote about how to choose (§2,
   * revised 2026-11-18).
   */
  test("a past card keeps the model's own hint and adds the notice after it", () => {
    const withHint = [
      "```futureos-choice",
      "title: 这三项都要？",
      "mode: multi",
      "hint: 点几个都行",
      "option: a | 接新浪",
      "option: b | 接 Nasdaq",
      "```",
    ].join("\n");
    const root = render(withHint);
    const texts = root.findAll(node => typeof node.props.children === "string")
      .map(node => node.props.children as string);
    expect(texts).toContain("点几个都行");
    expect(texts).toContain("choice.hint.readonly");
    // What was said at the time comes first; the fact that it is inert follows.
    expect(texts.indexOf("点几个都行")).toBeLessThan(texts.indexOf("choice.hint.readonly"));
  });

  test("a live card with no hint from the reply shows no subtitle at all", () => {
    const answer: ChoiceCardAnswer = { onPick: jest.fn(), selectedIds: [], title: "接下来做哪一件？" };
    const root = render(CARD, answer);
    expect(root.findAllByProps({ testID: "choice-card-hint" })).toHaveLength(0);
    expect(hintKeys(root)).toEqual([]);
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
