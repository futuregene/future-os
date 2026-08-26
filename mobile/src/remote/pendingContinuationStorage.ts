import AsyncStorage from "@react-native-async-storage/async-storage";

export interface PendingContinuation {
  version: 1;
  commandId: string;
  sessionId: string;
  sourceRunId: string;
  createdAt: number;
}

const KEY = "futureos.remote.pending-continuation.v1";

export async function loadPendingContinuation(): Promise<PendingContinuation | null> {
  try {
    const raw = await AsyncStorage.getItem(KEY);
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<PendingContinuation>;
    if (
      value.version !== 1 ||
      typeof value.commandId !== "string" ||
      !value.commandId ||
      typeof value.sessionId !== "string" ||
      !value.sessionId ||
      typeof value.sourceRunId !== "string" ||
      !value.sourceRunId ||
      typeof value.createdAt !== "number"
    ) {
      return null;
    }
    return value as PendingContinuation;
  } catch {
    return null;
  }
}

export async function savePendingContinuation(continuation: PendingContinuation): Promise<void> {
  await AsyncStorage.setItem(KEY, JSON.stringify(continuation));
}

/** Clear only the operation this caller completed; a newer retry must survive. */
export async function clearPendingContinuation(commandId: string): Promise<void> {
  const current = await loadPendingContinuation();
  if (current?.commandId === commandId) await AsyncStorage.removeItem(KEY);
}
