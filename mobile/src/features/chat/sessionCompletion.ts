import { buildSessionReference } from "@future-os/markdown";
import type { SessionReferenceMap, SessionReferenceTarget } from "../../remote/types";
import type { SkillQuery, TextSelection } from "./skillCompletion";

/**
 * The `#` session-reference trigger at the caret. The same caret rules as
 * `skillQuery` (`/`), for the other reference sigil: `#` must open a standalone
 * token, so `issue#12` and a `##` heading stay text instead of opening a menu,
 * and the token is one word (titles match on their first word or, more usefully,
 * on the workspace name the conversation is filed under).
 */
export function sessionQuery(text: string, selection: TextSelection): SkillQuery | null {
  const { start: cursor, end } = selection;
  if (cursor !== end || cursor < 0 || cursor > text.length) return null;
  const before = text.slice(0, cursor);
  const match = /(?:^|\s)(#[\p{L}\p{N}_.-]*)$/u.exec(before);
  if (!match) return null;
  const start = cursor - match[1]!.length;
  const tail = /^\S*/u.exec(text.slice(cursor))![0];
  const token = text.slice(start, cursor) + tail;
  if (!/^#[\p{L}\p{N}_.-]*$/u.test(token)) return null;
  return { start, end: cursor + tail.length, query: text.slice(start + 1, cursor) };
}

/**
 * The compact token a picked conversation stands for in the draft: `#title`.
 *
 * A phone keyboard cannot render the desktop's pill, so the draft shows this
 * instead of the whole markdown link, and the send expands it back (see
 * `expandSessionReferences`) using the id the map remembers. When the token is
 * already taken by a *different* conversation — two conversations sharing a
 * title, or the same one picked twice — the id's tail is appended so the two
 * tokens stay distinct; without it the second pick would silently expand to the
 * first.
 *
 * The token is display text, not identity: `taken`/the map carry the id, so
 * whitespace is collapsed to keep it on one line and a blank title falls back to
 * the id's tail instead of a bare `#`. The *label* the message carries is the
 * conversation's own title, untouched (see `buildSessionReference`).
 */
export function sessionReferenceToken(
  target: SessionReferenceTarget,
  taken: SessionReferenceMap = {},
): string {
  const label = target.title.trim().replace(/\s+/g, " ") || target.sessionId.slice(-5);
  const base = `#${label}`;
  if (!taken[base] || taken[base]!.sessionId === target.sessionId) return base;
  return `${base}\u00b7${target.sessionId.slice(-5)}`;
}

/**
 * Replace the typed `#query` with that compact token. The link the agent reads
 * is written at send time, not here.
 */
export function completeSessionReference(
  text: string,
  query: SkillQuery,
  target: SessionReferenceTarget,
  taken: SessionReferenceMap = {},
) {
  const token = sessionReferenceToken(target, taken);
  const suffix = text.slice(query.end);
  const separator = suffix.startsWith(" ") ? "" : " ";
  const cursor = query.start + token.length + 1;
  return {
    text: text.slice(0, query.start) + token + separator + suffix,
    selection: { start: cursor, end: cursor },
    token,
  };
}

/**
 * Expand every remembered token back into the link the message must carry:
 * `[title](futureos://session/<id>)`. Longest token first, so a token that is a
 * prefix of another (`#未命名` vs `#未命名·f5eec`) cannot corrupt it. Tokens the
 * user edited away are simply absent from the text and never expand.
 */
export function expandSessionReferences(text: string, refs: SessionReferenceMap): string {
  let expanded = text;
  for (const token of Object.keys(refs).sort((left, right) => right.length - left.length)) {
    if (!expanded.includes(token)) continue;
    expanded = expanded.split(token).join(buildSessionReference(refs[token]!));
  }
  return expanded;
}

/**
 * Drop references whose token no longer appears in the draft (the user deleted
 * them, or the message was sent and the draft cleared). Returns the same object
 * when nothing changed, so it can drive a state update without looping.
 */
export function pruneSessionReferences(
  refs: SessionReferenceMap,
  text: string,
): SessionReferenceMap {
  const kept = Object.entries(refs).filter(([token]) => text.includes(token));
  if (kept.length === Object.keys(refs).length) return refs;
  return Object.fromEntries(kept);
}
