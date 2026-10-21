import { useSyncExternalStore } from "react";

/**
 * How much of a settled reply the transcript shows.
 *
 * - `default` keeps a finished reply to the answer it ends on: the work that
 *   produced it — reasoning, tool calls and the notes the model wrote between
 *   them — folds into one collapsed "show process" row above the answer.
 * - `detailed` leaves the whole reply in place, work and answer interleaved in
 *   the order it happened.
 *
 * Either way the work is shown while the reply streams (the model is talking,
 * and hiding the live activity would leave an empty row), and a reader on
 * `default` can always open the folded part to see what it hid.
 *
 * A display preference, not backend state: like the theme and the text size it
 * is persisted in `localStorage` and read before the first React paint.
 */
export const REPLY_DETAIL_LEVELS = ["default", "detailed"] as const;
export type ReplyDetail = (typeof REPLY_DETAIL_LEVELS)[number];

export const DEFAULT_REPLY_DETAIL: ReplyDetail = "default";

/**
 * The level's own key. The setting used to be a boolean under another key, and
 * that value is deliberately not read: a stored `false` there cannot be told
 * apart from one an older default wrote, and the level this ships with is what
 * an untouched reader gets anyway.
 */
const STORAGE_KEY = "future.reply.detail";

function isReplyDetail(value: string | null): value is ReplyDetail {
  return (REPLY_DETAIL_LEVELS as readonly string[]).includes(value ?? "");
}

function readStored(): ReplyDetail {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (isReplyDetail(stored))
      return stored;
  }
  catch {
    // localStorage may be unavailable; fall through to the default.
  }
  return DEFAULT_REPLY_DETAIL;
}

let detail: ReplyDetail = readStored();
const listeners = new Set<() => void>();

function emitChange(): void {
  for (const listener of listeners) {
    listener();
  }
}

export function getReplyDetail(): ReplyDetail {
  return detail;
}

export function setReplyDetail(next: ReplyDetail): void {
  detail = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  }
  catch {
    // Persistence is best-effort; the change still applies for this session.
  }
  emitChange();
}

export function subscribeReplyDetail(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The level the reader picked. */
export function useReplyDetail(): ReplyDetail {
  return useSyncExternalStore(subscribeReplyDetail, getReplyDetail, () => DEFAULT_REPLY_DETAIL);
}

/** Whether a settled reply is folded onto its answer, i.e. the `default` level. */
export function foldsSettledReply(detail: ReplyDetail): boolean {
  return detail === "default";
}
