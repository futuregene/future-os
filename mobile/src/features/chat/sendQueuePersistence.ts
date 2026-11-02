import type { QueuedSend } from "../../lib/sendQueue";
import type { MobileAttachment } from "../../remote/types";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { File } from "expo-file-system";

/**
 * The phone's send queue, kept across app launches.
 *
 * A queued message is something the reader wrote and asked to send, so losing it
 * because the OS reclaimed the process (a reboot, an app left in the background
 * for a day, a swipe out of the task switcher) is losing their words. It lives in
 * AsyncStorage next to the composer draft and the pending-prompt record.
 *
 * What it cannot survive is the app being *uninstalled* — that takes every local
 * store with it, this one included. Only a queue held by the desktop could outlive
 * that, and that is a different product decision (the host would send on its own
 * after the run, with no last look from the phone).
 *
 * Attachments are pruned on the way back in: a cache file the OS cleaned up
 * cannot be sent, and an item whose words and files are both gone is not a
 * message. The check is conservative — anything it cannot answer stays.
 */

/** Bumped when the stored shape changes; older records are ignored, not migrated. */
const VERSION = 1;

const KEY = "futureos.remote.send-queue.v1";

/** One stored row: the queue's own fields, no runtime state. */
interface StoredItem {
  attachments: MobileAttachment[];
  /** The command id a recoverable failure left this message under, if any. */
  commandId?: string;
  content: string;
  createdAt: number;
  id: string;
}

interface StoredQueue {
  version: number;
  conversations: Record<string, StoredItem[]>;
}

/** Serialises writes: the last state written must be the last state kept. */
let writes: Promise<void> = Promise.resolve();

function isAttachment(value: unknown): value is MobileAttachment {
  if (!value || typeof value !== "object")
    return false;
  const attachment = value as Partial<MobileAttachment>;
  return typeof attachment.localUri === "string"
    && typeof attachment.name === "string"
    && typeof attachment.mimeType === "string"
    && (attachment.kind === "image" || attachment.kind === "file")
    && typeof attachment.originalSize === "number"
    && typeof attachment.transferSize === "number";
}

function isStoredItem(value: unknown): value is StoredItem {
  if (!value || typeof value !== "object")
    return false;
  const item = value as Partial<StoredItem>;
  return typeof item.id === "string"
    && item.id.length > 0
    && typeof item.content === "string"
    && typeof item.createdAt === "number"
    && Array.isArray(item.attachments)
    && item.attachments.every(isAttachment);
}

/**
 * Whether the file an attachment points at is still there.
 *
 * Unknown answers keep the attachment: a queue that silently drops a file because
 * a probe failed is worse than one whose send fails visibly later.
 */
function attachmentStillThere(attachment: MobileAttachment): boolean {
  try {
    return new File(attachment.localUri).exists;
  }
  catch {
    return true;
  }
}

/** Drop what cannot travel, and the items left with nothing to say. */
function pruneItem(item: StoredItem): StoredItem | null {
  const attachments = item.attachments.filter(attachmentStillThere);
  if (!item.content.trim() && attachments.length === 0)
    return null;
  return { ...item, attachments };
}

export async function loadSendQueue(): Promise<Map<string, QueuedSend<MobileAttachment>[]>> {
  const queued = new Map<string, QueuedSend<MobileAttachment>[]>();
  let raw: string | null = null;
  try {
    raw = await AsyncStorage.getItem(KEY);
  }
  catch {
    // An unreadable store is the same as an empty one: the reader's live queue
    // (if any) is what the session already holds.
    return queued;
  }
  if (!raw)
    return queued;
  let stored: Partial<StoredQueue>;
  try {
    stored = JSON.parse(raw) as Partial<StoredQueue>;
  }
  catch {
    return queued;
  }
  if (stored.version !== VERSION || !stored.conversations || typeof stored.conversations !== "object")
    return queued;
  for (const [conversationKey, items] of Object.entries(stored.conversations)) {
    if (!conversationKey || !Array.isArray(items))
      continue;
    const restored: QueuedSend<MobileAttachment>[] = [];
    for (const item of items) {
      if (!isStoredItem(item))
        continue;
      const pruned = pruneItem(item);
      if (!pruned)
        continue;
      restored.push({
        attachments: pruned.attachments,
        // A record written before this field existed has no command id, and the
        // next attempt mints one; that is the same path a freshly typed row takes.
        ...(typeof pruned.commandId === "string" && pruned.commandId
          ? { commandId: pruned.commandId }
          : {}),
        content: pruned.content,
        createdAt: pruned.createdAt,
        id: pruned.id,
      });
    }
    if (restored.length > 0)
      queued.set(conversationKey, restored);
  }
  return queued;
}

export function saveSendQueue(conversations: Map<string, readonly QueuedSend<MobileAttachment>[]>): Promise<void> {
  const record: StoredQueue = { conversations: {}, version: VERSION };
  for (const [conversationKey, items] of conversations) {
    if (items.length === 0)
      continue;
    record.conversations[conversationKey] = items.map(item => ({
      attachments: [...item.attachments],
      ...(item.commandId ? { commandId: item.commandId } : {}),
      content: item.content,
      createdAt: item.createdAt,
      id: item.id,
    }));
  }
  // A store failure must not break the send path: the queue is still in memory,
  // and the next change writes again.
  writes = writes
    .then(() => AsyncStorage.setItem(KEY, JSON.stringify(record)))
    .catch(() => {});
  return writes;
}
