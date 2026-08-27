import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { MAX_PROMPT_MESSAGE_BYTES, randomId, utf8Bytes } from "./codec";
import { isTransientNatsRequestError, type RemoteClient } from "./client";
import { clearSessionDraftIfMatches } from "./draftStorage";
import { uploadAttachments } from "./files";
import {
  clearPendingContinuation,
  discardPendingContinuation,
  loadPendingContinuation,
  savePendingContinuation,
  type PendingContinuation,
} from "./pendingContinuationStorage";
import {
  clearPendingPrompt,
  loadPendingPrompt,
  savePendingPrompt,
  type PendingPrompt,
} from "./pendingPromptStorage";
import { commitAcknowledgedUserMessage, emptyTimeline } from "./timeline";
import type { SyncEngine, ReconcileReason } from "./syncEngine";
import { modelProviderFromReference } from "./types";
import type {
  ConnectionPhase,
  MobileAttachment,
  PromptAck,
  RemoteCredentials,
  ThinkingLevel,
} from "./types";

function samePendingPrompt(
  pending: PendingPrompt,
  candidate: Omit<PendingPrompt, "version" | "commandId" | "createdAt">,
): boolean {
  const attachmentKey = (items: MobileAttachment[]) =>
    items.map(item => `${item.localUri}\u0000${item.name}\u0000${item.transferSize}`).sort();
  return (
    pending.draftKey === candidate.draftKey &&
    pending.sessionId === candidate.sessionId &&
    pending.text.trim() === candidate.text.trim() &&
    pending.modelId === candidate.modelId &&
    pending.thinkingLevel === candidate.thinkingLevel &&
    pending.mode === candidate.mode &&
    pending.workspaceId === candidate.workspaceId &&
    JSON.stringify(attachmentKey(pending.attachments)) ===
      JSON.stringify(attachmentKey(candidate.attachments))
  );
}

async function pendingPromptReceipt(
  client: RemoteClient,
  commandId: string,
): Promise<PromptAck | null> {
  return (
    await client.requestRetry<PromptAck | null>(
      { type: "get_prompt_receipt", promptId: commandId },
      "list",
    )
  ).data;
}

async function deliverPendingPrompt(
  client: RemoteClient,
  pending: PendingPrompt,
  checkReceipt: boolean,
  receiptSupported: boolean,
  onUploadProgress?: (completedBytes: number, totalBytes: number) => void,
): Promise<PromptAck> {
  if (checkReceipt && receiptSupported) {
    const receipt = await pendingPromptReceipt(client, pending.commandId);
    if (receipt) return receipt;
  }
  const uploaded = await uploadAttachments(client, pending.attachments, onUploadProgress);
  return (
    await client.requestRetry<PromptAck>(
      {
        id: pending.commandId,
        type: "prompt",
        sessionId: pending.sessionId,
        message: pending.text.trim(),
        modelId: pending.modelId,
        providerId: modelProviderFromReference(pending.modelId),
        level: pending.thinkingLevel,
        ...(uploaded.length
          ? { attachments: uploaded.map(attachment => ({ uploadId: attachment.uploadId! })) }
          : {}),
        ...(pending.mode === "workspace"
          ? { mode: "workspace", workspaceId: pending.workspaceId }
          : {}),
      },
      pending.sessionId,
    )
  ).data;
}

async function deliverPendingContinuation(
  client: RemoteClient,
  pending: PendingContinuation,
  checkReceipt: boolean,
  receiptSupported: boolean,
): Promise<PromptAck> {
  if (checkReceipt && receiptSupported) {
    const receipt = await pendingPromptReceipt(client, pending.commandId);
    if (receipt) return receipt;
  }
  return (
    await client.requestRetry<PromptAck>(
      {
        id: pending.commandId,
        type: "continue_run",
        sessionId: pending.sessionId,
        runId: pending.sourceRunId,
      },
      pending.sessionId,
    )
  ).data;
}

function continuationMatchesCredentials(
  pending: PendingContinuation,
  credentials: RemoteCredentials,
): boolean {
  return (
    pending.pairId === credentials.pairId &&
    pending.expectedDesktopId === credentials.expectedDesktopId
  );
}

interface PromptOutboxOptions {
  clientRef: MutableRefObject<RemoteClient | null>;
  credentialsRef: MutableRefObject<RemoteCredentials | null>;
  selectedRef: MutableRefObject<string>;
  streamingRef: MutableRefObject<Record<string, boolean>>;
  conversationEpochRef: MutableRefObject<number>;
  syncEngineRef: MutableRefObject<SyncEngine | null>;
  phase: ConnectionPhase;
  draft: boolean;
  draftMode: "chat" | "workspace";
  draftWorkspaceId: string;
  modelId: string;
  thinkingLevel: ThinkingLevel;
  fileTransferSupported: boolean;
  promptReceiptSupported: boolean;
  setSelectedSessionId: Dispatch<SetStateAction<string>>;
  setDraft: Dispatch<SetStateAction<boolean>>;
  setDraftMode: Dispatch<SetStateAction<"chat" | "workspace">>;
  setDraftWorkspaceId: Dispatch<SetStateAction<string>>;
  refreshSessions(): Promise<void>;
  reconcileSession(sessionId: string | undefined, reason: ReconcileReason, runId?: string): void;
  recordError(error: unknown): void;
}

export function usePromptOutbox({
  clientRef,
  credentialsRef,
  selectedRef,
  streamingRef,
  conversationEpochRef,
  syncEngineRef,
  phase,
  draft,
  draftMode,
  draftWorkspaceId,
  modelId,
  thinkingLevel,
  fileTransferSupported,
  promptReceiptSupported,
  setSelectedSessionId,
  setDraft,
  setDraftMode,
  setDraftWorkspaceId,
  refreshSessions,
  reconcileSession,
  recordError,
}: PromptOutboxOptions) {
  const [sending, setSending] = useState(false);
  const sendingRef = useRef(false);
  const pendingRecoveryRef = useRef<Promise<void> | null>(null);
  const continuationInFlightRef = useRef<{
    sessionId: string;
    sourceRunId: string;
    promise: Promise<void>;
  } | null>(null);

  const sendMessage = useCallback(
    async (
      text: string,
      attachments: MobileAttachment[] = [],
      onUploadProgress?: (completedBytes: number, totalBytes: number) => void,
    ) => {
      const client = clientRef.current;
      if (!text.trim() && attachments.length === 0) return;
      if (!client) throw new Error("not_connected");
      if (sendingRef.current) throw new Error("send_busy");
      if (attachments.length > 0 && !fileTransferSupported) {
        throw new Error("attachment_unsupported_desktop");
      }
      if (utf8Bytes(text) > MAX_PROMPT_MESSAGE_BYTES) throw new Error("prompt_too_large");

      const targetSessionId = selectedRef.current;
      const targetDraft = draft;
      const targetDraftMode = draftMode;
      const targetDraftWorkspaceId = draftWorkspaceId;
      const conversationEpoch = conversationEpochRef.current;
      const engine = syncEngineRef.current;
      const candidate = {
        draftKey: targetSessionId || "draft:new",
        sessionId: targetSessionId,
        text,
        attachments,
        modelId,
        thinkingLevel,
        mode: targetDraft ? targetDraftMode : ("chat" as const),
        workspaceId: targetDraft ? targetDraftWorkspaceId : "",
      };
      if (streamingRef.current[targetSessionId] ?? false) throw new Error("send_streaming");

      let pending = await loadPendingPrompt();
      let checkReceipt = false;
      if (pending && samePendingPrompt(pending, candidate)) {
        checkReceipt = true;
      } else {
        if (pending) {
          const previousReceipt = promptReceiptSupported
            ? await pendingPromptReceipt(client, pending.commandId)
            : null;
          if (previousReceipt) await clearSessionDraftIfMatches(pending.draftKey, pending);
          await clearPendingPrompt(pending.commandId);
        }
        pending = {
          version: 1,
          commandId: randomId("prompt"),
          ...candidate,
          createdAt: Date.now(),
        };
        await savePendingPrompt(pending);
      }

      sendingRef.current = true;
      setSending(true);
      try {
        const response = await deliverPendingPrompt(
          client,
          pending,
          checkReceipt,
          promptReceiptSupported,
          onUploadProgress,
        );
        await clearPendingPrompt(pending.commandId);
        await clearSessionDraftIfMatches(pending.draftKey, pending);
        const nextSessionId = response.sessionId || targetSessionId;
        engine?.mutate(nextSessionId, timeline =>
          commitAcknowledgedUserMessage(timeline ?? emptyTimeline(), {
            id: `local:${pending.commandId}`,
            runId: response.runId,
            text: text.trim(),
            ...(attachments.length
              ? {
                  attachments: attachments.map(attachment => ({
                    path: attachment.localUri,
                    name: attachment.name,
                    kind: attachment.kind,
                    mobilePreviewUnsupported: attachment.mobilePreviewUnsupported,
                  })),
                }
              : {}),
          }),
        );
        if (nextSessionId && nextSessionId !== targetSessionId) {
          const stillViewingSentDraft = conversationEpochRef.current === conversationEpoch;
          if (stillViewingSentDraft) {
            selectedRef.current = nextSessionId;
            setSelectedSessionId(nextSessionId);
            setDraft(false);
            setDraftMode("chat");
            setDraftWorkspaceId("");
          }
          engine?.mutate(targetSessionId, timeline => ({
            ...(timeline ?? emptyTimeline()),
            items: [],
          }));
          if (stillViewingSentDraft) void refreshSessions();
        }
      } catch (sendError) {
        if (!isTransientNatsRequestError(sendError)) {
          await clearPendingPrompt(pending.commandId);
        }
        throw sendError;
      } finally {
        sendingRef.current = false;
        setSending(false);
      }
    },
    [
      clientRef,
      conversationEpochRef,
      draft,
      draftMode,
      draftWorkspaceId,
      fileTransferSupported,
      modelId,
      promptReceiptSupported,
      refreshSessions,
      selectedRef,
      setDraft,
      setDraftMode,
      setDraftWorkspaceId,
      setSelectedSessionId,
      streamingRef,
      syncEngineRef,
      thinkingLevel,
    ],
  );

  const recoverPendingPrompt = useCallback(async () => {
    if (sendingRef.current) return;
    if (pendingRecoveryRef.current) return pendingRecoveryRef.current;
    const recovery = (async () => {
      const client = clientRef.current;
      if (!client || !credentialsRef.current) return;
      const pending = await loadPendingPrompt();
      if (!pending) return;
      sendingRef.current = true;
      setSending(true);
      try {
        const receipt = await deliverPendingPrompt(client, pending, true, promptReceiptSupported);
        await clearPendingPrompt(pending.commandId);
        await clearSessionDraftIfMatches(pending.draftKey, pending);
        void refreshSessions();
        reconcileSession(receipt.sessionId, "reconnect");
      } catch (recoveryError) {
        if (!isTransientNatsRequestError(recoveryError)) {
          await clearPendingPrompt(pending.commandId);
          recordError(recoveryError);
        }
      } finally {
        sendingRef.current = false;
        setSending(false);
      }
    })().finally(() => {
      pendingRecoveryRef.current = null;
    });
    pendingRecoveryRef.current = recovery;
    return recovery;
  }, [
    clientRef,
    credentialsRef,
    promptReceiptSupported,
    reconcileSession,
    recordError,
    refreshSessions,
  ]);

  const continueRun = useCallback(
    async (sessionId: string, runId: string) => {
      const inFlight = continuationInFlightRef.current;
      if (inFlight) {
        if (inFlight.sessionId === sessionId && inFlight.sourceRunId === runId) {
          return inFlight.promise;
        }
        await inFlight.promise;
      }

      const operation = (async () => {
        const client = clientRef.current;
        const credentials = credentialsRef.current;
        if (!client || !credentials) throw new Error("not_connected");
        let pending = await loadPendingContinuation();
        if (pending && !continuationMatchesCredentials(pending, credentials)) {
          await discardPendingContinuation();
          pending = null;
        }
        let checkReceipt = pending !== null;
        if (pending && (pending.sessionId !== sessionId || pending.sourceRunId !== runId)) {
          if (promptReceiptSupported) await pendingPromptReceipt(client, pending.commandId);
          await clearPendingContinuation(pending.commandId);
          pending = null;
          checkReceipt = false;
        }
        if (!pending) {
          pending = {
            version: 2,
            commandId: randomId("continue"),
            pairId: credentials.pairId,
            expectedDesktopId: credentials.expectedDesktopId,
            sessionId,
            sourceRunId: runId,
            createdAt: Date.now(),
          } satisfies PendingContinuation;
          await savePendingContinuation(pending);
        }
        try {
          await deliverPendingContinuation(client, pending, checkReceipt, promptReceiptSupported);
          await clearPendingContinuation(pending.commandId);
        } catch (continueError) {
          if (!isTransientNatsRequestError(continueError)) {
            await clearPendingContinuation(pending.commandId);
          }
          throw continueError;
        }
      })();

      continuationInFlightRef.current = {
        sessionId,
        sourceRunId: runId,
        promise: operation,
      };
      try {
        await operation;
      } finally {
        if (continuationInFlightRef.current?.promise === operation) {
          continuationInFlightRef.current = null;
        }
      }
    },
    [clientRef, credentialsRef, promptReceiptSupported],
  );

  const recoverPendingContinuation = useCallback(async () => {
    const client = clientRef.current;
    const credentials = credentialsRef.current;
    if (!client || !credentials || continuationInFlightRef.current) return;
    const pending = await loadPendingContinuation();
    if (!pending || continuationInFlightRef.current) return;
    if (!continuationMatchesCredentials(pending, credentials)) {
      await discardPendingContinuation();
      return;
    }

    const operation = (async () => {
      try {
        const receipt = await deliverPendingContinuation(
          client,
          pending,
          true,
          promptReceiptSupported,
        );
        await clearPendingContinuation(pending.commandId);
        void refreshSessions();
        reconcileSession(receipt.sessionId || pending.sessionId, "reconnect", receipt.runId);
      } catch (recoveryError) {
        if (!isTransientNatsRequestError(recoveryError)) {
          await clearPendingContinuation(pending.commandId);
          recordError(recoveryError);
        }
      }
    })();

    continuationInFlightRef.current = {
      sessionId: pending.sessionId,
      sourceRunId: pending.sourceRunId,
      promise: operation,
    };
    try {
      await operation;
    } finally {
      if (continuationInFlightRef.current?.promise === operation) {
        continuationInFlightRef.current = null;
      }
    }
  }, [
    clientRef,
    credentialsRef,
    promptReceiptSupported,
    reconcileSession,
    recordError,
    refreshSessions,
  ]);

  useEffect(() => {
    if (phase !== "ready") return;
    void (async () => {
      await recoverPendingPrompt();
      await recoverPendingContinuation();
    })();
  }, [phase, recoverPendingContinuation, recoverPendingPrompt]);

  return { sending, sendMessage, continueRun };
}
