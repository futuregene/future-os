import AsyncStorage from "@react-native-async-storage/async-storage";
import { createAsyncOperationQueue } from "./asyncOperationQueue";

export interface PendingContinuation {
  version: 2;
  commandId: string;
  pairId: string;
  expectedDesktopId: string;
  sessionId: string;
  sourceRunId: string;
  createdAt: number;
}

const KEY = "futureos.remote.pending-continuation.v1";
const enqueueOperation = createAsyncOperationQueue();

async function loadPendingContinuationDirect(): Promise<PendingContinuation | null> {
  try {
    const raw = await AsyncStorage.getItem(KEY);
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

export function loadPendingContinuation(): Promise<PendingContinuation | null> {
  return enqueueOperation(loadPendingContinuationDirect);
}

export async function savePendingContinuation(continuation: PendingContinuation): Promise<void> {
  await enqueueOperation(() => AsyncStorage.setItem(KEY, JSON.stringify(continuation)));
}

/** Clear only the operation this caller completed; a newer retry must survive. */
export async function clearPendingContinuation(commandId: string): Promise<void> {
  await enqueueOperation(async () => {
    const current = await loadPendingContinuationDirect();
    if (current?.commandId === commandId) await AsyncStorage.removeItem(KEY);
  });
}

/** Drop any continuation, including legacy/malformed records that cannot be decoded safely. */
export async function discardPendingContinuation(): Promise<void> {
  await enqueueOperation(() => AsyncStorage.removeItem(KEY));
}
