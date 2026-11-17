import { createElement } from "react";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { Text } from "react-native";
import { ChoiceCardsEnabledProvider, ChoiceCardAnswerProvider } from "../ChoiceCard";
import { MarkdownText } from "../MarkdownText";

jest.mock("lucide-react-native", () => ({ Check: () => null, ListChecks: () => null }));

jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "zh-CN" } }),
}));

/** A reply that closes with a card, exactly as the agent is taught to write one. */
const REPLY = [
  "今天的检查都过了。",
  "",
  "```futureos-choice",
  "title: 接下来做哪一件？",
  "option: forest | 重画森林图",
  "option: flow | 补一张实验流程图",
  "```",
].join("\n");

/** The same fence with a single row: not a card, so it degrades to source. */
const MALFORMED = ["```futureos-choice", "title: 只有一行", "option: only | 唯一的选项", "```"].join("\n");

/** The text a `<Text>` node paints, including nested runs. */
function paintedText(node: ReactTestInstance | string): string {
  return typeof node === "string"
    ? node
    : node.children.map(child => paintedText(child)).join("");
}

/**
 * The desktop's switch, seen from the phone. The phone draws what the desktop
 * draws: with the switch off there is no card, and the fence is not surfaced as
 * a code block either — the block leaves no trace at all.
 */
describe("the desktop's choice-card switch, on the phone", () => {
  let renderer: ReactTestRenderer;
  afterEach(() => { act(() => renderer?.unmount()); });

  function render(text: string, enabled: boolean) {
    act(() => {
      renderer = create(
        createElement(
          ChoiceCardsEnabledProvider,
          { value: enabled },
          createElement(
            ChoiceCardAnswerProvider,
            { value: { onPick: jest.fn(), selectedIds: [], title: "接下来做哪一件？" } },
            createElement(MarkdownText, { text }),
          ),
        ),
      );
    });
    return renderer.root;
  }

  function texts(): string[] {
    return renderer.root.findAllByType(Text).map(node => paintedText(node));
  }

  test("off: no card node, and not one character of the fence", () => {
    const root = render(REPLY, false);
    // Structural, not visual: zero card nodes…
    expect(root.findAll(node => node.props.testID === "choice-card")).toHaveLength(0);
    expect(root.findAll(node => node.props.testID === "choice-card-degraded")).toHaveLength(0);
    // …and the fence never reaches the reader, not even as code.
    expect(JSON.stringify(renderer.toJSON())).not.toContain("futureos-choice");
    expect(texts()).not.toContain("接下来做哪一件？");
    expect(texts()).not.toContain("重画森林图");
    // The prose around it survives: only the card block goes.
    expect(texts()).toContain("今天的检查都过了。");
  });

  test("off hides a malformed fence too, instead of falling back to its source", () => {
    // A one-row card is not a card, and the renderer's normal fallback is the
    // fenced source. With the switch off that fallback must not resurface it.
    const root = render(MALFORMED, false);
    expect(root.findAll(node => node.props.testID === "choice-card")).toHaveLength(0);
    expect(JSON.stringify(renderer.toJSON())).not.toContain("futureos-choice");
    expect(texts()).not.toContain("唯一的选项");
  });

  test("on: the same input draws the card, and its rows stay pickable", () => {
    const onPick = jest.fn();
    act(() => {
      renderer = create(
        createElement(
          ChoiceCardsEnabledProvider,
          { value: true },
          createElement(
            ChoiceCardAnswerProvider,
            { value: { onPick, selectedIds: [], title: "接下来做哪一件？" } },
            createElement(MarkdownText, { text: REPLY }),
          ),
        ),
      );
    });
    const root = renderer.root;
    expect(root.findAll(node => node.props.testID === "choice-card").length).toBeGreaterThan(0);
    expect(texts()).toContain("接下来做哪一件？");
    const rows = root.findAll(
      node => typeof node.props.testID === "string" && node.props.testID.startsWith("choice-option-"),
    );
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.some(row => row.props.disabled === false)).toBe(true);
  });
});
