import type { RemoteSessionRow } from "./mergeConversations";
import { buildSessionReference } from "@future-os/markdown";

/**
 * The `#` conversation reference a remote composer can write.
 *
 * The phone and this app's own composer both let a message point at another
 * conversation, and the message carries that pointer as a markdown link the
 * agent resolves (`[title](futureos://session/<id>)`). The remote composer is a
 * plain `<textarea>`, so it cannot hold the rich pill the local editor builds;
 * it shows the phone's compact token instead — `#title` in the draft — and
 * expands it back into the link at send time with the id the map remembers.
 * The two ends therefore send the same message, which is the point.
 *
 * Pure: the trigger at the caret, the list the menu offers, the token a pick
 * leaves behind, and the expansion the send applies. The menu that draws the
 * list is `RemoteSessionMenu`, and this file is separate so it holds no
 * elements (the fast-refresh rule, and the reason `remoteCommands` is its own
 * module too).
 */

/** One selectable conversation in the composer's `#` menu. */
export interface RemoteReferenceOption {
  /** Agent session id — the payload a reference carries. */
  sessionId: string;
  title: string;
  /**
   * The workspace it is filed under on that host; null for a chat.
   *
   * Shown on the row rather than filing rows under headings: the list's one
   * order is recency, and a heading would cut that order into stretches that
   * each restart at the top. The name is still worth a row — a query matches it
   * (see `filterRemoteReferenceOptions`), and a match whose reason is invisible
   * reads as random.
   */
  workspace: string | null;
}

/**
 * The references a draft holds, keyed by the token that stands in for each one.
 *
 * Kept until send (and with the draft in storage) because the token alone does
 * not carry the id: losing the map would turn a reference back into plain text.
 */
export type RemoteReferenceMap = Record<string, { sessionId: string; title: string }>;

/**
 * The conversations the `#` menu offers, newest activity first.
 *
 * The host's own catalogue is the source, so a conversation this client has
 * never opened is still referenceable. A chat is a chat by its `mode`, never by
 * its workspace — every chat on a host is filed under a workspace named after
 * itself, and naming that here would label a chat with its own title. The
 * conversation being composed in is dropped (a reference to itself is not a
 * reference), and so is a row without a session id: that id is the whole
 * payload.
 *
 * "Activity" is the host's `COALESCE(last_message_at, updated_at)`, which the
 * catalogue carries as `lastMessageAt`; a host that predates the field sends
 * none, and the conversation's start time stands in — such a row sorts below
 * the ones that can be dated, never as though it were just used.
 */
export function remoteReferenceOptions(
  sessions: RemoteSessionRow[],
  currentSessionId?: string | null,
): RemoteReferenceOption[] {
  return sessions
    .filter(session => session.sessionId !== "" && session.sessionId !== currentSessionId)
    .map(session => ({
      option: toReferenceOption(session),
      at: session.lastMessageAt ?? session.createdAt ?? Number.NEGATIVE_INFINITY,
    }))
    .sort((left, right) => right.at - left.at
      || compareText(left.option.sessionId, right.option.sessionId))
    .map(({ option }) => option);
}

function toReferenceOption(session: RemoteSessionRow): RemoteReferenceOption {
  if (session.mode !== "workspace")
    return { sessionId: session.sessionId, title: session.title, workspace: null };
  return {
    sessionId: session.sessionId,
    title: session.title,
    workspace: session.workspaceName ?? "",
  };
}

/** Narrow the menu to conversations whose title or workspace matches `query`. */
export function filterRemoteReferenceOptions(
  options: RemoteReferenceOption[],
  query: string,
): RemoteReferenceOption[] {
  const search = query.trim().toLocaleLowerCase();
  if (!search)
    return options;
  return options.filter(option =>
    option.title.toLocaleLowerCase().includes(search)
    || (option.workspace ?? "").toLocaleLowerCase().includes(search));
}

/** Where a draft's `#` word sits, from the caret. */
export interface SessionReferenceQuery {
  /** Index of the `#`. */
  start: number;
  /** End of the word being replaced (past the caret when one continues there). */
  end: number;
  /** What follows the `#`; empty for a bare `#`. */
  query: string;
}

/**
 * The `#` trigger at the caret, or null.
 *
 * The phone's rule, for the same sigil: only a `#` that begins a word counts, so
 * `issue#12` and a `##` heading stay text instead of opening a menu over the
 * message being written. The token is one word — titles match on their first
 * word, or, more usefully, on the workspace the conversation is filed under.
 *
 * The caret has to be known: a textarea has no notion of "the word being
 * typed", and a `#` earlier in the draft must not describe what is at the
 * caret. `caret` is null when the selection is not collapsed, which is also the
 * case where nothing should open.
 */
export function sessionReferenceQuery(
  draft: string,
  caret: number | null,
): SessionReferenceQuery | null {
  if (caret === null || caret < 0 || caret > draft.length)
    return null;
  const before = draft.slice(0, caret);
  const match = /(?:^|\s)(#[\p{L}\p{N}_.-]*)$/u.exec(before);
  if (!match)
    return null;
  const start = caret - match[1]!.length;
  const tail = /^\S*/u.exec(draft.slice(caret))![0];
  // A caret in the middle of a longer word: what is being typed is that word,
  // not a reference, and replacing only its head would eat the rest.
  if (!/^#[\p{L}\p{N}_.-]*$/u.test(match[1]! + tail))
    return null;
  return { start, end: caret + tail.length, query: draft.slice(start + 1, caret) };
}

/**
 * The compact token a picked conversation stands for: `#title`.
 *
 * A textarea cannot render the local editor's pill, so the draft shows this and
 * the send expands it back. When the token is already taken by a *different*
 * conversation — two conversations sharing a title, or the same one picked
 * twice — the id's tail is appended so the two stay distinct; without it the
 * second pick would silently expand to the first.
 *
 * The token is display text, not identity: the map carries the id, so
 * whitespace is collapsed to keep it on one line and a blank title falls back to
 * the id's tail instead of a bare `#`.
 */
export function remoteReferenceToken(
  target: { sessionId: string; title: string },
  taken: RemoteReferenceMap = {},
): string {
  const label = target.title.trim().replace(/\s+/g, " ") || target.sessionId.slice(-5);
  const base = `#${label}`;
  if (!taken[base] || taken[base]!.sessionId === target.sessionId)
    return base;
  return `${base}\u00B7${target.sessionId.slice(-5)}`;
}

/**
 * Replace the typed `#query` with that compact token, and say where the caret
 * goes. The link the agent reads is written at send time, not here.
 */
export function completeRemoteReference(
  draft: string,
  query: SessionReferenceQuery,
  target: { sessionId: string; title: string },
  taken: RemoteReferenceMap = {},
): { text: string; caret: number; token: string } {
  const token = remoteReferenceToken(target, taken);
  const suffix = draft.slice(query.end);
  const separator = suffix.startsWith(" ") ? "" : " ";
  return {
    text: draft.slice(0, query.start) + token + separator + suffix,
    caret: query.start + token.length + 1,
    token,
  };
}

/**
 * Expand every remembered token into the link the message must carry:
 * `[title](futureos://session/<id>)`. Longest token first, so a token that is a
 * prefix of another cannot corrupt it. Tokens the user edited away are simply
 * absent from the text and never expand.
 */
export function expandRemoteReferences(text: string, refs: RemoteReferenceMap): string {
  let expanded = text;
  for (const token of Object.keys(refs).sort((left, right) => right.length - left.length)) {
    if (!expanded.includes(token))
      continue;
    expanded = expanded.split(token).join(buildSessionReference(refs[token]!));
  }
  return expanded;
}

/**
 * Drop references whose token no longer appears in the draft (deleted by the
 * user, or the message was sent and the draft cleared). Returns the same object
 * when nothing changed, so it can drive a state update without looping.
 */
export function pruneRemoteReferences(
  refs: RemoteReferenceMap,
  text: string,
): RemoteReferenceMap {
  const kept = Object.entries(refs).filter(([token]) => text.includes(token));
  if (kept.length === Object.keys(refs).length)
    return refs;
  return Object.fromEntries(kept);
}

/** A total order on text; `localeCompare` is locale-dependent and not stable. */
function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}
