import { useSyncExternalStore } from "react";

/**
 * How a settled reply is read.
 *
 * `finalAnswerOnly` keeps a finished reply to the answer it ends on: the work
 * that produced it — reasoning, tool calls and the notes the model wrote
 * between them — folds into one collapsed "show process" row above the answer.
 * The work is still shown while the reply streams (the model is talking, and
 * hiding the live activity would leave an empty row), and a reader can always
 * open the folded part to see exactly what the setting hid.
 *
 * A display preference, not backend state: like the theme and the text size it
 * is persisted in `localStorage` and read before the first React paint.
 */
const STORAGE_KEY = "future.reply.finalAnswerOnly";

function readStored(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === "true";
  }
  catch {
    // localStorage may be unavailable; fall through to the default.
    return false;
  }
}

let finalAnswerOnly = readStored();
const listeners = new Set<() => void>();

function emitChange(): void {
  for (const listener of listeners) {
    listener();
  }
}

export function getFinalAnswerOnly(): boolean {
  return finalAnswerOnly;
}

export function setFinalAnswerOnly(next: boolean): void {
  finalAnswerOnly = next;
  try {
    localStorage.setItem(STORAGE_KEY, String(next));
  }
  catch {
    // Persistence is best-effort; the change still applies for this session.
  }
  emitChange();
}

export function subscribeFinalAnswerOnly(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Whether settled replies show only their final answer. */
export function useFinalAnswerOnly(): boolean {
  return useSyncExternalStore(subscribeFinalAnswerOnly, getFinalAnswerOnly, () => false);
}
