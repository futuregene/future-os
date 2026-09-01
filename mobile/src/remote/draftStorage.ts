import AsyncStorage from "@react-native-async-storage/async-storage";
import { File } from "expo-file-system";
import { createAsyncOperationQueue } from "./asyncOperationQueue";
import type { MobileAttachment } from "./types";

/**
 * Per-session composer draft: the *unsent* input a conversation carries between
 * visits — text and pending attachments. Persisted to AsyncStorage keyed by
 * sessionId, so navigating back to the session list and reopening the session
 * restores what was being composed. Desktop parity (composerDraft.ts): the
 * shape is versioned and unreadable drafts are discarded, never migrated.
 */
export interface SessionDraft {
  version: number;
  text: string;
  attachments: MobileAttachment[];
}

const DRAFT_VERSION = 1;
const KEY_PREFIX = "futureos.remote.draft.v1:";
const enqueueOperation = createAsyncOperationQueue();

function storageKey(sessionId: string): string {
  return `${KEY_PREFIX}${sessionId}`;
}

function attachmentFileExists(localUri: string): boolean {
  try {
    return new File(localUri).exists;
  } catch {
    // File API unavailable (e.g. tests) — keep the attachment rather than
    // dropping a user's pending work we can't verify.
    return true;
  }
}

/**
 * Read a session's draft, or null when absent/unreadable/stale-version.
 * Attachments whose backing file no longer exists (a temporary camera/cache
 * file was pruned) are dropped rather than surfacing a dead tap target.
 */
async function loadSessionDraftDirect(sessionId: string): Promise<SessionDraft | null> {
  if (!sessionId) return null;
  try {
    const raw = await AsyncStorage.getItem(storageKey(sessionId));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as SessionDraft;
    if (!parsed || typeof parsed !== "object" || parsed.version !== DRAFT_VERSION) return null;
    const text = typeof parsed.text === "string" ? parsed.text : "";
    const attachments = Array.isArray(parsed.attachments)
      ? parsed.attachments.filter(
          attachment =>
            attachment &&
            typeof attachment.localUri === "string" &&
            typeof attachment.name === "string" &&
            typeof attachment.mimeType === "string" &&
            (attachment.kind === "image" || attachment.kind === "file") &&
            attachmentFileExists(attachment.localUri),
        )
      : [];
    if (text.trim().length === 0 && attachments.length === 0) return null;
    return { version: DRAFT_VERSION, text, attachments };
  } catch {
    return null;
  }
}

export function loadSessionDraft(sessionId: string): Promise<SessionDraft | null> {
  return enqueueOperation(() => loadSessionDraftDirect(sessionId));
}

/**
 * Persist a session's draft. An empty draft (no text and no attachments) clears
 * the slot instead of writing a blank entry, so a composer that was emptied
 * leaves nothing behind.
 */
export async function saveSessionDraft(
  sessionId: string,
  draft: Omit<SessionDraft, "version">,
): Promise<void> {
  if (!sessionId) return;
  await enqueueOperation(async () => {
    const hasAttachments = (draft.attachments?.length ?? 0) > 0;
    try {
      if (!hasAttachments && (draft.text ?? "").trim().length === 0) {
        await AsyncStorage.removeItem(storageKey(sessionId));
        return;
      }
      await AsyncStorage.setItem(
        storageKey(sessionId),
        JSON.stringify({ version: DRAFT_VERSION, ...draft }),
      );
    } catch {
      // Storage full/unavailable — a dropped draft is non-fatal.
    }
  });
}

/** Remove a session's draft (e.g. after its message is sent). */
export async function clearSessionDraft(sessionId: string): Promise<void> {
  if (!sessionId) return;
  await enqueueOperation(async () => {
    try {
      await AsyncStorage.removeItem(storageKey(sessionId));
    } catch {
      // Ignore — storage unavailable.
    }
  });
}

/**
 * A cold-start outbox acknowledgement may arrive after the user has already
 * edited the composer again. Remove only the exact draft that produced the
 * acknowledged prompt; never erase newer local work.
 */
export async function clearSessionDraftIfMatches(
  sessionId: string,
  expected: Pick<SessionDraft, "text" | "attachments">,
): Promise<void> {
  if (!sessionId) return;
  await enqueueOperation(async () => {
    const current = await loadSessionDraftDirect(sessionId);
    if (!current || current.text.trim() !== expected.text.trim()) return;
    const identities = (items: MobileAttachment[]) =>
      items.map(item => `${item.localUri}\u0000${item.name}\u0000${item.transferSize}`).sort();
    if (
      JSON.stringify(identities(current.attachments)) !==
      JSON.stringify(identities(expected.attachments))
    ) {
      return;
    }
    try {
      await AsyncStorage.removeItem(storageKey(sessionId));
    } catch {
      // Ignore — storage unavailable.
    }
  });
}
