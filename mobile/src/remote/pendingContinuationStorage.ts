import AsyncStorage from "@react-native-async-storage/async-storage";
import { createAsyncOperationQueue } from "./asyncOperationQueue";

export interface PendingContinuation {
  version: 2;
  commandId: string;
  bridgeInstanceId?: string;
  pairId: string;
  expectedDesktopId: string;
  sessionId: string;
  sourceRunId: string;
  createdAt: number;
}

const KEY = "futureos.remote.pending-continuation.v1";
const enqueueOperation = createAsyncOperationQueue();

function storageKey(pairId?: string): string {
  return pairId ? `${KEY}.${pairId}` : KEY;
}

async function loadPendingContinuationDirect(pairId?: string): Promise<PendingContinuation | null> {
  try {
    const raw = await AsyncStorage.getItem(storageKey(pairId));
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<PendingContinuation>;
    if (
      value.version !== 2 ||
      typeof value.commandId !== "string" ||
      !value.commandId ||
      typeof value.pairId !== "string" ||
      !value.pairId ||
      typeof value.expectedDesktopId !== "string" ||
      !value.expectedDesktopId ||
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

export function loadPendingContinuation(pairId?: string): Promise<PendingContinuation | null> {
  return enqueueOperation(async () => {
    const current = await loadPendingContinuationDirect(pairId);
    if (current || !pairId) return current;
    const legacy = await loadPendingContinuationDirect();
    if (legacy?.pairId !== pairId) return null;
    await AsyncStorage.setItem(storageKey(pairId), JSON.stringify(legacy));
    await AsyncStorage.removeItem(KEY);
    return legacy;
  });
}

export async function savePendingContinuation(continuation: PendingContinuation, pairId?: string): Promise<void> {
  await enqueueOperation(() => AsyncStorage.setItem(storageKey(pairId), JSON.stringify(continuation)));
}

/** Clear only the operation this caller completed; a newer retry must survive. */
export async function clearPendingContinuation(commandId: string, pairId?: string): Promise<void> {
  await enqueueOperation(async () => {
    const current = await loadPendingContinuationDirect(pairId);
    if (current?.commandId === commandId) await AsyncStorage.removeItem(storageKey(pairId));
  });
}

/** Drop only this pair's continuation, including its legacy record. */
export async function discardPendingContinuation(pairId?: string): Promise<void> {
  await enqueueOperation(async () => {
    await AsyncStorage.removeItem(storageKey(pairId));
    if (pairId && (await loadPendingContinuationDirect())?.pairId === pairId)
      await AsyncStorage.removeItem(KEY);
  });
}
