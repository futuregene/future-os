import type { Root } from "mdast";
import type { FutureMarkdownDocument, MarkdownNode } from "./types";
import { collectReferences, exceedsNestingLimit, parseFutureMarkdown, parseMdast } from "./parseFutureMarkdown";

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
  // Only a one-to-one source/node mapping is safe to checkpoint. Definitions
  // are handled before conversion; filtered nodes cannot establish a boundary.
  if (fragment.nodes.length !== tree.children.length) return null;
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

// Definitions (including those inside containers) can resolve references in
// already frozen blocks. Inspect the parsed suffix, not a '[' heuristic: inline
// links/images/task lists are common and do not invalidate the stable prefix.
function hasDefinitions(tree: Root): boolean {
  const stack: { type: string; children?: { type: string }[] }[] = [tree];
  while (stack.length) {
    const node = stack.pop()!;
    if (node.type === "definition" || node.type === "footnoteDefinition") return true;
    if (node.children) for (const child of node.children) stack.push(child);
  }
  return false;
}

/** One bounded checkpoint per rendered message/segment. Appends preserve stable
 * block/row objects and parse only the mutable suffix. Definitions invalidate
 * the prefix; replacement and finalization use the canonical parser.
 * Transient fragments bypass the shared settled-document LRU.
 */
export function createStreamingMarkdownParser() {
  let previous: Snapshot | null = null;
  return (text: string, streaming: boolean): FutureMarkdownDocument => {
    if (!streaming) {
      previous = null;
      return parseFutureMarkdown(text);
    }
    const append = previous && text.startsWith(previous.text);
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
          if (node?.type === "table" && lastRow !== undefined) {
            const merged: Table = {
              ...table.node, rows: [...table.node.rows.slice(0, -1), ...node.rows],
            };
            const nodes = [...previous.prefix, merged];
            const document = { raw: text, nodes, references:
              previous.document.references.length || fragment.references.length ? collectReferences(nodes) : [] };
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
    if (exceedsNestingLimit(tree) || (raw.includes("[") && hasDefinitions(tree))) {
      previous = null;
      return parseFutureMarkdown(text, offset === 0 ? tree : undefined, false);
    }
    const fragment = parseFutureMarkdown(raw, tree, false);
    const nodes = [...prefix, ...fragment.nodes];
    const document = offset === 0 ? fragment : {
      raw: text, nodes, references:
        previous?.document.references.length || fragment.references.length ? collectReferences(nodes) : [],
    };
    previous = checkpoint(text, document, fragment, tree, offset);
    return document;
  };
}
