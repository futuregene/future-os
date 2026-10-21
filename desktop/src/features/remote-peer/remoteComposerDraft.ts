import type { RemoteUpload } from "./remotePeerClient";
import type { RemoteReferenceMap } from "./remoteSessionReference";

/**
 * Per-conversation remote-composer draft: the *unsent* input a remote
 * conversation carries between visits — text, pending uploads, and the
 * conversations its `#` tokens point at. Stored in `localStorage`, keyed by
 * both desktop and session, so it survives app restarts and is scoped to a
 * specific host+conversation pair.
 *
 * The reference map is part of the draft because the token alone does not
 * carry the id: reopening a draft with `#Plan` in it and no map would leave
 * plain text where a link was meant (the phone keeps its map the same way).
 *
 * The shape is intentionally open for growth: bump `DRAFT_VERSION` only on a
 * breaking change (a stored draft with a mismatched version is ignored, never
 * migrated). New optional fields can be added without a version bump.
 */
export interface RemoteComposerDraft {
  /** Schema version; a mismatch makes the stored draft unreadable (discarded). */
  version: number;
  /** Editor text. */
  text?: string;
  /** Pending uploads waiting to be attached to the next message. */
  uploads?: RemoteUpload[];
  /** The `#` tokens in `text` and the conversation each one stands for. */
  references?: RemoteReferenceMap;
}

const DRAFT_VERSION = 1;
const KEY_PREFIX = "remote-composer-draft:";

function storageKey(desktopId: string, sessionId: string): string {
  return `${KEY_PREFIX}${desktopId}:${sessionId || "draft:new"}`;
}

/** Read a conversation's remote draft, or null when absent/unreadable/stale. */
export function loadRemoteComposerDraft(desktopId: string, sessionId: string): RemoteComposerDraft | null {
  try {
    const raw = localStorage.getItem(storageKey(desktopId, sessionId));
    if (!raw)
      return null;
    const parsed = JSON.parse(raw) as RemoteComposerDraft;
    if (!parsed || typeof parsed !== "object" || parsed.version !== DRAFT_VERSION)
      return null;
    return parsed;
  }
  catch {
    return null;
  }
}

/**
 * Persist a remote conversation's draft. An empty draft (no text and no
 * uploads) clears the slot instead of writing a blank entry.
 */
export function saveRemoteComposerDraft(
  desktopId: string,
  sessionId: string,
  draft: Omit<RemoteComposerDraft, "version">,
): void {
  const hasUploads = (draft.uploads?.length ?? 0) > 0;
  if (!hasUploads && (draft.text ?? "").trim().length === 0) {
    clearRemoteComposerDraft(desktopId, sessionId);
    return;
  }
  try {
    localStorage.setItem(
      storageKey(desktopId, sessionId),
      JSON.stringify({ version: DRAFT_VERSION, ...draft }),
    );
  }
  catch {
    // localStorage full/unavailable — a dropped draft is non-fatal.
  }
}

/** Remove a remote conversation's draft (e.g. after its message is sent). */
export function clearRemoteComposerDraft(desktopId: string, sessionId: string): void {
  try {
    localStorage.removeItem(storageKey(desktopId, sessionId));
  }
  catch {
    // Ignore — storage unavailable.
  }
}
