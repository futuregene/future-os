import AsyncStorage from "@react-native-async-storage/async-storage";
import { createAsyncOperationQueue } from "./asyncOperationQueue";
import type { MobileAttachment, ThinkingLevel } from "./types";

export interface PendingPrompt {
  version: 2;
  commandId: string;
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

async function loadPendingPromptDirect(): Promise<PendingPrompt | null> {
  try {
    const raw = await AsyncStorage.getItem(KEY);
    if (!raw) return null;
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
    return null;
  }
}

export function loadPendingPrompt(): Promise<PendingPrompt | null> {
  return enqueueOperation(loadPendingPromptDirect);
}

export async function savePendingPrompt(prompt: PendingPrompt): Promise<void> {
  await enqueueOperation(() => AsyncStorage.setItem(KEY, JSON.stringify(prompt)));
}

/** Drop all pending delivery state on unpair, including legacy records. */
export async function discardPendingPrompt(): Promise<void> {
  await enqueueOperation(() => AsyncStorage.removeItem(KEY));
}

/** Clear only the record this caller completed; a newer send must survive. */
export async function clearPendingPrompt(commandId: string): Promise<void> {
  await enqueueOperation(async () => {
    const current = await loadPendingPromptDirect();
    if (current?.commandId === commandId) await AsyncStorage.removeItem(KEY);
  });
}
