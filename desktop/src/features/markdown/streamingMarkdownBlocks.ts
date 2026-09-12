import type { FutureMarkdownDocument, MarkdownNode } from "@future-os/markdown";
import type { Root } from "mdast";
import { parseFutureMarkdown, remarkLatexMath } from "@future-os/markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import remarkParse from "remark-parse";
import { unified } from "unified";

const streamingMarkdownProcessor = unified().use(remarkParse).use(remarkMath).use(remarkGfm).use(remarkLatexMath);

export interface StreamingMarkdownBlock {
  /** Stable source offset used as the renderer key while later text grows. */
  start: number;
  /** Exact Markdown source for this top-level slice. */
  content: string;
  /** Only the final block of an in-flight reply remains mutable. */
  live: boolean;
  /** Parsed off-thread for streaming content; omitted by the boundary splitter. */
  document?: FutureMarkdownDocument;
  parsed?: true;
}

/** Worker-side parsing: the UI must not parse the mutable block again. */
export function projectStreamingMarkdown(text: string, live: boolean): StreamingMarkdownBlock[] {
  if (!text)
    return [];
  const tree = streamingMarkdownProcessor.parse(text) as Root;
  return projectTree(text, live, tree);
}

function projectTree(text: string, live: boolean, tree: Root): StreamingMarkdownBlock[] {
  const blocks = splitTree(text, live, tree);
  // Definitions can change inline tokenization across block boundaries. Keep
  // the existing isolated-block parsing for those documents rather than using
  // context-dependent subtrees. Ordinary blocks reuse the already parsed tree.
  const independent = blocks.length > 1 && blocks.length === tree.children.length && !hasDefinitions(tree);
  return blocks.map((block, index) => ({
    ...block,
    document: parseFutureMarkdown(
      block.content,
      blocks.length === 1 ? tree : independent ? { type: "root", children: [tree.children[index]!] } : undefined,
      { cache: !block.live },
    ),
    parsed: true,
  }));
}

interface DefinitionNode {
  type: string;
  children?: DefinitionNode[];
}

function hasDefinitions(root: DefinitionNode): boolean {
  const stack = [root];
  while (stack.length > 0) {
    const node = stack.pop()!;
    if (node.type === "definition" || node.type === "footnoteDefinition")
      return true;
    // Definitions are flow nodes, never children of headings/table cells or
    // other inline content. Only descend into containers that can hold them.
    if (node.type === "root" || node.type === "blockquote" || node.type === "list" || node.type === "listItem") {
      for (const child of node.children ?? [])
        stack.push(child);
    }
  }
  return false;
}

interface TableCheckpoint {
  text: string;
  header: string;
  tailStart: number;
  block: StreamingMarkdownBlock;
  table: Extract<MarkdownNode, { type: "table" }>;
}

function tableCheckpoint(text: string, blocks: StreamingMarkdownBlock[], tree: Root): TableCheckpoint | null {
  const sourceTable = tree.children.length === 1 ? tree.children[0] : undefined;
  const block = blocks.length === 1 ? blocks[0] : undefined;
  const table = block?.document?.nodes.length === 1 ? block.document.nodes[0] : undefined;
  if (sourceTable?.type !== "table" || table?.type !== "table" || !block
    || block.document?.references.length || sourceTable.children.length < 2) {
    return null;
  }
  const bodyStart = sourceTable.children[1]?.position?.start.offset;
  const tailStart = sourceTable.children[sourceTable.children.length - 1]?.position?.start.offset;
  if (bodyStart === undefined || tailStart === undefined)
    return null;
  return { text, header: text.slice(0, bodyStart), tailStart, block, table };
}

/**
 * One projector per worker. In a single GFM table, completed physical rows
 * cannot be changed by appending to the last row. Reparse the header and that
 * mutable row, not thousands of committed rows. Any change of document shape,
 * replacement, or Future-reference context falls back to the full parser.
 */
export function createStreamingMarkdownProjector() {
  let previous: TableCheckpoint | null = null;
  let latest: { text: string; blocks: StreamingMarkdownBlock[] } | null = null;
  return (text: string, live: boolean): StreamingMarkdownBlock[] => {
    // Retain only the current version, not every intermediate live document in
    // the shared 512-entry cache. Finalization needs flags, not another parse.
    if (latest?.text === text) {
      const last = latest.blocks.length - 1;
      const blocks = latest.blocks.map((block, index) => {
        const nextLive = live && index === last;
        return block.live === nextLive ? block : { ...block, live: nextLive };
      });
      latest = { text, blocks };
      return blocks;
    }
    const blocks = project(text, live);
    latest = { text, blocks };
    return blocks;
  };

  function project(text: string, live: boolean): StreamingMarkdownBlock[] {
    if (previous && text.startsWith(previous.text)) {
      const fragment = previous.header + text.slice(previous.tailStart);
      const tree = streamingMarkdownProcessor.parse(fragment) as Root;
      const sourceTable = tree.children.length === 1 ? tree.children[0] : undefined;
      if (sourceTable?.type === "table" && sourceTable.children.length > 1) {
        const document = parseFutureMarkdown(fragment, tree, { cache: !live });
        const table = document.nodes[0];
        const lastRowOffset = sourceTable.children[sourceTable.children.length - 1]?.position?.start.offset;
        if (document.nodes.length === 1 && table?.type === "table"
          && document.references.length === 0 && lastRowOffset !== undefined) {
          const merged = { ...table, rows: [...previous.table.rows.slice(0, -1), ...table.rows] };
          const block: StreamingMarkdownBlock = {
            content: text,
            live,
            start: 0,
            parsed: true,
            document: { ...document, raw: text, nodes: [merged] },
          };
          previous = {
            text,
            header: previous.header,
            tailStart: previous.tailStart + lastRowOffset - previous.header.length,
            block,
            table: merged,
          };
          return [block];
        }
      }
    }
    if (!text) {
      previous = null;
      return [];
    }
    const tree = streamingMarkdownProcessor.parse(text) as Root;
    const blocks = projectTree(text, live, tree);
    previous = tableCheckpoint(text, blocks, tree);
    return blocks;
  }
}

/** Cheap and lossless provisional rendering, including worker failure recovery. */
export function plainStreamingMarkdown(text: string, live: boolean): StreamingMarkdownBlock[] {
  return text
    ? [{
        content: text,
        live,
        start: 0,
        document: {
          raw: text,
          references: [],
          nodes: [{ type: "paragraph", children: [{ type: "text", text }] }],
        },
      }]
    : [];
}

interface StreamingMarkdownRoot {
  children: Array<{
    position?: { start?: { offset?: number } };
    type: string;
  }>;
}

/**
 * Desktop-only projection for the growing Markdown renderer. Completed source
 * blocks stay immutable while the final block remains mutable. Reference-style
 * definitions can affect any block, so those documents remain whole.
 */
export function splitStreamingMarkdown(raw: string, live: boolean): StreamingMarkdownBlock[] {
  if (!raw)
    return [];

  const tree = streamingMarkdownProcessor.parse(raw) as unknown as StreamingMarkdownRoot;
  return splitTree(raw, live, tree);
}

function splitTree(raw: string, live: boolean, tree: StreamingMarkdownRoot): StreamingMarkdownBlock[] {
  if (tree.children.some(node => node.type === "definition"))
    return [{ content: raw, live, start: 0 }];

  const starts = tree.children
    .map(node => node.position?.start?.offset)
    .filter((offset): offset is number => typeof offset === "number");
  if (starts.length <= 1)
    return [{ content: raw, live, start: 0 }];

  const blocks: StreamingMarkdownBlock[] = [];
  for (let index = 0; index < starts.length; index++) {
    const start = index === 0 ? 0 : starts[index]!;
    const end = starts[index + 1] ?? raw.length;
    if (end <= start)
      /* v8 ignore next -- remark emits strictly increasing, in-range top-level
         offsets, so a non-positive-length block never materializes */
      continue;
    blocks.push({
      content: raw.slice(start, end),
      live: live && index === starts.length - 1,
      start,
    });
  }
  return blocks.length > 0 ? blocks : [{ content: raw, live, start: 0 }];
}
