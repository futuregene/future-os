import AsyncStorage from "@react-native-async-storage/async-storage";
import { createAsyncOperationQueue } from "./asyncOperationQueue";
import type { MobileAttachment, ThinkingLevel } from "./types";

export interface PendingPrompt {
  version: 2;
  commandId: string;
  bridgeInstanceId?: string;
  pairId: string;
  expectedDesktopId: string;
  draftKey: string;
  sessionId: string;
  text: string;
  attachments: MobileAttachment[];
  modelId: string;
  thinkingLevel: ThinkingLevel;
  mode: "chat" | "workspace";
  workspaceId: string;
  createdAt: number;
}

const KEY = "futureos.remote.pending-prompt.v1";
const enqueueOperation = createAsyncOperationQueue();

function storageKey(pairId?: string): string {
  return pairId ? `${KEY}.${pairId}` : KEY;
}

async function loadPendingPromptDirect(pairId?: string): Promise<PendingPrompt | null> {
  const raw = await AsyncStorage.getItem(storageKey(pairId));
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<PendingPrompt>;
    if (
      // Legacy records have no safe destination identity. Leave drafts alone,
      // but never automatically replay these records on the current pairing.
      value.version !== 2 ||
      typeof value.pairId !== "string" ||
      !value.pairId ||
      typeof value.expectedDesktopId !== "string" ||
      !value.expectedDesktopId ||
      typeof value.commandId !== "string" ||
      !value.commandId ||
      typeof value.draftKey !== "string" ||
      typeof value.sessionId !== "string" ||
      typeof value.text !== "string" ||
      !Array.isArray(value.attachments) ||
      typeof value.modelId !== "string" ||
      typeof value.thinkingLevel !== "string" ||
      (value.mode !== "chat" && value.mode !== "workspace") ||
      typeof value.workspaceId !== "string" ||
      typeof value.createdAt !== "number"
    ) {
      return null;
    }
    return value as PendingPrompt;
  } catch {
    // Invalid legacy/corrupt data is not replayable. Storage I/O errors occur
    // before this parser and must propagate so callers do not mint a new
    // command id while an older delivery may already have been accepted.
    return null;
  }
}

export function loadPendingPrompt(pairId?: string): Promise<PendingPrompt | null> {
  return enqueueOperation(async () => {
    const current = await loadPendingPromptDirect(pairId);
    if (current || !pairId) return current;
    const legacy = await loadPendingPromptDirect();
    if (legacy?.pairId !== pairId) return null;
    await AsyncStorage.setItem(storageKey(pairId), JSON.stringify(legacy));
    await AsyncStorage.removeItem(KEY);
    return legacy;
  });
}

export async function savePendingPrompt(prompt: PendingPrompt, pairId?: string): Promise<void> {
  await enqueueOperation(() => AsyncStorage.setItem(storageKey(pairId), JSON.stringify(prompt)));
}

/** Drop this pair's pending delivery state without affecting other desktops. */
export async function discardPendingPrompt(pairId?: string): Promise<void> {
  await enqueueOperation(async () => {
    await AsyncStorage.removeItem(storageKey(pairId));
    if (pairId && (await loadPendingPromptDirect())?.pairId === pairId)
      await AsyncStorage.removeItem(KEY);
  });
}

/** Clear only the record this caller completed; a newer send must survive. */
export async function clearPendingPrompt(commandId: string, pairId?: string): Promise<void> {
  await enqueueOperation(async () => {
    const current = await loadPendingPromptDirect(pairId);
    if (current?.commandId === commandId) await AsyncStorage.removeItem(storageKey(pairId));
  });
}
