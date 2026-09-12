import type { Root } from "mdast";
import type { FutureMarkdownDocument, MarkdownNode } from "./types";
import { exceedsNestingLimit, parseFutureMarkdown, parseMdast } from "./parseFutureMarkdown";

type Table = Extract<MarkdownNode, { type: "table" }>;
interface Snapshot {
  text: string;
  document: FutureMarkdownDocument;
  prefix: MarkdownNode[];
  tailStart: number;
  table?: { header: string; lastRowStart: number; node: Table };
}

function checkpoint(
  text: string,
  document: FutureMarkdownDocument,
  fragment: FutureMarkdownDocument,
  tree: Root,
  offset: number,
): Snapshot | null {
  // Only a one-to-one source/node mapping is safe to checkpoint. Definitions,
  // filtered HTML, embedded references and the nesting guard use full parsing.
  if (document.references.length || fragment.nodes.length !== tree.children.length) return null;
  const last = tree.children[tree.children.length - 1];
  const start = last?.position?.start.offset;
  if (start === undefined) return null;
  const snapshot: Snapshot = {
    text, document, prefix: document.nodes.slice(0, -1), tailStart: offset + start,
  };
  const node = document.nodes[document.nodes.length - 1];
  if (last?.type === "table" && node?.type === "table" && last.children.length > 1) {
    const bodyStart = last.children[1]?.position?.start.offset;
    const rowStart = last.children[last.children.length - 1]?.position?.start.offset;
    if (bodyStart !== undefined && rowStart !== undefined) {
      snapshot.table = {
        header: text.slice(snapshot.tailStart, offset + bodyStart),
        lastRowStart: offset + rowStart,
        node,
      };
    }
  }
  return snapshot;
}

/** One bounded checkpoint per rendered message/segment. Appends preserve stable
 * block/row objects and parse only the mutable suffix. Bracket-bearing source
 * conservatively falls back: even a definition appended much later can resolve
 * an earlier reference. Replacement and finalization use the canonical parser.
 * Transient fragments bypass the shared settled-document LRU.
 */
export function createStreamingMarkdownParser() {
  let previous: Snapshot | null = null;
  return (text: string, streaming: boolean): FutureMarkdownDocument => {
    if (!streaming) {
      previous = null;
      return parseFutureMarkdown(text);
    }
    const append = previous && text.startsWith(previous.text) && !text.includes("[");
    if (append && previous) {
      if (text === previous.text) return previous.document;
      const table = previous.table;
      if (table) {
        const raw = table.header + text.slice(table.lastRowStart);
        const tree = parseMdast(raw);
        const source = tree.children.length === 1 ? tree.children[0] : undefined;
        if (source?.type === "table" && source.children.length > 1) {
          const fragment = parseFutureMarkdown(raw, tree, false);
          const node = fragment.nodes.length === 1 ? fragment.nodes[0] : undefined;
          const lastRow = source.children[source.children.length - 1]?.position?.start.offset;
          if (node?.type === "table" && !fragment.references.length && lastRow !== undefined) {
            const merged: Table = {
              ...table.node, rows: [...table.node.rows.slice(0, -1), ...node.rows],
            };
            const document = { raw: text, nodes: [...previous.prefix, merged], references: [] };
            previous = {
              ...previous, text, document,
              table: { header: table.header, lastRowStart: table.lastRowStart + lastRow - table.header.length, node: merged },
            };
            return document;
          }
        }
      }
    }
    const offset = append && previous ? previous.tailStart : 0;
    const prefix = append && previous ? previous.prefix : [];
    const raw = text.slice(offset);
    const tree = parseMdast(raw);
    if (exceedsNestingLimit(tree)) {
      previous = null;
      return parseFutureMarkdown(text, offset === 0 ? tree : undefined, false);
    }
    const fragment = parseFutureMarkdown(raw, tree, false);
    const document = offset === 0 ? fragment : {
      raw: text, nodes: [...prefix, ...fragment.nodes], references: fragment.references,
    };
    previous = text.includes("[") ? null : checkpoint(text, document, fragment, tree, offset);
    return document;
  };
}
