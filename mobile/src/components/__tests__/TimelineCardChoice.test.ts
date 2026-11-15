import { createElement } from "react";
import { AppState } from "react-native";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { TimelineCard } from "../TimelineCard";
import type { TimelineCardProps } from "../TimelineCard";
import type { TimelineItem } from "../../remote/types";
import { setAppearance } from "../../theme/preferences";

/**
 * §2's "only the thread's last card is interactive" is decided by the reply, not
 * by the card: the screen hands an answer surface to every reply, and a reply
 * offers it only when it is the thread's **last** one and has **settled**. The
 * card itself (which of a reply's cards takes the pick) has its own suite;
 * `MarkdownText` stays real here because that is the boundary under test.
 */
jest.mock("lucide-react-native", () => Object.fromEntries([
  "AlertTriangle", "Brain", "Check", "ChevronDown", "ChevronUp", "CircleAlert", "Copy", "FileText",
  "GitBranch", "ListChecks", "Paperclip", "Pencil", "TerminalSquare", "TriangleAlert", "Wrench", "X",
].map(name => [name, name])));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "zh-CN" } }),
}));

const CARD = "接下来做哪一件？\n\n```futureos-choice\ntitle: 接下来做哪一件？\noption: a | 重画森林图\noption: b | 补一张实验流程图\n```";

const reply = (fields: Partial<Extract<TimelineItem, { kind: "message" }>> = {}): TimelineItem => ({
  id: "a",
  kind: "message",
  role: "assistant",
  segments: [{ id: "t1", kind: "text", text: CARD }],
  text: CARD,
  ...fields,
});

describe("the answer surface a reply offers its closing card", () => {
  let tree: ReactTestRenderer;
  beforeEach(async () => {
    await setAppearance({ replyDetail: "detailed" });
    (AppState.addEventListener as jest.Mock).mockReturnValue({ remove: jest.fn() });
  });
  afterEach(() => { if (tree) act(() => tree.unmount()); });

  function render(item: TimelineItem, props: Partial<TimelineCardProps> = {}) {
    act(() => { tree = create(createElement(TimelineCard, { item, ...props })); });
  }

  /** The card's rows, as the reader can reach them. */
  function rows() {
    const seen = new Set<string>();
    return tree.root.findAll(node => {
      const testID: unknown = node.props.testID;
      if (typeof testID !== "string" || !testID.startsWith("choice-option")) return false;
      if (node.props.accessibilityRole === undefined || seen.has(testID)) return false;
      seen.add(testID);
      return true;
    });
  }

  const answer = { onPick: jest.fn(), selectedIds: [], title: "接下来做哪一件？" };

  test("the thread's last settled reply answers, and its rows report the pick path", () => {
    render(reply(), { choiceAnswer: answer, isLatestAssistant: true });
    const options = rows();
    expect(options).toHaveLength(2);
    for (const row of options) expect(row.props.disabled).toBe(false);
    act(() => options[1]!.props.onPress());
    expect(answer.onPick).toHaveBeenCalledWith(
      expect.objectContaining({ title: "接下来做哪一件？", type: "choiceCard" }),
      "b",
    );
  });

  test("an earlier reply's card is read-only: its question was already answered", () => {
    render(reply(), { choiceAnswer: answer, isLatestAssistant: false });
    const options = rows();
    expect(options).toHaveLength(2);
    for (const row of options) expect(row.props.disabled).toBe(true);
  });

  test("a reply still being written offers nothing: its closing question does not exist yet", async () => {
    // The last settle turns the rows on; until then a fence can still gain rows
    // under a finger.
    render({ ...reply({ streaming: true }) } as TimelineItem, { choiceAnswer: answer, isLatestAssistant: true });
    for (const row of rows()) expect(row.props.disabled).toBe(true);
    // The motion preference is read asynchronously while a reply streams; let it
    // land inside this test rather than after it.
    await act(async () => { await Promise.resolve(); });
  });

  test("a caller that offers no answer surface leaves every card readable but inert", () => {
    render(reply(), { isLatestAssistant: true });
    const options = rows();
    expect(options).toHaveLength(2);
    for (const row of options) expect(row.props.disabled).toBe(true);
  });
});
