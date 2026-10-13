import type { RemoteUpload } from "./remotePeerClient";
import { continueRemoteRun, getRemotePromptReceipt, promptRemoteConversation } from "./remotePeerClient";

/**
 * Durable remote prompt/continuation outbox.
 *
 * When a prompt or continuation cannot reach the host, it is persisted in
 * localStorage keyed by desktop+session so the user can leave and come back.
 * Recovery checks `get_prompt_receipt` first: if the host already accepted the
 * command, the cached ack is returned without re-running it.
 */

export interface PendingPrompt {
  version: 1;
  commandId: string;
  sessionId: string;
  text: string;
  attachmentIds: string[];
  workspaceId: string | null;
  freshModelId: string | null;
  freshThinkingLevel: string | null;
  createdAt: number;
}

export interface PendingContinuation {
  version: 1;
  commandId: string;
  sessionId: string;
  runId: string;
  createdAt: number;
}

interface PendingOutbox {
  prompt: PendingPrompt | null;
  continuation: PendingContinuation | null;
}

const OUTBOX_KEY_PREFIX = "remote-prompt-outbox:";

function storageKey(desktopId: string, sessionId: string): string {
  return `${OUTBOX_KEY_PREFIX}${desktopId}:${sessionId}`;
}

function load<T>(desktopId: string, sessionId: string): T | null {
  try {
    const raw = localStorage.getItem(storageKey(desktopId, sessionId));
    if (!raw)
      return null;
    return JSON.parse(raw) as T;
  }
  catch {
    return null;
  }
}

function save<T>(desktopId: string, sessionId: string, value: T): void {
  try {
    localStorage.setItem(storageKey(desktopId, sessionId), JSON.stringify(value));
  }
  catch {
    // Ignore storage failures.
  }
}

export function loadPendingOutbox(desktopId: string, sessionId: string): PendingOutbox {
  const raw = load<PendingOutbox>(desktopId, sessionId);
  if (!raw || typeof raw !== "object")
    return { prompt: null, continuation: null };
  return {
    prompt: raw.prompt && raw.prompt.version === 1 ? raw.prompt : null,
    continuation: raw.continuation && raw.continuation.version === 1 ? raw.continuation : null,
  };
}

function savePendingOutbox(desktopId: string, sessionId: string, outbox: PendingOutbox): void {
  save(desktopId, sessionId, outbox);
}

function randomId(prefix: string): string {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  const value = Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
  return `${prefix}_${value}`;
}

export function isRecoverableSendError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return /peer_not_connected|remote_peer_disconnected|transport|timeout|network/i.test(message);
}

export interface PromptResult {
  sessionId: string;
  threadId: string;
  runId?: string;
}

export interface RemotePromptOutbox {
  /** The prompt currently waiting to be delivered, if any. */
  pendingPrompt: PendingPrompt | null;
  /** Queue a prompt for delivery. If the host is reachable it is sent now. */
  sendPrompt: (desktopId: string, sessionId: string, input: {
    text: string;
    attachments: RemoteUpload[];
    workspaceId: string | null;
    freshModelId: string | null;
    freshThinkingLevel: string | null;
  }) => Promise<PromptResult>;
  /** Queue a continuation for delivery. If the host is reachable it is sent now. */
  sendContinuation: (desktopId: string, sessionId: string, runId: string) => Promise<unknown>;
  /** Attempt to deliver any pending item; called after reconnect. */
  recover: (desktopId: string, sessionId: string) => Promise<void>;
  /** Clear a pending prompt after a successful send. */
  clearPrompt: (desktopId: string, sessionId: string) => void;
}

/**
 * In-memory mutable state for the outbox. This module intentionally keeps state
 * here rather than in a hook so that tests and the composer can talk to the
 * same queue without wiring a context provider.
 */
const state = new Map<string, PendingOutbox>();

function key(desktopId: string, sessionId: string): string {
  return `${desktopId}:${sessionId}`;
}

function getState(desktopId: string, sessionId: string): PendingOutbox {
  const k = key(desktopId, sessionId);
  let current = state.get(k);
  if (!current) {
    current = loadPendingOutbox(desktopId, sessionId);
    state.set(k, current);
  }
  return current;
}

function setState(desktopId: string, sessionId: string, outbox: PendingOutbox): void {
  const k = key(desktopId, sessionId);
  state.set(k, outbox);
  savePendingOutbox(desktopId, sessionId, outbox);
}

async function deliverPrompt(desktopId: string, _sessionId: string, pending: PendingPrompt): Promise<PromptResult> {
  // Check receipt first: the host may have already accepted this command.
  const receipt = await getRemotePromptReceipt(desktopId, pending.commandId);
  if (receipt) {
    return receipt;
  }
  const result = await promptRemoteConversation(
    desktopId,
    pending.sessionId,
    pending.text,
    pending.attachmentIds,
    pending.workspaceId,
    { modelId: pending.freshModelId, thinkingLevel: pending.freshThinkingLevel },
    pending.commandId,
  );
  return result;
}

async function deliverContinuation(desktopId: string, _sessionId: string, pending: PendingContinuation): Promise<unknown> {
  const receipt = await getRemotePromptReceipt(desktopId, pending.commandId);
  if (receipt) {
    return receipt;
  }
  return continueRemoteRun(desktopId, pending.sessionId, pending.runId, pending.commandId);
}

/**
 * Deliver a prompt, persisting it first and clearing it on success.
 *
 * If delivery fails with a recoverable error the prompt is left in the outbox
 * and the promise rejects with that error. Non-recoverable errors clear the
 * persisted entry before rejecting.
 */
export async function sendRemotePrompt(
  desktopId: string,
  sessionId: string,
  input: {
    text: string;
    attachments: RemoteUpload[];
    workspaceId: string | null;
    freshModelId: string | null;
    freshThinkingLevel: string | null;
  },
): Promise<PromptResult> {
  const commandId = randomId("prompt");
  const pending: PendingPrompt = {
    version: 1,
    commandId,
    sessionId,
    text: input.text,
    attachmentIds: input.attachments.map(upload => upload.uploadId),
    workspaceId: input.workspaceId,
    freshModelId: input.freshModelId,
    freshThinkingLevel: input.freshThinkingLevel,
    createdAt: Date.now(),
  };
  const outbox = getState(desktopId, sessionId);
  setState(desktopId, sessionId, { ...outbox, prompt: pending });

  try {
    const result = await deliverPrompt(desktopId, sessionId, pending);
    const updated = getState(desktopId, sessionId);
    if (updated.prompt?.commandId === commandId) {
      setState(desktopId, sessionId, { ...updated, prompt: null });
    }
    return result;
  }
  catch (error) {
    if (!isRecoverableSendError(error)) {
      const updated = getState(desktopId, sessionId);
      if (updated.prompt?.commandId === commandId) {
        setState(desktopId, sessionId, { ...updated, prompt: null });
      }
    }
    throw error;
  }
}

export async function sendRemoteContinuation(
  desktopId: string,
  sessionId: string,
  runId: string,
): Promise<unknown> {
  const commandId = randomId("continue");
  const pending: PendingContinuation = {
    version: 1,
    commandId,
    sessionId,
    runId,
    createdAt: Date.now(),
  };
  const outbox = getState(desktopId, sessionId);
  setState(desktopId, sessionId, { ...outbox, continuation: pending });

  try {
    const result = await deliverContinuation(desktopId, sessionId, pending);
    const updated = getState(desktopId, sessionId);
    if (updated.continuation?.commandId === commandId) {
      setState(desktopId, sessionId, { ...updated, continuation: null });
    }
    return result;
  }
  catch (error) {
    if (!isRecoverableSendError(error)) {
      const updated = getState(desktopId, sessionId);
      if (updated.continuation?.commandId === commandId) {
        setState(desktopId, sessionId, { ...updated, continuation: null });
      }
    }
    throw error;
  }
}

export async function recoverRemoteOutbox(desktopId: string, sessionId: string): Promise<void> {
  const outbox = getState(desktopId, sessionId);
  if (!outbox.prompt && !outbox.continuation)
    return;

  if (outbox.prompt) {
    try {
      await deliverPrompt(desktopId, sessionId, outbox.prompt);
      const updated = getState(desktopId, sessionId);
      setState(desktopId, sessionId, { ...updated, prompt: null });
    }
    catch {
      // Leave it for the next recovery attempt.
    }
  }

  if (outbox.continuation) {
    try {
      await deliverContinuation(desktopId, sessionId, outbox.continuation);
      const updated = getState(desktopId, sessionId);
      setState(desktopId, sessionId, { ...updated, continuation: null });
    }
    catch {
      // Leave it for the next recovery attempt.
    }
  }
}

export function getRemoteOutbox(desktopId: string, sessionId: string): PendingOutbox {
  return getState(desktopId, sessionId);
}

/** Reset the in-memory outbox cache. Intended for tests. */
export function resetRemoteOutbox(): void {
  state.clear();
}

export function clearRemoteOutboxPrompt(desktopId: string, sessionId: string): void {
  const outbox = getState(desktopId, sessionId);
  setState(desktopId, sessionId, { ...outbox, prompt: null });
}

export function clearRemoteOutboxContinuation(desktopId: string, sessionId: string): void {
  const outbox = getState(desktopId, sessionId);
  setState(desktopId, sessionId, { ...outbox, continuation: null });
}
