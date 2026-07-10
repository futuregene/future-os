import type { MessageAttachment } from "./agentThreadTypes";

/**
 * Per-conversation composer draft: the *unsent* input a conversation carries
 * between visits — text, `@`-mention pills, and pending attachments. Stored in
 * `sessionStorage`, keyed by conversation, so it is never shared across
 * conversations and is discarded when the app closes (session end).
 *
 * The shape is intentionally open for growth: bump `DRAFT_VERSION` only on a
 * breaking change (a stored draft with a mismatched version is ignored, never
 * migrated). New optional fields can be added without a version bump — older
 * stored drafts simply omit them and readers fall back to defaults.
 */
export interface ComposerDraft {
  /** Schema version; a mismatch makes the stored draft unreadable (discarded). */
  version: number;
  /**
   * Editor content as `getContent()` markdown \u2014 verbatim text with `@` mentions
   * as `[name](./path)`. The same representation that gets sent to the model and
   * that MessageBlock renders, re-hydrated into pills by `MentionEditor.restore`.
   */
  text?: string;
  /** Pending attachments for the next turn. */
  attachments?: MessageAttachment[];
  // Add further per-turn draft fields here (e.g. a future per-turn override).
}

const DRAFT_VERSION = 1;
const KEY_PREFIX = "composer-draft:";

function storageKey(draftKey: string): string {
  return `${KEY_PREFIX}${draftKey}`;
}

/** Read a conversation's draft, or null when absent/unreadable/stale-version. */
export function loadComposerDraft(draftKey: string): ComposerDraft | null {
  try {
    const raw = sessionStorage.getItem(storageKey(draftKey));
    if (!raw)
      return null;
    const parsed = JSON.parse(raw) as ComposerDraft;
    if (!parsed || typeof parsed !== "object" || parsed.version !== DRAFT_VERSION)
      return null;
    return parsed;
  }
  catch {
    // Corrupt JSON or sessionStorage unavailable — treat as no draft.
    return null;
  }
}

/**
 * Persist a conversation's draft. An empty draft (no text/pills and no
 * attachments) clears the slot instead of writing a blank entry, so a
 * composer that was emptied leaves nothing behind.
 */
export function saveComposerDraft(draftKey: string, draft: Omit<ComposerDraft, "version">): void {
  const hasAttachments = (draft.attachments?.length ?? 0) > 0;
  if (!hasAttachments && (draft.text ?? "").trim().length === 0) {
    clearComposerDraft(draftKey);
    return;
  }
  try {
    sessionStorage.setItem(storageKey(draftKey), JSON.stringify({ version: DRAFT_VERSION, ...draft }));
  }
  catch {
    // sessionStorage full/unavailable — a dropped draft is non-fatal.
  }
}

/** Remove a conversation's draft (e.g. after its message is sent). */
export function clearComposerDraft(draftKey: string): void {
  try {
    sessionStorage.removeItem(storageKey(draftKey));
  }
  catch {
    // Ignore — storage unavailable.
  }
}
