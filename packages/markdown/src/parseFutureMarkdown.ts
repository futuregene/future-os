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
  StreamingMarkdownBlock,
} from "./types";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import remarkParse from "remark-parse";
import { unified } from "unified";
import { localFilePath } from "./localPath";

const markdownProcessor = unified().use(remarkParse).use(remarkMath).use(remarkGfm);

// Cross-instance parse cache: `useMemo([content])` in MarkdownContent only
// survives within one mounted component, so a thread switch re-parses every
// message's markdown from scratch. Caching by content string here makes a
// re-mount of a previously-rendered message (thread switch, lazy-window
// materialization) a map lookup instead of a unified parse. The returned
// document is rendered read-only (MarkdownContent maps `nodes`; consumers
// never mutate it), so sharing it across instances is safe. LRU-bounded so a
// long-lived app doesn't hoard every reply ever parsed.
const parseCache = new Map<string, FutureMarkdownDocument>();
// Tuned up after the lazy-mount window was reverted: with the full list
// re-rendered on every thread switch, a larger cache covers more distinct
// messages (long conversations, forks, retried replies) before eviction.
const PARSE_CACHE_MAX = 512;

interface ParseContext {
  definitions: Map<string, Definition>;
}

export function parseFutureMarkdown(raw: string): FutureMarkdownDocument {
  const cached = parseCache.get(raw);
  if (cached) {
    // LRU touch.
    parseCache.delete(raw);
    parseCache.set(raw, cached);
    return cached;
  }
  const tree = parseMdast(raw);
  const context = createParseContext(tree);
  const nodes = tree.children.flatMap((node) =>
    blockToFutureNode(node, context),
  );
  const references = collectReferences(nodes);
  const document = { nodes, raw, references };
  if (parseCache.size >= PARSE_CACHE_MAX) {
    const oldest = parseCache.keys().next().value;
    if (oldest !== undefined) parseCache.delete(oldest);
  }
  parseCache.set(raw, document);
  return document;
}

/**
 * Split a growing Markdown document at top-level parser boundaries. Everything
 * before the final block is immutable; the final block may still change meaning
 * when another line arrives (setext heading, list tightness, table, fence, ...).
 *
 * Reference definitions can affect blocks anywhere in the document, so keep
 * those documents whole until the stream settles instead of freezing a prefix
 * with incomplete definition context.
 */
export function splitStreamingMarkdown(raw: string, live: boolean): StreamingMarkdownBlock[] {
  if (!raw)
    return [];

  const tree = parseMdast(raw);
  if (tree.children.some(node => node.type === "definition")) {
    return [{ content: raw, live, start: 0 }];
  }

  const starts = tree.children
    .map(node => node.position?.start.offset)
    .filter((offset): offset is number => typeof offset === "number");
  if (starts.length <= 1)
    return [{ content: raw, live, start: 0 }];

  const blocks: StreamingMarkdownBlock[] = [];
  for (let index = 0; index < starts.length; index++) {
    // Preserve leading whitespace in the first block. Subsequent parser offsets
    // naturally retain the blank-line separator at the end of the prior slice.
    const start = index === 0 ? 0 : starts[index]!;
    const end = starts[index + 1] ?? raw.length;
    if (end <= start)
      continue;
    blocks.push({
      content: raw.slice(start, end),
      live: live && index === starts.length - 1,
      start,
    });
  }
  return blocks.length > 0 ? blocks : [{ content: raw, live, start: 0 }];
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

function blockToFutureNode(
  node: RootContent,
  context: ParseContext,
): MarkdownNode[] {
  switch (node.type) {
    case "blockquote":
      return [
        {
          children: node.children.flatMap((child) =>
            blockToFutureNode(child, context),
          ),
          type: "blockquote",
        },
      ];
    /* v8 ignore next 2 -- remark never emits a bare break as a block child */
    case "break":
      return [{ children: [{ type: "break" }], type: "paragraph" }];
    case "code": {
      const futureEmbed = parseFutureEmbed(node);
      if (futureEmbed) {
        return [{ reference: futureEmbed, type: "futureEmbed" }];
      }
      return [
        { code: node.value, language: node.lang ?? undefined, type: "code" },
      ];
    }
    case "definition":
      return [];
    /* v8 ignore next 2 -- remark never emits delete (GFM strikethrough) as a block child */
    case "delete":
      return [
        {
          children: [
            {
              children: phrasingToInline(node.children, context),
              type: "delete",
            },
          ],
          type: "paragraph",
        },
      ];
    case "footnoteDefinition":
      return [
        {
          children: node.children.flatMap((child) =>
            blockToFutureNode(child, context),
          ),
          type: "blockquote",
        },
      ];
    case "heading":
      return [
        {
          children: phrasingToInline(node.children, context),
          level: normalizeHeadingLevel(node.depth),
          type: "heading",
        },
      ];
    case "html":
      return htmlToSafeParagraph(node);
    case "math":
      return [{ code: node.value, type: "mathBlock" }];
    /* v8 ignore start -- remark never emits phrasing nodes as block children */
    case "image":
      return [{ children: [imageToInline(node)], type: "paragraph" }];
    case "imageReference": {
      const image = imageReferenceToInline(node, context);
      return [
        {
          children: image ? [image] : [{ text: mdastText(node), type: "text" }],
          type: "paragraph",
        },
      ];
    }
    case "linkReference":
      return [
        { children: [linkReferenceToInline(node, context)], type: "paragraph" },
      ];
    /* v8 ignore stop */
    case "list":
      return [listToFutureNode(node, context)];
    case "paragraph": {
      const children = phrasingToInline(node.children, context);
      // A paragraph containing only a single math formula (e.g. `$$E=mc^2$$`
      // on one line, the model's common output) is promoted to a block-level
      // formula so it renders centered, matching Typora/Obsidian behavior.
      if (
        children.length === 1
        && children[0]?.type === "mathInline"
      ) {
        return [{ code: children[0].code, type: "mathBlock" }];
      }
      return [{ children, type: "paragraph" }];
    }
    case "table":
      return [tableToFutureNode(node, context)];
    /* v8 ignore next 2 -- remark never emits a bare text node as a block child */
    case "text":
      return [{ children: textToInlineNodes(node), type: "paragraph" }];
    case "thematicBreak":
      return [{ type: "thematicBreak" }];
    /* v8 ignore start -- the frontmatter plugin is not enabled, and the mdast
       union is exhaustive, so the yaml arm and default are defensive only */
    case "yaml":
      return [{ code: node.value, language: "yaml", type: "code" }];
    default:
      return [];
    /* v8 ignore stop */
  }
}

function phrasingToInline(
  nodes: PhrasingContent[],
  context: ParseContext,
): InlineNode[] {
  return compactTextNodes(
    nodes.flatMap((node) => phrasingNodeToInline(node, context)),
  );
}

function phrasingNodeToInline(
  node: PhrasingContent,
  context: ParseContext,
): InlineNode[] {
  switch (node.type) {
    case "break":
      return [breakToInline(node)];
    case "delete":
      return [
        { children: phrasingToInline(node.children, context), type: "delete" },
      ];
    case "emphasis":
      return [emphasisToInline(node, context)];
    case "footnoteReference":
      return [{ text: `[^${node.identifier}]`, type: "text" }];
    case "html":
      return [{ text: node.value, type: "text" }];
    case "image":
      return [imageToInline(node)];
    case "imageReference":
      return [
        imageReferenceToInline(node, context) ?? {
          text: mdastText(node),
          type: "text",
        },
      ];
    case "inlineCode":
      return [inlineCodeToInline(node)];
    case "inlineMath":
      return [{ code: node.value, displayMode: false, type: "mathInline" }];
    case "link":
      return [linkToInline(node, context)];
    case "linkReference":
      return [linkReferenceToInline(node, context)];
    case "strong":
      return [strongToInline(node, context)];
    case "text":
      return textToInlineNodes(node);
    /* v8 ignore next 2 -- exhaustive over PhrasingContent; defensive only */
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
  return {
    children: phrasingToInline(node.children, context),
    href: node.url,
    type: "link",
  };
}

function linkReferenceToInline(
  node: LinkReference,
  context: ParseContext,
): InlineNode {
  const definition = context.definitions.get(
    normalizeIdentifier(node.identifier),
  );
  const label = mdastText({ children: node.children, type: "paragraph" });
  /* v8 ignore next 3 -- remark only creates reference nodes for defined identifiers */
  if (!definition) {
    return { text: label, type: "text" };
  }

  const reference = parseFutureLink(label, definition.url);
  if (reference) {
    return { reference, type: "futureReference" };
  }
  return {
    children: phrasingToInline(node.children, context),
    href: definition.url,
    type: "link",
  };
}

function imageReferenceToInline(
  node: ImageReference,
  context: ParseContext,
): InlineNode | null {
  const definition = context.definitions.get(
    normalizeIdentifier(node.identifier),
  );
  /* v8 ignore next 2 -- remark only creates reference nodes for defined identifiers */
  if (!definition) return null;

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

/**
 * A bare `[<local path>]` mention inside plain text (remark keeps it literal
 * since there's no link definition). Only pure paths qualify — see
 * `localFilePath` — so ordinary `[text]` in prose stays untouched.
 */
const BRACKET_PATH = /\[([^\]\n]+)\]/g;

function textToInlineNodes(node: Text): InlineNode[] {
  const value = node.value;
  const nodes: InlineNode[] = [];
  let last = 0;
  BRACKET_PATH.lastIndex = 0;
  for (
    let match = BRACKET_PATH.exec(value);
    match;
    match = BRACKET_PATH.exec(value)
  ) {
    const inner = match[1] ?? "";
    const path = localFilePath(inner);
    if (!path) continue;
    if (match.index > last)
      nodes.push({ text: value.slice(last, match.index), type: "text" });
    nodes.push({
      reference: {
        label: inner,
        source: "inline",
        targetId: path,
        targetType: "file",
        view: "chip",
      },
      type: "futureReference",
    });
    last = match.index + match[0].length;
  }
  if (nodes.length === 0) return [{ text: value, type: "text" }];
  if (last < value.length)
    nodes.push({ text: value.slice(last), type: "text" });
  return nodes;
}

function listToFutureNode(node: List, context: ParseContext): MarkdownNode {
  return {
    items: node.children.map((child) => listItemToFutureNode(child, context)),
    ordered: Boolean(node.ordered),
    type: "list",
  };
}

function listItemToFutureNode(
  node: ListItem,
  context: ParseContext,
): ListItemNode {
  const convertedBlocks = node.children.flatMap((child) =>
    blockToFutureNode(child, context),
  );
  const firstParagraph =
    convertedBlocks[0]?.type === "paragraph" ? convertedBlocks[0] : null;
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
    alignments:
      node.align?.map((alignment) => alignment ?? null) ??
      headers.map(() => null),
    headers,
    rows: bodyRows.map((row) =>
      tableRowToCells(row.children, context, headers.length),
    ),
    type: "table",
  };
}

function tableRowToCells(
  cells: TableCell[],
  context: ParseContext,
  length?: number,
): InlineNode[][] {
  const parsedCells = cells.map((cell) =>
    phrasingToInline(cell.children, context),
  );
  if (length === undefined || parsedCells.length === length) return parsedCells;
  if (parsedCells.length > length) return parsedCells.slice(0, length);
  const emptyCells = Array.from({ length: length - parsedCells.length }).fill(
    [],
  ) as InlineNode[][];
  return [...parsedCells, ...emptyCells];
}

function htmlToSafeParagraph(node: Html): MarkdownNode[] {
  const trimmed = node.value.trim();
  /* v8 ignore next 2 -- remark html nodes always have non-whitespace content */
  if (!trimmed) return [];
  return [{ children: [{ text: trimmed, type: "text" }], type: "paragraph" }];
}

function parseFutureEmbed(node: Code): FutureReference | null {
  // Minimal link mode: only `futureos-file` block embeds are recognized;
  // application-object embeds (approval/artifact/review/run/tool) are
  // disabled and fall through to a plain code block. To restore them, widen the
  // pattern back to `futureos-(approval|artifact|file|review|run|tool)`.
  const match = node.lang?.match(/^futureos-(file)$/);
  if (!match) return null;

  const fields = parseDirectiveFields(node.value.split("\n"));
  const id = fields.id?.trim();
  if (!id) return null;

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
    if (separator <= 0) continue;
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
  /* v8 ignore start -- disabled minimal link mode: parseFutureUrl is always null
     while isFutureReferenceType returns false */
  if (parsed) {
    return {
      label,
      source: "inline",
      targetId: parsed.targetId,
      targetType: parsed.targetType,
      view: parsed.view ?? "chip",
    };
  }
  /* v8 ignore stop */

  // A plain markdown link whose destination is a local path becomes a `file`
  // reference — same display/menu pipeline as the old `futureos://file/…`, but
  // the model just writes the path verbatim (no scheme, no percent-encoding).
  const path = localFilePath(href);
  if (path) {
    return {
      label,
      source: "inline",
      targetId: path,
      targetType: "file",
      view: "chip",
    };
  }

  return null;
}

function parseFutureUrl(href: string) {
  try {
    const url = new URL(href);
    if (url.protocol !== "futureos:") return null;

    const targetType = url.hostname;
    if (!isFutureReferenceType(targetType)) return null;

    /* v8 ignore start -- unreachable while isFutureReferenceType returns false
       (minimal link mode); retained for re-enabling app-object references */
    // `futureos://` carries only id-based internal objects (artifact/run/…);
    // local files use plain markdown-path links instead. Ids never start with a
    // slash, so stripping the single URL path separator is all that's needed.
    const targetId = safeDecodeURIComponent(url.pathname.replace(/^\//, ""));
    if (!targetId) return null;

    return {
      targetId,
      targetType,
      view: normalizeInlineView(url.searchParams.get("view") ?? undefined),
    };
    /* v8 ignore stop */
  } catch {
    return null;
  }
}

/* v8 ignore start -- only called from the disabled futureos:// path */
function safeDecodeURIComponent(value: string) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}
/* v8 ignore stop */

function isFutureReferenceType(value: string): value is FutureReferenceType {
  // Minimal link mode: application-object references (approval/artifact/review/
  // run via the `futureos://` scheme) are disabled — any such link
  // falls through to a plain/inert link. Local files are unaffected: they arrive
  // as plain markdown path links, not via this scheme. To restore app objects,
  // uncomment the checks below (and re-enable them in `parseFutureEmbed` and the
  // prompt guidelines in agent/src/prompt/mod.rs).
  void value;
  return false;
  // return value === "approval"
  //   || value === "artifact"
  //   || value === "review"
  //   || value === "run";
}

/* v8 ignore start -- only called from the disabled futureos:// path */
function normalizeInlineView(
  view: string | undefined,
): FutureReferenceView | undefined {
  if (!view) return undefined;
  if (
    view === "card" ||
    view === "chip" ||
    view === "diff-summary" ||
    view === "output-summary" ||
    view === "summary" ||
    view === "timeline"
  ) {
    return view;
  }
  return undefined;
}
/* v8 ignore stop */

function normalizeView(view: string | undefined): FutureReferenceView {
  if (view === "chip") return "chip";
  if (view === "diff-summary") return "diff-summary";
  if (view === "output-summary") return "output-summary";
  if (view === "timeline") return "timeline";
  if (view === "summary") return "summary";
  return "card";
}

function normalizeHeadingLevel(depth: number): 1 | 2 | 3 {
  if (depth <= 1) return 1;
  if (depth === 2) return 2;
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

function collectBlockReferences(
  node: MarkdownNode,
  references: FutureReference[],
) {
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

function collectInlineReferences(
  nodes: InlineNode[],
  references: FutureReference[],
) {
  for (const node of nodes) {
    if (node.type === "futureReference") {
      references.push(node.reference);
    } else if (node.type === "image") {
      const path = localFilePath(node.src);
      if (path) {
        references.push({
          label: node.alt || undefined,
          source: "inline",
          targetId: path,
          targetType: "file",
          view: "chip",
        });
      }
    } else if (
      node.type === "strong" ||
      node.type === "italic" ||
      node.type === "delete"
    ) {
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
    } else {
      compacted.push(node);
    }
  }
  return compacted;
}

function mdastText(node: {
  children?: PhrasingContent[];
  type: string;
  value?: string;
}): string {
  if (typeof node.value === "string") return node.value;
  return node.children?.map((child) => mdastText(child)).join("") ?? "";
}
