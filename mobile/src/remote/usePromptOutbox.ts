import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { MAX_PROMPT_MESSAGE_BYTES, randomId, utf8Bytes } from "./codec";
import { isDeferredRequestError, isTransientNatsRequestError, type RemoteClient } from "./client";
import { clearSessionDraftIfMatches } from "./draftStorage";
import { desktopDraftKey } from "./desktopDraftKey";
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
import { commitAcknowledgedUserMessage, emptyTimeline } from "@future-os/remote-client/timeline";
import type { SyncEngine, ReconcileReason } from "@future-os/remote-client/syncEngine";
import { modelProviderFromReference } from "./types";
import type {
  ConnectionPhase,
  MobileAttachment,
  PromptAck,
  RemoteCredentials,
  ThinkingLevel,
} from "./types";

/**
 * Whether a failed delivery is one the link recovers from.
 *
 * These are the failures that keep the durable record on purpose: the message is
 * not refused, it has not arrived yet, and the outbox delivers it when the link is
 * back. Anything else is a refusal — the desktop will not take this message — and
 * the record goes.
 */
export function isRecoverableDeliveryError(error: unknown): boolean {
  return isDeferredRequestError(error) || isTransientNatsRequestError(error);
}

/**
 * The command id a failed send left its durable record under, when the failure is
 * one the link recovers from.
 *
 * Such a failure keeps the record (`sendMessage` deliberately does not clear it),
 * so the message is delivered — by the outbox, on the next readiness edge — under
 * that id. A caller that is going to try the same message again has to reuse it:
 * the desktop looks a prompt up by command id alone, and a second id is a second
 * prompt in a second run, charged twice. Null means there is no record to reuse —
 * the send was refused, or it never got as far as storing one.
 */
export function deliveryCommandId(error: unknown): string | null {
  const value = (error as { commandId?: unknown } | null | undefined)?.commandId;
  return typeof value === "string" && value ? value : null;
}

function withDeliveryCommandId(error: unknown, commandId: string): unknown {
  if (error && typeof error === "object") {
    try {
      (error as { commandId?: string }).commandId = commandId;
    }
    catch {
      // A frozen error still reaches the caller; it just cannot carry the id, and
      // the next attempt mints a fresh one — the behaviour from before this fix.
    }
  }
  return error;
}

async function clearAcknowledgedPrompt(commandId: string, pairId: string): Promise<void> {
  try {
    await clearPendingPrompt(commandId, pairId);
  } catch (error) {
    // The Desktop already accepted this command. Keep the durable record with
    // its original command id so recovery (or a repeated tap) checks the
    // receipt instead of minting a second command and running it twice.
    console.warn("[remote] acknowledged prompt cleanup deferred", { commandId, error });
  }
}

async function clearAcknowledgedContinuation(commandId: string, pairId: string): Promise<void> {
  try {
    await clearPendingContinuation(commandId, pairId);
  } catch (error) {
    console.warn("[remote] acknowledged continuation cleanup deferred", { commandId, error });
  }
}

function samePendingPrompt(
  pending: PendingPrompt,
  candidate: Omit<PendingPrompt, "version" | "commandId" | "createdAt">,
): boolean {
  const attachmentKey = (items: MobileAttachment[]) =>
    items.map((item) => `${item.localUri}\u0000${item.name}\u0000${item.transferSize}`).sort();
  return (
    pending.draftKey === candidate.draftKey &&
    pending.sessionId === candidate.sessionId &&
    pending.text.trim() === candidate.text.trim() &&
    pending.modelId === candidate.modelId &&
    pending.thinkingLevel === candidate.thinkingLevel &&
    pending.mode === candidate.mode &&
    pending.workspaceId === candidate.workspaceId &&
    (pending.busyPolicy ?? "") === (candidate.busyPolicy ?? "") &&
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
  assertCurrentPairing: () => void,
  onUploadProgress?: (completedBytes: number, totalBytes: number) => void,
  recovering = false,
): Promise<PromptAck> {
  assertCurrentPairing();
  if (checkReceipt && receiptSupported) {
    const receipt = await pendingPromptReceipt(client, pending.commandId);
    assertCurrentPairing();
    if (receipt) return receipt;
  }
  if (
    recovering &&
    (!pending.bridgeInstanceId || pending.bridgeInstanceId !== client.accessIdentity)
  )
    throw new Error("remote_access_changed");
  const uploaded = await uploadAttachments(client, pending.attachments, onUploadProgress);
  assertCurrentPairing();
  return (
    await client.requestRetry<PromptAck>(
      {
        id: pending.commandId,
        ...(pending.bridgeInstanceId ? { bridgeInstanceId: pending.bridgeInstanceId } : {}),
        type: "prompt",
        sessionId: pending.sessionId,
        message: pending.text.trim(),
        modelId: pending.modelId,
        providerId: modelProviderFromReference(pending.modelId),
        level: pending.thinkingLevel,
        ...(uploaded.length
          ? { attachments: uploaded.map((attachment) => ({ uploadId: attachment.uploadId! })) }
          : {}),
        // What this send means if the session is still running: the phone used
        // to answer that itself by holding the message, and the session is the
        // one place the answer can be true for every device.
        ...(pending.busyPolicy ? { busyPolicy: pending.busyPolicy } : {}),
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
  recovering = false,
): Promise<PromptAck> {
  if (checkReceipt && receiptSupported) {
    const receipt = await pendingPromptReceipt(client, pending.commandId);
    if (receipt) return receipt;
  }
  if (
    recovering &&
    (!pending.bridgeInstanceId || pending.bridgeInstanceId !== client.accessIdentity)
  )
    throw new Error("remote_access_changed");
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

function pendingMatchesCredentials(
  pending: Pick<PendingPrompt, "pairId" | "expectedDesktopId">,
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
  compactingRef: MutableRefObject<Record<string, boolean>>;
  conversationEpochRef: MutableRefObject<number>;
  syncEngineRef: MutableRefObject<SyncEngine | null>;
  phase: ConnectionPhase;
  businessReady: boolean;
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
  compactingRef,
  conversationEpochRef,
  syncEngineRef,
  phase,
  businessReady,
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
    client: RemoteClient;
    sessionId: string;
    sourceRunId: string;
    promise: Promise<void>;
  } | null>(null);

  const sendMessage = useCallback(
    async (
      text: string,
      attachments: MobileAttachment[] = [],
      onUploadProgress?: (completedBytes: number, totalBytes: number) => void,
      /**
       * The command id this message already has, when it has one.
       *
       * A message the reader has sent once keeps the id that send minted, so every
       * later attempt at it — this screen's retry, the outbox's recovery after the
       * link returns, or the same pair of them on a phone that was restarted — is
       * one prompt on the desktop, which dedupes by this id alone.
       *
       * `busyPolicy` says what the send means while the conversation still runs
       * (`enqueue_coalescing`: the session holds it and runs it at the turn
       * boundary). Absent keeps the legacy refusal, which is what an idle send
       * wants: a busy session then answers with a reason the reader can read.
       */
      options?: { busyPolicy?: string; commandId?: string },
    ) => {
      const requestedCommandId = options?.commandId || null;
      const client = clientRef.current;
      const credentials = credentialsRef.current;
      if (!text.trim() && attachments.length === 0) return;
      if (!client || !credentials) throw new Error("not_connected");
      const assertCurrentPairing = () => {
        const current = credentialsRef.current;
        if (
          clientRef.current !== client ||
          !current ||
          !pendingMatchesCredentials(credentials, current)
        ) {
          throw new Error("pairing_changed");
        }
      };
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
        pairId: credentials.pairId,
        expectedDesktopId: credentials.expectedDesktopId,
        draftKey: desktopDraftKey(credentials.expectedDesktopId, targetSessionId),
        sessionId: targetSessionId,
        text,
        attachments,
        ...(options?.busyPolicy ? { busyPolicy: options.busyPolicy } : {}),
        modelId,
        thinkingLevel,
        mode: targetDraft ? targetDraftMode : ("chat" as const),
        workspaceId: targetDraft ? targetDraftWorkspaceId : "",
      };
      if (compactingRef.current[targetSessionId]) throw new Error("send_compacting");
      // A send is never refused over this client's own idea of whether the
      // session is still running. That flag is a local projection: it lags a
      // run that already ended (a missed terminal event, a socket that died
      // mid-stream) and there is nothing left to clear it — automatic resume
      // was retired in #1355, so a stale `streaming` entry sits there forever
      // and every later message answers "message failed to send". The desktop
      // owns the truth: a run still in flight is refused there before anything
      // is persisted, and that refusal reaches the user as its own sentence.

      // Acquire the prompt lane before the first storage await. Recovery and a
      // user tap can otherwise both read the same slot and deliver concurrently.
      sendingRef.current = true;
      setSending(true);
      /**
       * The desktop has this message: the record goes, the words the reader wrote
       * are committed into the conversation, and a send that opened a new session
       * moves the screen onto it.
       *
       * Shared by a fresh delivery and by one the desktop had already accepted,
       * which is why it takes the record it settles instead of closing over one.
       */
      const settle = async (response: PromptAck, delivered: PendingPrompt) => {
        await clearAcknowledgedPrompt(delivered.commandId, credentials.pairId);
        await clearSessionDraftIfMatches(delivered.draftKey, delivered);
        assertCurrentPairing();
        const nextSessionId = response.sessionId || targetSessionId;
        // A send that named a busy policy was handed to the session to WAIT, and
        // a message the session is holding has not been sent: it belongs to the
        // queue panel, not to the transcript. Drawing it here is what put a
        // second "你好" at the bottom of the conversation — the bubble this wrote
        // sat under the reply it was never answered by, and when the session ran
        // the queued prompt at the turn boundary its own `user_message` event
        // added the real one beside it.
        //
        // The desktop's send pipeline states the same rule (`echoOptimistically`
        // in `sendPipeline.ts`): the ack of a queued send draws nothing, while a
        // queued-intent send the session ran immediately still gets its bubble
        // there. That second ending is covered here by the branch below — the
        // `user_message` mirror of the run paints it, under the Agent's entry id.
        const queuedForLater = !!delivered.busyPolicy;
        if (!queuedForLater) {
          engine?.mutate(nextSessionId, (timeline) =>
            commitAcknowledgedUserMessage(timeline ?? emptyTimeline(), {
              id: `local:${delivered.commandId}`,
              runId: response.runId,
              text: delivered.text.trim(),
              ...(attachments.length
                ? {
                    attachments: attachments.map((attachment) => ({
                      path: attachment.localUri,
                      name: attachment.name,
                      kind: attachment.kind,
                      mobilePreviewUnsupported: attachment.mobilePreviewUnsupported,
                    })),
                  }
                : {}),
            }),
          );
        }
        if (nextSessionId && nextSessionId !== targetSessionId) {
          const stillViewingSentDraft = conversationEpochRef.current === conversationEpoch;
          if (stillViewingSentDraft) {
            selectedRef.current = nextSessionId;
            setSelectedSessionId(nextSessionId);
            setDraft(false);
            setDraftMode("chat");
            setDraftWorkspaceId("");
          }
          engine?.mutate(targetSessionId, (timeline) => ({
            ...(timeline ?? emptyTimeline()),
            items: [],
          }));
          if (stillViewingSentDraft) void refreshSessions();
        }
      };
      try {
        let pending = await loadPendingPrompt(credentials.pairId);
        assertCurrentPairing();
        if (pending && !pendingMatchesCredentials(pending, credentials)) {
          await clearPendingPrompt(pending.commandId, credentials.pairId);
          pending = null;
        }
        let checkReceipt = false;
        if (pending && samePendingPrompt(pending, candidate)) {
          checkReceipt = true;
        } else {
          // A caller that is re-attempting a delivery it already started hands the
          // command id back. Ask the desktop about that id before anything else: an
          // id it accepted is a delivered message — even though this phone no longer
          // holds the record, because the outbox cleared it when it delivered — and
          // sending the same words under a second id would start a second run.
          if (requestedCommandId) {
            const accepted = promptReceiptSupported
              ? await pendingPromptReceipt(client, requestedCommandId)
              : null;
            assertCurrentPairing();
            if (accepted) {
              const delivered: PendingPrompt = {
                version: 2,
                bridgeInstanceId: client.accessIdentity,
                commandId: requestedCommandId,
                ...candidate,
                createdAt: Date.now(),
              };
              if (pending) await clearPendingPrompt(pending.commandId, credentials.pairId);
              await settle(accepted, delivered);
              return;
            }
          }
          if (pending) {
            const previousReceipt = promptReceiptSupported
              ? await pendingPromptReceipt(client, pending.commandId)
              : null;
            assertCurrentPairing();
            if (previousReceipt) await clearSessionDraftIfMatches(pending.draftKey, pending);
            await clearPendingPrompt(pending.commandId, credentials.pairId);
          }
          assertCurrentPairing();
          pending = {
            version: 2,
            bridgeInstanceId: client.accessIdentity,
            commandId: requestedCommandId ?? randomId("prompt"),
            ...candidate,
            createdAt: Date.now(),
          };
          await savePendingPrompt(pending, credentials.pairId);
        }
        if (pending.bridgeInstanceId !== client.accessIdentity) {
          pending = { ...pending, bridgeInstanceId: client.accessIdentity };
          await savePendingPrompt(pending, credentials.pairId);
        }
        try {
          const response = await deliverPendingPrompt(
            client,
            pending,
            checkReceipt,
            promptReceiptSupported,
            assertCurrentPairing,
            onUploadProgress,
          );
          await settle(response, pending);
        } catch (sendError) {
          if (!isRecoverableDeliveryError(sendError)) {
            if (clientRef.current === client) {
              await clearPendingPrompt(pending.commandId, credentials.pairId);
            }
            throw sendError;
          }
          // The record stays, on purpose: the outbox delivers it on the next
          // readiness edge, under this command id. Reporting the id is what lets
          // the caller try the same message rather than mint a second one.
          throw withDeliveryCommandId(sendError, pending.commandId);
        }
      } finally {
        sendingRef.current = false;
        setSending(false);
      }
    },
    [
      clientRef,
      credentialsRef,
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
      compactingRef,
      syncEngineRef,
      thinkingLevel,
    ],
  );

  const recoverPendingPrompt = useCallback(async () => {
    if (sendingRef.current) return;
    if (pendingRecoveryRef.current) return pendingRecoveryRef.current;
    const client = clientRef.current;
    const credentials = credentialsRef.current;
    if (!client || !credentials) return;
    const assertCurrentPairing = () => {
      const current = credentialsRef.current;
      if (
        clientRef.current !== client ||
        !current ||
        !pendingMatchesCredentials(credentials, current)
      ) {
        throw new Error("pairing_changed");
      }
    };
    // Publish ownership synchronously, before loadPendingPrompt yields.
    sendingRef.current = true;
    setSending(true);
    const recovery = (async () => {
      const pending = await loadPendingPrompt(credentials.pairId);
      if (!pending) return;
      if (!pendingMatchesCredentials(pending, credentials)) {
        await clearPendingPrompt(pending.commandId, credentials.pairId);
        return;
      }
      try {
        const receipt = await deliverPendingPrompt(
          client,
          pending,
          true,
          promptReceiptSupported,
          assertCurrentPairing,
          undefined,
          true,
        );
        assertCurrentPairing();
        await clearAcknowledgedPrompt(pending.commandId, credentials.pairId);
        await clearSessionDraftIfMatches(pending.draftKey, pending);
        assertCurrentPairing();
        void refreshSessions();
        reconcileSession(receipt.sessionId, "reconnect");
      } catch (recoveryError) {
        if (clientRef.current !== client) return;
        if (!isRecoverableDeliveryError(recoveryError)) {
          await clearPendingPrompt(pending.commandId, credentials.pairId);
          if (!(
            recoveryError instanceof Error && recoveryError.message === "remote_access_changed"
          ))
            recordError(recoveryError);
        }
      }
    })().finally(() => {
      sendingRef.current = false;
      setSending(false);
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
      if (compactingRef.current[sessionId]) throw new Error("send_compacting");
      const client = clientRef.current;
      const credentials = credentialsRef.current;
      if (!client || !credentials) throw new Error("not_connected");
      const inFlight = continuationInFlightRef.current;
      if (inFlight) {
        if (inFlight.client === client && inFlight.sessionId === sessionId && inFlight.sourceRunId === runId) {
          return inFlight.promise;
        }
        await inFlight.promise;
      }

      const operation = (async () => {
        if (clientRef.current !== client || credentialsRef.current?.pairId !== credentials.pairId)
          throw new Error("pairing_changed");
        let pending = await loadPendingContinuation(credentials.pairId);
        if (pending && !pendingMatchesCredentials(pending, credentials)) {
          await discardPendingContinuation(credentials.pairId);
          pending = null;
        }
        let checkReceipt = pending !== null;
        if (pending && (pending.sessionId !== sessionId || pending.sourceRunId !== runId)) {
          if (promptReceiptSupported) await pendingPromptReceipt(client, pending.commandId);
          await clearPendingContinuation(pending.commandId, credentials.pairId);
          pending = null;
          checkReceipt = false;
        }
        if (!pending) {
          pending = {
            version: 2,
            bridgeInstanceId: client.accessIdentity,
            commandId: randomId("continue"),
            pairId: credentials.pairId,
            expectedDesktopId: credentials.expectedDesktopId,
            sessionId,
            sourceRunId: runId,
            createdAt: Date.now(),
          } satisfies PendingContinuation;
          await savePendingContinuation(pending, credentials.pairId);
        }
        if (pending.bridgeInstanceId !== client.accessIdentity) {
          pending = { ...pending, bridgeInstanceId: client.accessIdentity };
          await savePendingContinuation(pending, credentials.pairId);
        }
        try {
          await deliverPendingContinuation(client, pending, checkReceipt, promptReceiptSupported);
          await clearAcknowledgedContinuation(pending.commandId, credentials.pairId);
        } catch (continueError) {
          if (!isRecoverableDeliveryError(continueError)) {
            await clearPendingContinuation(pending.commandId, credentials.pairId);
          }
          throw continueError;
        }
      })();

      continuationInFlightRef.current = {
        client,
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
    [clientRef, compactingRef, credentialsRef, promptReceiptSupported],
  );

  const recoverPendingContinuation = useCallback(async () => {
    const client = clientRef.current;
    const credentials = credentialsRef.current;
    if (!client || !credentials || continuationInFlightRef.current) return;
    const pending = await loadPendingContinuation(credentials.pairId);
    if (!pending || continuationInFlightRef.current) return;
    if (!pendingMatchesCredentials(pending, credentials)) {
      await discardPendingContinuation(credentials.pairId);
      return;
    }

    const operation = (async () => {
      try {
        const receipt = await deliverPendingContinuation(
          client,
          pending,
          true,
          promptReceiptSupported,
          true,
        );
        if (clientRef.current !== client || credentialsRef.current?.pairId !== pending.pairId)
          return;
        await clearAcknowledgedContinuation(pending.commandId, credentials.pairId);
        void refreshSessions();
        reconcileSession(receipt.sessionId || pending.sessionId, "reconnect", receipt.runId);
      } catch (recoveryError) {
        if (clientRef.current !== client) return;
        if (!isRecoverableDeliveryError(recoveryError)) {
          await clearPendingContinuation(pending.commandId, credentials.pairId);
          if (!(
            recoveryError instanceof Error && recoveryError.message === "remote_access_changed"
          ))
            recordError(recoveryError);
        }
      }
    })();

    continuationInFlightRef.current = {
      client,
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
    if (phase !== "ready" || !businessReady) return;
    let active = true;
    void (async () => {
      // A readiness edge can arrive before an old receipt probe settles.
      // Wait for that owner, then make one pass on the now-usable channel.
      await pendingRecoveryRef.current;
      if (!active) return;
      await recoverPendingPrompt();
      await continuationInFlightRef.current?.promise.catch(() => undefined);
      if (active) await recoverPendingContinuation();
    })().catch(recordError);
    return () => { active = false; };
  }, [phase, businessReady, recordError, recoverPendingContinuation, recoverPendingPrompt]);

  return { sending, sendMessage, continueRun };
}
