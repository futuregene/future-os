/**
 * Whitelist converter from raw HTML blocks to typed `MarkdownNode`s.
 *
 * CommonMark hands us each raw HTML block as a single opaque `html` node (a
 * string). The default parser drops those to inert literal text — which is why
 * a README that uses GitHub-flavored HTML (`<p align="center">`, `<h3
 * align="center">`, `<img width=…>`) shows its raw tags instead of rendering.
 *
 * This module parses such a block with `parse5` and maps a narrow whitelist of
 * *presentation and structure* elements onto the existing node model. It is a
 * whitelist, not an HTML engine: anything outside the whitelist (other tags,
 * attribute values that would inject behavior, script/handler-bearing markup)
 * is either flattened to plain text or dropped, so hostile HTML cannot reach
 * the renderer. No element here can carry an event handler or a script:
 * parse5 stores attributes opaquely and we only ever read the few names below.
 *
 * The whitelist is exactly the surface a GitHub README uses:
 *   <p align> / <div> / <section>  → paragraph (+ align)
 *   <h1>..<h6> [align]             → heading (+ align)
 *   <img src alt width>            → image (+ width)
 *   <a href>                       → link
 *   <br>                           → break
 *   <strong>/<b>, <em>/<i>,
 *   <del>/<s>/<strike>             → strong / italic / delete
 *   <code>                         → inline code
 *   <details>/<summary>            → their text, flattened (collapsible UI
 *                                     is out of scope; the content stays)
 *   <span>                         → its text
 *   <!-- comment -->               → nothing
 */
import { parseFragment } from "parse5";

import type { InlineNode, MarkdownNode } from "./types";

interface HtmlAttrs {
  [name: string]: string | null;
}

interface HtmlNode {
  readonly nodeName: string;
  readonly tagName?: string | null;
  readonly value?: string;
  readonly attrs?: ReadonlyArray<{ readonly name: string; readonly value: string | null }>;
  readonly childNodes?: ReadonlyArray<HtmlNode>;
}

const HEADING_LEVEL: Record<string, 1 | 2 | 3 | 4 | 5 | 6> = {
  h1: 1, h2: 2, h3: 3, h4: 4, h5: 5, h6: 6,
};

/**
 * The only tags this converter understands. Anything else (a `<script>`,
 * `<iframe>`, an unknown tag, …) must not be restructured — the block is kept
 * verbatim as inert text so it stays readable but never becomes executable
 * markup. A fragment is "safe" when every element in it is whitelisted.
 */
const SAFE_TAGS = new Set([
  "p", "div", "section",
  "h1", "h2", "h3", "h4", "h5", "h6",
  "img", "a", "br",
  "strong", "b", "em", "i", "del", "s", "strike", "code",
  "details", "summary", "span",
]);

function isSafeFragment(root: { readonly childNodes?: ReadonlyArray<HtmlNode> }): boolean {
  const walk = (node: HtmlNode): boolean => {
    if (!isElement(node)) return true;
    if (!SAFE_TAGS.has(node.tagName as string)) return false;
    return (node.childNodes ?? []).every(walk);
  };
  return (root.childNodes ?? []).every(walk);
}

function readAttrs(node: HtmlNode): HtmlAttrs {
  const out: HtmlAttrs = {};
  for (const attr of node.attrs ?? []) {
    // First occurrence wins: a duplicated attribute is ignored, not overrid.
    if (!(attr.name in out)) out[attr.name] = attr.value;
  }
  return out;
}

function isElement(node: HtmlNode): boolean {
  // parse5 exposes real elements through `tagName`; text/comment fragments
  // only have `nodeName` ("#text").
  return typeof node.tagName === "string" && node.tagName.length > 0;
}

function textOf(node: HtmlNode): string {
  let out = "";
  for (const child of node.childNodes ?? []) {
    if (!isElement(child)) {
      if (typeof child.value === "string") out += child.value;
    } else {
      out += textOf(child);
    }
  }
  return out;
}

function alignOf(attrs: HtmlAttrs): "center" | "left" | "right" | undefined {
  const value = attrs.align;
  return value === "center" || value === "left" || value === "right"
    ? value
    : undefined;
}

/** `width="600"` → 600; a non-positive / non-numeric value is dropped. */
function widthOf(attrs: HtmlAttrs): number | undefined {
  const raw = attrs.width;
  if (typeof raw !== "string") return undefined;
  const value = Number(raw);
  return Number.isFinite(value) && value > 0 ? value : undefined;
}

/** Collapse the whitespace a browser folds (a multi-line source block). */
function collapseWhitespace(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

/**
 * Convert one inline phrasing node (a child of a paragraph/heading/link) into
 * typed inline nodes. Returns `null` for whitespace-only or unrecognized
 * markup; unrecognized *inline* markup is dropped rather than surfaced as text
 * because its meaning is undefined here (a bare `<foo>hi</foo>` is not a
 * GitHub-README construct).
 */
function inlineNode(node: HtmlNode): InlineNode | null {
  if (!isElement(node)) {
    if (typeof node.value !== "string") return null;
    const text = collapseWhitespace(node.value);
    return text ? { type: "text", text } : null;
  }

  const name = node.tagName as string;
  const children = childrenToInline(node);

  switch (name) {
    case "br":
      return { type: "break" };
    case "img": {
      const attrs = readAttrs(node);
      return {
        type: "image",
        src: attrs.src ?? "",
        alt: attrs.alt ?? "",
        width: widthOf(attrs),
      };
    }
    case "a":
      return {
        type: "link",
        href: readAttrs(node).href ?? "",
        children,
      };
    case "strong":
    case "b":
      return { type: "strong", children };
    case "em":
    case "i":
      return { type: "italic", children };
    case "del":
    case "s":
    case "strike":
      return { type: "delete", children };
    case "code":
      return { type: "code", code: textOf(node) };
    case "details":
    case "summary":
    case "span":
      return { type: "text", text: textOf(node) };
    default:
      // Headings and unknown markup are block-level; a heading cannot appear
      // as inline phrasing, so drop it here (the block path handles headings).
      return null;
  }
}

function childrenToInline(node: HtmlNode): InlineNode[] {
  return (node.childNodes ?? []).map(inlineNode).filter(
    (child): child is InlineNode => child !== null,
  );
}

/**
 * Convert one raw HTML block into block-level nodes. The block's direct
 * children are mapped to blocks (headings, paragraphs, `<details>`/
 * `<summary>`); other direct children (a bare `<a>`, `<img>`, `<strong>`) are
 * treated as inline content and gathered into a surrounding paragraph.
 */
export function htmlBlockToNodes(raw: string): MarkdownNode[] {
  const trimmed = raw.trim();
  if (!trimmed) return [];

  let fragment;
  try {
    fragment = parseFragment(trimmed);
  } catch {
    // parseFragment throws on truly malformed input; fall back to inert text.
    return [{ type: "paragraph", children: [{ type: "text", text: trimmed }] }];
  }

  // A block that names a non-whitelisted tag is not restructured: keep it
  // verbatim as inert text (readable, never executable). Comments, which carry
  // no elements, are always safe and below.
  if (!isSafeFragment(fragment)) {
    return [{ type: "paragraph", children: [{ type: "text", text: collapseWhitespace(trimmed) }] }];
  }

  const blocks: MarkdownNode[] = [];
  const pendingInline: InlineNode[] = [];
  const flushInline = () => {
    if (pendingInline.length === 0) return;
    // Copy the array: the same reference is reused and cleared on each flush.
    blocks.push({ type: "paragraph", children: [...pendingInline] });
    pendingInline.length = 0;
  };

  const fragmentChildren: HtmlNode[] = ((fragment.childNodes as HtmlNode[] | null) ?? []);
  for (const node of fragmentChildren) {
    if (!isElement(node)) {
      if (typeof node.value === "string") {
        const text = collapseWhitespace(node.value);
        if (text) pendingInline.push({ type: "text", text });
      }
      continue;
    }

    const name = node.tagName as string;
    const attrs = readAttrs(node);
    const level = HEADING_LEVEL[name];
    if (level) {
      flushInline();
      blocks.push({
        type: "heading",
        level,
        align: alignOf(attrs),
        children: childrenToInline(node),
      });
      continue;
    }
    if (name === "p" || name === "div" || name === "section") {
      flushInline();
      blocks.push({
        type: "paragraph",
        align: alignOf(attrs),
        children: childrenToInline(node),
      });
      continue;
    }
    // `<details>` opens a collapsible in a browser; we flatten its summary
    // text so the content survives without the interactive wrapper. A closing
    // `</details>` tag (a lone HTML block) yields nothing.
    if (name === "details" || name === "summary") {
      flushInline();
      const text = collapseWhitespace(textOf(node));
      if (text) blocks.push({ type: "paragraph", children: [{ type: "text", text }] });
      continue;
    }
    const inline = inlineNode(node);
    if (inline) pendingInline.push(inline);
  }

  flushInline();
  return blocks;
}
