import type { FutureMarkdownDocument, InlineNode, ListItemNode, MarkdownNode } from "./types";

/**
 * CommonMark soft breaks — the newlines inside a paragraph that are not hard
 * breaks (`\` or two trailing spaces, which mdast turns into `{type: "break"}`)
 * — render as a space. Source wrapped at a fixed column (a `.md` file, or a
 * reply written through an editor) must reflow to the reader's width instead;
 * keeping them as breaks leaves the ragged short lines of the source on screen.
 *
 * mdast keeps those newlines inside `text` values, which React Native paints as
 * hard breaks and HTML paints as hard breaks under `whitespace-pre-wrap`. Both
 * ends render chat bubbles that way on purpose (a single newline there is the
 * author's line break), so the document surfaces — file preview, artifact
 * preview — opt in to the CommonMark reading through this function.
 *
 * The input document is returned unchanged when it holds no soft break, so
 * memoized renderers keep their identity. Table cells need no treatment: a GFM
 * row comes from a single source line, so a cell cannot carry a soft break.
 */
export function joinSoftBreaks(document: FutureMarkdownDocument): FutureMarkdownDocument {
  const nodes = joinBlocks(document.nodes);
  return nodes === document.nodes ? document : { ...document, nodes };
}

/** Whitespace around the newline collapses with it: HTML folds `a\n   b` to
 * `a b`, while React Native would paint the source indentation verbatim. */
const SOFT_BREAK = /\s*\n\s*/g;

function joinBlocks(nodes: MarkdownNode[]): MarkdownNode[] {
  let changed = false;
  const joined = nodes.map(node => {
    const next = joinBlock(node);
    if (next !== node) changed = true;
    return next;
  });
  return changed ? joined : nodes;
}

function joinBlock(node: MarkdownNode): MarkdownNode {
  switch (node.type) {
    case "paragraph":
    case "heading": {
      const children = joinInline(node.children);
      return children === node.children ? node : { ...node, children };
    }
    case "blockquote": {
      const children = joinBlocks(node.children);
      return children === node.children ? node : { ...node, children };
    }
    case "list": {
      let changed = false;
      const items = node.items.map(item => {
        const next = joinListItem(item);
        if (next !== item) changed = true;
        return next;
      });
      return changed ? { ...node, items } : node;
    }
    default:
      return node;
  }
}

function joinListItem(item: ListItemNode): ListItemNode {
  const children = joinInline(item.children);
  const blocks = item.blocks ? joinBlocks(item.blocks) : undefined;
  if (children === item.children && (!item.blocks || blocks === item.blocks)) return item;
  return { ...item, children, blocks };
}

function joinInline(nodes: InlineNode[]): InlineNode[] {
  let changed = false;
  const joined = nodes.map(node => {
    const next = joinInlineNode(node);
    if (next !== node) changed = true;
    return next;
  });
  return changed ? joined : nodes;
}

function joinInlineNode(node: InlineNode): InlineNode {
  switch (node.type) {
    case "text": {
      const text = node.text.replace(SOFT_BREAK, " ");
      return text === node.text ? node : { ...node, text };
    }
    case "strong":
    case "italic":
    case "delete": {
      const children = joinInline(node.children);
      return children === node.children ? node : { ...node, children };
    }
    case "link": {
      const children = joinInline(node.children);
      return children === node.children ? node : { ...node, children };
    }
    case "futureReference": {
      if (!node.children) return node;
      const children = joinInline(node.children);
      return children === node.children ? node : { ...node, children };
    }
    default:
      return node;
  }
}
