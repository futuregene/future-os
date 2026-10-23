/**
 * The title a conversation is shown by when it has no name of its own.
 *
 * An unnamed conversation is named after its first user message, and that
 * message is markdown *source*: a composer writes a reference to another
 * conversation as `[手机截图镜像修复](futureos://session/<id>)`, and a first
 * message beginning with one used to name its conversation
 * `[手机截图镜像修复](futureos://sessio…` — brackets, a URL, no topic. A title
 * is a place for text, not for drawing markdown, and the Desktop mints the
 * placeholder title itself (before any Agent session exists) from the raw
 * composer text.
 *
 * The rule mirrors `plain_title_text` in `packages/rpc/src/title.rs`, which is
 * what the Agent, the host's prompt path and the importer apply. The two
 * implementations must agree: the same message must not name a conversation one
 * way on the desktop and another way on the phone.
 */

/** The URL prefix that marks a link as a reference to another conversation. */
const SESSION_REFERENCE_URL = "futureos://session/";

interface InlineLink {
  /** Index just past the closing `)`. */
  end: number;
  label: string;
  target: string;
}

/**
 * Plain text of a user message, for a title or list preview derived from it.
 *
 * Two transformations, in order:
 *
 * 1. A reference to another conversation is *addressing*, not prose, so it
 *    contributes nothing. Its label is the other conversation's title, and
 *    keeping it named this conversation after that one — two identical rows in
 *    one list, where the reader cannot tell which one they are looking at.
 * 2. Every other link contributes its label and an image its alt text, so the
 *    title reads as the sentence that was written.
 *
 * A message that is *only* a reference keeps the label after all: naming a
 * conversation after the one it points at beats naming it `[`. Whitespace is
 * collapsed, because a title is one line whatever the message's line breaks
 * were and a dropped link must not leave the spaces that framed it behind.
 */
export function plainTitleText(text: string): string {
  const withoutReferences = renderTitleText(text, true);
  if (withoutReferences) return withoutReferences;
  // Nothing but a reference (or nothing at all): the labels are all we have.
  return renderTitleText(text, false);
}

/** `dropReferences` decides what a conversation reference contributes: its label, or nothing. */
function renderTitleText(text: string, dropReferences: boolean): string {
  let rendered = "";
  let copied = 0;
  let index = 0;
  while (index < text.length) {
    const link = inlineLinkAt(text, index);
    if (!link) {
      index += 1;
      continue;
    }
    rendered += text.slice(copied, index);
    if (!(dropReferences && isConversationReference(link.target)))
      rendered += link.label;
    index = link.end;
    copied = index;
  }
  rendered += text.slice(copied);
  return rendered.replace(/\s+/g, " ").trim();
}

/** Whether a link target addresses another conversation in this app. */
function isConversationReference(target: string): boolean {
  // Both spellings of a target are in the wild: `[a](x)` and `[a](<x>)`.
  const trimmed = target.trim();
  const bare = trimmed.startsWith("<") && trimmed.endsWith(">")
    ? trimmed.slice(1, -1)
    : trimmed;
  return bare.startsWith(SESSION_REFERENCE_URL);
}

/**
 * An inline markdown image (`![alt](target)`) or link (`[label](target)`)
 * starting at `from`, or null when no *complete* link starts there — text that
 * merely contains brackets is left exactly as it was, because a title may not
 * silently eat it.
 */
function inlineLinkAt(text: string, from: number): InlineLink | null {
  let start = from;
  if (text[start] === "!") start += 1;
  if (text[start] !== "[") return null;
  const labelStart = start + 1;
  const labelEnd = text.indexOf("]", labelStart);
  if (labelEnd < 0) return null;
  if (text[labelEnd + 1] !== "(") return null;
  // Targets can carry parentheses of their own (`[x](file(1).md)`), so the
  // link ends at the `)` that closes the one that opened it.
  const targetStart = labelEnd + 2;
  let depth = 1;
  let end = targetStart;
  for (; end < text.length; end += 1) {
    if (text[end] === "(") depth += 1;
    else if (text[end] === ")") {
      depth -= 1;
      if (depth === 0) break;
    }
  }
  if (depth !== 0) return null; // unterminated — not a link
  return {
    end: end + 1,
    label: text.slice(labelStart, labelEnd),
    target: text.slice(targetStart, end),
  };
}
