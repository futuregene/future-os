import type { ResolvedMarkdownReference } from "../../integrations/storage/markdownReferences";

export type FutureReferenceType = "approval" | "artifact" | "research" | "review" | "run" | "tool";
export type FutureReferenceView = "card" | "chip" | "diff-summary" | "output-summary" | "summary" | "timeline";

export interface FutureReference {
  label?: string;
  source: "inline" | "block";
  targetId: string;
  targetType: FutureReferenceType;
  view: FutureReferenceView;
}

export type InlineNode
  = | { text: string; type: "text" }
    | { children: InlineNode[]; type: "strong" }
    | { children: InlineNode[]; type: "italic" }
    | { children: InlineNode[]; type: "delete" }
    | { code: string; type: "code" }
    | { type: "break" }
    | { children: InlineNode[]; href: string; type: "link" }
    | { alt: string; src: string; title?: string; type: "image" }
    | { reference: FutureReference; type: "futureReference" };

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

export type MarkdownNode
  = | { children: InlineNode[]; level: 1 | 2 | 3; type: "heading" }
    | { children: InlineNode[]; type: "paragraph" }
    | { children: MarkdownNode[]; type: "blockquote" }
    | { code: string; language?: string; type: "code" }
    | { items: ListItemNode[]; ordered: boolean; type: "list" }
    | { reference: FutureReference; type: "futureEmbed" }
    | { type: "thematicBreak" }
    | ({ type: "table" } & TableNode);

export interface FutureMarkdownDocument {
  nodes: MarkdownNode[];
  raw: string;
  references: FutureReference[];
}

export type ResolvedReferenceMap = Record<string, ResolvedMarkdownReference>;

export function referenceKey(reference: Pick<FutureReference, "targetId" | "targetType">) {
  return `${reference.targetType}:${reference.targetId}`;
}
