export type FutureReferenceType =
  "approval" | "artifact" | "file" | "review" | "run";
export type FutureReferenceView =
  "card" | "chip" | "diff-summary" | "output-summary" | "summary" | "timeline";

export interface FutureReference {
  label?: string;
  source: "inline" | "block";
  targetId: string;
  targetType: FutureReferenceType;
  view: FutureReferenceView;
}

/** How many of a `futureos-choice` card's options can be picked. */
export type ChoiceCardMode = "single" | "multi";

/** One row of a `futureos-choice` card. */
export interface ChoiceCardOption {
  /** One line on what picking it changes; the fence may omit it. */
  description?: string;
  /** Stable identifier echoed back on send: ASCII `[a-z0-9-]`, ≤ 24 chars. */
  id: string;
  /**
   * The instruction as the user would say it. This is the text that reaches the
   * agent when the option is picked, so it is a sentence, not a menu label.
   */
  label: string;
}

/**
 * A `futureos-choice` block: the reply's closing question, drawn as a card.
 *
 * The card is the question's view; the answer lives in the composer draft
 * (design `docs/design/choice-cards.zh-CN.md` §2), which is why the node carries
 * no selection state.
 */
export interface ChoiceCardNode {
  /** Overrides the default subtitle ("pick one and it lands in the input"). */
  hint?: string;
  mode: ChoiceCardMode;
  options: ChoiceCardOption[];
  title: string;
  type: "choiceCard";
}

export type InlineNode =
  | { text: string; type: "text" }
  | { children: InlineNode[]; type: "strong" }
  | { children: InlineNode[]; type: "italic" }
  | { children: InlineNode[]; type: "delete" }
  | { code: string; type: "code" }
  | { type: "break" }
  | { children: InlineNode[]; href: string; type: "link" }
  | { alt: string; src: string; title?: string; width?: number; type: "image" }
  | { reference: FutureReference; children?: InlineNode[]; type: "futureReference" }
  | { code: string; displayMode: false; type: "mathInline" };

export interface ListItemNode {
  blocks?: MarkdownNode[];
  checked?: boolean;
  children: InlineNode[];
}

export interface TableNode {
  alignments: Array<"center" | "left" | "right" | null>;
  headers: InlineNode[][];
  rows: InlineNode[][][];
}

export type MarkdownNode =
  | { children: InlineNode[]; level: 1 | 2 | 3 | 4 | 5 | 6; type: "heading"; align?: "center" | "left" | "right" }
  | { children: InlineNode[]; type: "paragraph"; align?: "center" | "left" | "right" }
  | { children: MarkdownNode[]; type: "blockquote" }
  | { code: string; language?: string; type: "code" }
  | { items: ListItemNode[]; ordered: boolean; start?: number; type: "list" }
  | { reference: FutureReference; type: "futureEmbed" }
  | ChoiceCardNode
  | { type: "thematicBreak" }
  | ({ type: "table" } & TableNode)
  | { code: string; type: "mathBlock" };

export interface FutureMarkdownDocument {
  nodes: MarkdownNode[];
  raw: string;
  references: FutureReference[];
}

export function referenceKey(
  reference: Pick<FutureReference, "targetId" | "targetType">,
) {
  return `${reference.targetType}:${reference.targetId}`;
}
