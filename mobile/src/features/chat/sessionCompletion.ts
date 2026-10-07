import { buildSessionReference } from "@future-os/markdown";
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
 * Replace the typed `#query` with the reference link the agent understands —
 * `[title](futureos://session/<id>)`. The id is the payload: the main
 * conversation reads or messages that session through it.
 */
export function completeSessionReference(
  text: string,
  query: SkillQuery,
  session: { sessionId: string; title: string },
) {
  const replacement = buildSessionReference(session);
  const suffix = text.slice(query.end);
  const separator = suffix.startsWith(" ") ? "" : " ";
  const cursor = query.start + replacement.length + 1;
  return {
    text: text.slice(0, query.start) + replacement + separator + suffix,
    selection: { start: cursor, end: cursor },
  };
}
