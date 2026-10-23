/**
 * The messages a conversation has already sent, for `↑`/`↓` in its composer.
 *
 * Codex (`composer.history_search_previous`) and OpenCode (`history_previous`)
 * both put the previous prompt back on an empty box, and a reader who has just
 * sent a one-line correction wants the paragraph they cut it from. The history
 * is **per conversation**, like the draft beside it: the message you retype is
 * the one you wrote here, and another conversation's prompts would be noise.
 *
 * Stored in `sessionStorage` under its own key — never inside the draft, which
 * is cleared on send and would take the history with it. Session-scoped for the
 * same reason the draft is: nothing here is worth persisting across launches.
 *
 * Recall is a **cursor over a frozen list**, not a second buffer. The list is
 * read once when a conversation's composer mounts; a message sent later is
 * pushed to the store and to the cursor's own list, so the walk keeps working
 * without re-reading storage on every key press.
 */

const HISTORY_LIMIT = 50;
const KEY_PREFIX = "composer-history:";

/** The entries stored for a conversation, oldest first. */
export function loadComposerHistory(draftKey: string): string[] {
  try {
    const raw = sessionStorage.getItem(`${KEY_PREFIX}${draftKey}`);
    if (!raw)
      return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed))
      return [];
    return parsed.filter((entry): entry is string => typeof entry === "string" && entry.trim().length > 0);
  }
  catch {
    // Corrupt JSON or storage unavailable — an empty history is not an error.
    return [];
  }
}

function storeComposerHistory(draftKey: string, entries: readonly string[]): void {
  try {
    sessionStorage.setItem(`${KEY_PREFIX}${draftKey}`, JSON.stringify(entries.slice(-HISTORY_LIMIT)));
  }
  catch {
    // Storage full/unavailable — recall simply does not survive the visit.
  }
}

/**
 * Remember a sent message.
 *
 * Consecutive duplicates collapse: re-sending the same text (a retry after a
 * delivery failure) must not need two ↑ presses to get past itself.
 */
export function pushComposerHistory(draftKey: string, text: string): string[] {
  const trimmed = text.trim();
  if (!trimmed)
    return loadComposerHistory(draftKey);
  const entries = loadComposerHistory(draftKey);
  if (entries[entries.length - 1] === trimmed)
    return entries;
  const next = [...entries, trimmed].slice(-HISTORY_LIMIT);
  storeComposerHistory(draftKey, next);
  return next;
}

/** Walking backwards through what this conversation already sent. */
export interface ComposerHistoryCursor {
  /**
   * The previous message, or `null` when there is nothing older — the caller
   * leaves the key to the editor then.
   */
  previous: () => string | null;
  /** The next (newer) message; `""` means "past the newest — empty the box". */
  next: () => string | null;
  /** Forget the walk: the reader is typing again, or the message was sent. */
  reset: () => void;
  /** Remember a message this composer just sent. */
  push: (text: string) => void;
}

/**
 * A cursor over one conversation's history.
 *
 * `null` means "not recalling yet", which is a different state from "at the
 * newest entry": the first `previous()` starts at the newest message, and
 * walking forward past it lands on an empty box rather than on the draft the
 * reader had before they pressed ↑.
 */
export function createComposerHistoryCursor(draftKey: string | undefined): ComposerHistoryCursor {
  let entries = draftKey ? loadComposerHistory(draftKey) : [];
  let index: number | null = null;
  return {
    next: () => {
      if (entries.length === 0 || index === null)
        return null;
      if (index >= entries.length - 1) {
        index = null;
        return "";
      }
      index += 1;
      return entries[index] ?? null;
    },
    previous: () => {
      if (entries.length === 0)
        return null;
      index = index === null ? entries.length - 1 : Math.max(0, index - 1);
      return entries[index] ?? null;
    },
    push: (text) => {
      if (!draftKey)
        return;
      entries = pushComposerHistory(draftKey, text);
      index = null;
    },
    reset: () => {
      index = null;
    },
  };
}
