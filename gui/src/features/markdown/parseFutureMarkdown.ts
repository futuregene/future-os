import type {
  Break,
  Code,
  Definition,
  Emphasis,
  Html,
  Image,
  ImageReference,
  InlineCode,
  Link,
  LinkReference,
  List,
  ListItem,
  PhrasingContent,
  Root,
  RootContent,
  Strong,
  Table,
  TableCell,
  Text,
} from "mdast";
import type {
  FutureMarkdownDocument,
  FutureReference,
  FutureReferenceType,
  FutureReferenceView,
  InlineNode,
  ListItemNode,
  MarkdownNode,
} from "./futureMarkdownTypes";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import { unified } from "unified";

const markdownProcessor = unified()
  .use(remarkParse)
  .use(remarkGfm);

interface ParseContext {
  definitions: Map<string, Definition>;
}

export function parseFutureMarkdown(raw: string): FutureMarkdownDocument {
  const tree = parseMdast(raw);
  const context = createParseContext(tree);
  const nodes = tree.children.flatMap(node => blockToFutureNode(node, context));
  const references = collectReferences(nodes);
  return { nodes, raw, references };
}

function parseMdast(raw: string): Root {
  return markdownProcessor.parse(raw) as Root;
}

function createParseContext(tree: Root): ParseContext {
  const definitions = new Map<string, Definition>();
  for (const node of tree.children) {
    if (node.type === "definition") {
      definitions.set(normalizeIdentifier(node.identifier), node);
    }
  }
  return { definitions };
}

function blockToFutureNode(node: RootContent, context: ParseContext): MarkdownNode[] {
  switch (node.type) {
    case "blockquote":
      return [{ children: node.children.flatMap(child => blockToFutureNode(child, context)), type: "blockquote" }];
    case "break":
      return [{ children: [{ type: "break" }], type: "paragraph" }];
    case "code": {
      const futureEmbed = parseFutureEmbed(node);
      if (futureEmbed) {
        return [{ reference: futureEmbed, type: "futureEmbed" }];
      }
      return [{ code: node.value, language: node.lang ?? undefined, type: "code" }];
    }
    case "definition":
      return [];
    case "delete":
      return [{ children: [{ children: phrasingToInline(node.children, context), type: "delete" }], type: "paragraph" }];
    case "footnoteDefinition":
      return [
        {
          children: node.children.flatMap(child => blockToFutureNode(child, context)),
          type: "blockquote",
        },
      ];
    case "heading":
      return [{ children: phrasingToInline(node.children, context), level: normalizeHeadingLevel(node.depth), type: "heading" }];
    case "html":
      return htmlToSafeParagraph(node);
    case "image":
      return [{ children: [imageToInline(node)], type: "paragraph" }];
    case "imageReference": {
      const image = imageReferenceToInline(node, context);
      return [{ children: image ? [image] : [{ text: mdastText(node), type: "text" }], type: "paragraph" }];
    }
    case "linkReference":
      return [{ children: [linkReferenceToInline(node, context)], type: "paragraph" }];
    case "list":
      return [listToFutureNode(node, context)];
    case "paragraph":
      return [{ children: phrasingToInline(node.children, context), type: "paragraph" }];
    case "table":
      return [tableToFutureNode(node, context)];
    case "text":
      return [{ children: [{ text: node.value, type: "text" }], type: "paragraph" }];
    case "thematicBreak":
      return [{ type: "thematicBreak" }];
    case "yaml":
      return [{ code: node.value, language: "yaml", type: "code" }];
    default:
      return [];
  }
}

function phrasingToInline(nodes: PhrasingContent[], context: ParseContext): InlineNode[] {
  return compactTextNodes(nodes.flatMap(node => phrasingNodeToInline(node, context)));
}

function phrasingNodeToInline(node: PhrasingContent, context: ParseContext): InlineNode[] {
  switch (node.type) {
    case "break":
      return [breakToInline(node)];
    case "delete":
      return [{ children: phrasingToInline(node.children, context), type: "delete" }];
    case "emphasis":
      return [emphasisToInline(node, context)];
    case "footnoteReference":
      return [{ text: `[^${node.identifier}]`, type: "text" }];
    case "html":
      return [{ text: node.value, type: "text" }];
    case "image":
      return [imageToInline(node)];
    case "imageReference":
      return [imageReferenceToInline(node, context) ?? { text: mdastText(node), type: "text" }];
    case "inlineCode":
      return [inlineCodeToInline(node)];
    case "link":
      return [linkToInline(node, context)];
    case "linkReference":
      return [linkReferenceToInline(node, context)];
    case "strong":
      return [strongToInline(node, context)];
    case "text":
      return [textToInline(node)];
    default:
      return [];
  }
}

function breakToInline(_node: Break): InlineNode {
  return { type: "break" };
}

function emphasisToInline(node: Emphasis, context: ParseContext): InlineNode {
  return { children: phrasingToInline(node.children, context), type: "italic" };
}

function imageToInline(node: Image): InlineNode {
  return {
    alt: node.alt ?? "",
    src: node.url,
    title: node.title ?? undefined,
    type: "image",
  };
}

function inlineCodeToInline(node: InlineCode): InlineNode {
  return { code: node.value, type: "code" };
}

function linkToInline(node: Link, context: ParseContext): InlineNode {
  const label = mdastText({ children: node.children, type: "paragraph" });
  const reference = parseFutureLink(label, node.url);
  if (reference) {
    return { reference, type: "futureReference" };
  }
  return { children: phrasingToInline(node.children, context), href: node.url, type: "link" };
}

function linkReferenceToInline(node: LinkReference, context: ParseContext): InlineNode {
  const definition = context.definitions.get(normalizeIdentifier(node.identifier));
  const label = mdastText({ children: node.children, type: "paragraph" });
  if (!definition) {
    return { text: label, type: "text" };
  }

  const reference = parseFutureLink(label, definition.url);
  if (reference) {
    return { reference, type: "futureReference" };
  }
  return { children: phrasingToInline(node.children, context), href: definition.url, type: "link" };
}

function imageReferenceToInline(node: ImageReference, context: ParseContext): InlineNode | null {
  const definition = context.definitions.get(normalizeIdentifier(node.identifier));
  if (!definition)
    return null;

  return {
    alt: node.alt ?? "",
    src: definition.url,
    title: definition.title ?? undefined,
    type: "image",
  };
}

function strongToInline(node: Strong, context: ParseContext): InlineNode {
  return { children: phrasingToInline(node.children, context), type: "strong" };
}

function textToInline(node: Text): InlineNode {
  return { text: node.value, type: "text" };
}

function listToFutureNode(node: List, context: ParseContext): MarkdownNode {
  return {
    items: node.children.map(child => listItemToFutureNode(child, context)),
    ordered: Boolean(node.ordered),
    type: "list",
  };
}

function listItemToFutureNode(node: ListItem, context: ParseContext): ListItemNode {
  const convertedBlocks = node.children.flatMap(child => blockToFutureNode(child, context));
  const firstParagraph = convertedBlocks[0]?.type === "paragraph" ? convertedBlocks[0] : null;
  const children = firstParagraph?.children ?? [];
  const blocks = firstParagraph ? convertedBlocks.slice(1) : convertedBlocks;
  return {
    blocks: blocks.length > 0 ? blocks : undefined,
    checked: node.checked ?? undefined,
    children,
  };
}

function tableToFutureNode(node: Table, context: ParseContext): MarkdownNode {
  const [headerRow, ...bodyRows] = node.children;
  const headers = headerRow ? tableRowToCells(headerRow.children, context) : [];
  return {
    alignments: node.align?.map(alignment => alignment ?? null) ?? headers.map(() => null),
    headers,
    rows: bodyRows.map(row => tableRowToCells(row.children, context, headers.length)),
    type: "table",
  };
}

function tableRowToCells(cells: TableCell[], context: ParseContext, length?: number): InlineNode[][] {
  const parsedCells = cells.map(cell => phrasingToInline(cell.children, context));
  if (length === undefined || parsedCells.length === length)
    return parsedCells;
  if (parsedCells.length > length)
    return parsedCells.slice(0, length);
  const emptyCells = Array.from({ length: length - parsedCells.length }).fill([]) as InlineNode[][];
  return [...parsedCells, ...emptyCells];
}

function htmlToSafeParagraph(node: Html): MarkdownNode[] {
  const trimmed = node.value.trim();
  if (!trimmed)
    return [];
  return [{ children: [{ text: trimmed, type: "text" }], type: "paragraph" }];
}

function parseFutureEmbed(node: Code): FutureReference | null {
  const match = node.lang?.match(/^futureos-(approval|artifact|file|research|review|run|tool)$/);
  if (!match)
    return null;

  const fields = parseDirectiveFields(node.value.split("\n"));
  const id = fields.id?.trim();
  if (!id)
    return null;

  return {
    label: fields.title,
    source: "block",
    targetId: id,
    targetType: match[1] as FutureReferenceType,
    view: normalizeView(fields.view),
  };
}

function parseDirectiveFields(lines: string[]) {
  const fields: Record<string, string> = {};
  for (const line of lines) {
    const separator = line.indexOf(":");
    if (separator <= 0)
      continue;
    const key = line.slice(0, separator).trim();
    const value = line.slice(separator + 1).trim();
    if (key) {
      fields[key] = value;
    }
  }
  return fields;
}

function parseFutureLink(label: string, href: string): FutureReference | null {
  const parsed = parseFutureUrl(href);
  if (!parsed)
    return null;

  return {
    label,
    source: "inline",
    targetId: parsed.targetId,
    targetType: parsed.targetType,
    view: parsed.view ?? "chip",
  };
}

function parseFutureUrl(href: string) {
  try {
    const url = new URL(href);
    if (url.protocol !== "futureos:")
      return null;

    const targetType = url.hostname;
    if (!isFutureReferenceType(targetType))
      return null;

    const targetId = safeDecodeURIComponent(url.pathname.replace(/^\/+/, ""));
    if (!targetId)
      return null;

    return {
      targetId,
      targetType,
      view: normalizeInlineView(url.searchParams.get("view") ?? undefined),
    };
  }
  catch {
    return null;
  }
}

function safeDecodeURIComponent(value: string) {
  try {
    return decodeURIComponent(value);
  }
  catch {
    return value;
  }
}

function isFutureReferenceType(value: string): value is FutureReferenceType {
  return value === "approval"
    || value === "artifact"
    || value === "file"
    || value === "research"
    || value === "review"
    || value === "run"
    || value === "tool";
}

function normalizeInlineView(view: string | undefined): FutureReferenceView | undefined {
  if (!view)
    return undefined;
  if (view === "card"
    || view === "chip"
    || view === "diff-summary"
    || view === "output-summary"
    || view === "summary"
    || view === "timeline") {
    return view;
  }
  return undefined;
}

function normalizeView(view: string | undefined): FutureReferenceView {
  if (view === "chip")
    return "chip";
  if (view === "diff-summary")
    return "diff-summary";
  if (view === "output-summary")
    return "output-summary";
  if (view === "timeline")
    return "timeline";
  if (view === "summary")
    return "summary";
  return "card";
}

function normalizeHeadingLevel(depth: number): 1 | 2 | 3 {
  if (depth <= 1)
    return 1;
  if (depth === 2)
    return 2;
  return 3;
}

function normalizeIdentifier(value: string) {
  return value.trim().replace(/\s+/g, " ").toLowerCase();
}

function collectReferences(nodes: MarkdownNode[]) {
  const references: FutureReference[] = [];
  for (const node of nodes) {
    collectBlockReferences(node, references);
  }
  return references;
}

function collectBlockReferences(node: MarkdownNode, references: FutureReference[]) {
  if (node.type === "futureEmbed") {
    references.push(node.reference);
    return;
  }

  if (node.type === "paragraph" || node.type === "heading") {
    collectInlineReferences(node.children, references);
    return;
  }

  if (node.type === "blockquote") {
    for (const child of node.children) {
      collectBlockReferences(child, references);
    }
    return;
  }

  if (node.type === "list") {
    for (const item of node.items) {
      collectInlineReferences(item.children, references);
      for (const block of item.blocks ?? []) {
        collectBlockReferences(block, references);
      }
    }
    return;
  }

  if (node.type === "table") {
    for (const header of node.headers) {
      collectInlineReferences(header, references);
    }
    for (const row of node.rows) {
      for (const cell of row) {
        collectInlineReferences(cell, references);
      }
    }
  }
}

function collectInlineReferences(nodes: InlineNode[], references: FutureReference[]) {
  for (const node of nodes) {
    if (node.type === "futureReference") {
      references.push(node.reference);
    }
    else if (node.type === "strong" || node.type === "italic" || node.type === "delete") {
      collectInlineReferences(node.children, references);
    }
  }
}

function compactTextNodes(nodes: InlineNode[]) {
  const compacted: InlineNode[] = [];
  for (const node of nodes) {
    const previous = compacted[compacted.length - 1];
    if (node.type === "text" && previous?.type === "text") {
      previous.text += node.text;
    }
    else {
      compacted.push(node);
    }
  }
  return compacted;
}

function mdastText(node: { children?: PhrasingContent[]; type: string; value?: string }): string {
  if (typeof node.value === "string")
    return node.value;
  return node.children?.map(child => mdastText(child)).join("") ?? "";
}
