import { buildSessionReference } from "@future-os/markdown";

export const PILL_ATTR = "data-mention";

/** True when the editor has no text and no pills. */
export function isEditorEmpty(editor: HTMLDivElement | null): boolean {
  if (!editor)
    return true;
  if (editor.querySelector(`[${PILL_ATTR}]`))
    return false;
  return (editor.textContent ?? "").trim().length === 0;
}

/**
 * The active `@` mention at the caret, if any. Reads the caret's text node and
 * matches `@query` at its end — a pill (separate node) naturally bounds it.
 * The `@` may appear anywhere in the text (start, after whitespace, or inside
 * CJK sentences); an ASCII letter/digit or `./@` right before it suppresses the
 * trigger so emails (`foo@bar`) and paths (`./x`, `a/b`) never open the menu.
 */
export function mentionContext(editor: HTMLDivElement | null): {
  query: string;
  textNode: Text;
  atOffset: number;
  caretOffset: number;
} | null {
  const selection = window.getSelection();
  if (!editor || !selection || selection.rangeCount === 0 || !selection.isCollapsed)
    return null;
  const node = selection.anchorNode;
  if (!node || node.nodeType !== Node.TEXT_NODE || !editor.contains(node))
    return null;
  const caretOffset = selection.anchorOffset;
  const before = (node.textContent ?? "").slice(0, caretOffset);
  const match = before.match(/(^|[^\w.@/])@([^\s@]*)$/);
  if (!match)
    return null;
  const query = match[2] ?? "";
  return {
    query,
    textNode: node as Text,
    atOffset: caretOffset - query.length - 1, // index of `@`
    caretOffset,
  };
}

/**
 * The active `/` skill trigger at the caret, if any. Same caret scan as
 * `mentionContext`, but matches `/query`. Like `@`, the `/` may sit anywhere
 * in the text (including mid-sentence in CJK); an ASCII letter/digit or `./@`
 * right before it suppresses the trigger so paths (`./x`, `a/b`) and dates
 * (`2026/07`) never open the skill menu.
 */
export function slashContext(editor: HTMLDivElement | null): {
  query: string;
  textNode: Text;
  slashOffset: number;
  caretOffset: number;
} | null {
  const selection = window.getSelection();
  if (!editor || !selection || selection.rangeCount === 0 || !selection.isCollapsed)
    return null;
  const node = selection.anchorNode;
  if (!node || node.nodeType !== Node.TEXT_NODE || !editor.contains(node))
    return null;
  const caretOffset = selection.anchorOffset;
  const before = (node.textContent ?? "").slice(0, caretOffset);
  const match = before.match(/(^|[^\w.@/])\/([^\s/]*)$/);
  if (!match)
    return null;
  const query = match[2] ?? "";
  return {
    query,
    textNode: node as Text,
    slashOffset: caretOffset - query.length - 1, // index of `/`
    caretOffset,
  };
}

/**
 * The active `#` conversation trigger at the caret, if any. Same caret scan as
 * `slashContext`, but matches `#query`. A preceding word character, `.`, `/`,
 * `@` or a second `#` suppresses the trigger, so `##` markdown headings,
 * `a#b` and `issue#12` stay literal while `#fix the build` opens the menu.
 */
export function hashContext(editor: HTMLDivElement | null): {
  query: string;
  textNode: Text;
  hashOffset: number;
  caretOffset: number;
} | null {
  const selection = window.getSelection();
  if (!editor || !selection || selection.rangeCount === 0 || !selection.isCollapsed)
    return null;
  const node = selection.anchorNode;
  if (!node || node.nodeType !== Node.TEXT_NODE || !editor.contains(node))
    return null;
  const caretOffset = selection.anchorOffset;
  const before = (node.textContent ?? "").slice(0, caretOffset);
  const match = before.match(/(^|[^\w.@/#])#([^\s#]*)$/);
  if (!match)
    return null;
  const query = match[2] ?? "";
  return {
    query,
    textNode: node as Text,
    hashOffset: caretOffset - query.length - 1, // index of `#`
    caretOffset,
  };
}

/** Serialize the editor: text verbatim, pills → markdown links. */
export function serialize(editor: HTMLDivElement | null): string {
  if (!editor)
    return "";
  let out = "";
  const visit = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      out += node.textContent ?? "";
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE)
      return;
    const element = node as HTMLElement;
    const pillKind = element.getAttribute(PILL_ATTR);
    if (pillKind === "skill") {
      out += `/${element.getAttribute("data-skill") ?? ""}`;
      return;
    }
    if (pillKind === "session") {
      // The link form (not a bare `#id`) is the contract every client and the
      // agent read; the session id travels in the destination.
      out += buildSessionReference({
        sessionId: element.getAttribute("data-session") ?? "",
        title: element.getAttribute("data-title") ?? "",
      });
      return;
    }
    if (pillKind) {
      const label = (element.textContent ?? "").replace(/\[/g, "(").replace(/\]/g, ")");
      const path = element.getAttribute("data-path") ?? "";
      // Angle-wrap whenever the path holds whitespace OR parens: a bare `)` in
      // the path closes the markdown link early, truncating downstream parsing
      // (MessageBlock's MENTION_LINK matches the `<...>` form for these).
      out += `[${label}](${/[\s()]/.test(path) ? `<${path}>` : path})`;
      return;
    }
    if (element.tagName === "BR") {
      out += "\n";
      return;
    }
    for (const child of Array.from(element.childNodes))
      visit(child);
    // A browser-inserted block wrapper implies a line break after it.
    if (element.tagName === "DIV" || element.tagName === "P")
      out += "\n";
  };
  for (const child of Array.from(editor.childNodes))
    visit(child);
  return out.replace(/\u200B/g, ""); // strip any stray zero-width spaces
}
