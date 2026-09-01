import AsyncStorage from "@react-native-async-storage/async-storage";
import { createAsyncOperationQueue } from "./asyncOperationQueue";
import type { MobileAttachment, ThinkingLevel } from "./types";

export interface PendingPrompt {
  version: 1;
  commandId: string;
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
      value.version !== 1 ||
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

/** Clear only the record this caller completed; a newer send must survive. */
export async function clearPendingPrompt(commandId: string): Promise<void> {
  await enqueueOperation(async () => {
    const current = await loadPendingPromptDirect();
    if (current?.commandId === commandId) await AsyncStorage.removeItem(KEY);
  });
}
