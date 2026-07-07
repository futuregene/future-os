import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import type { AgentMessage, MessageAttachment } from "./agentThreadTypes";
import type { ComposerSendPayload } from "./Composer";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "../../i18n";
import { sendPromptToFutureAgent } from "../../integrations/agent/agentClient";
import {
  abortRun,
  appendMessage,
  createRun,
  deleteTempAttachment,
  listMessages,
  listRuns,
  storedTimeToIso,
} from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { upsertFutureReferenceData } from "../markdown/futureReferenceStore";
import {
  buildAgentFailureContent,
  matchesSettledRun,
  toAgentMessage,
  updateRunStatusSafe,
} from "./agentMessageFormatters";
import { buildInlineAttachmentContext, imageAttachmentPaths, stringifyMessageContent } from "./attachments";
import { buildReferencePrompt } from "./buildReferencePrompt";
import { importChatAttachments, withImageThumbnails } from "./threadAttachments";
import {
  clientId,
  deriveRenderFields,
  loadCurrentRun,
  patchMessage,
  restoreMessageActivities,
  runDurationMs,
  safeListRunEvents,
  updatePendingMessageFromRunEvents,
  upsertStreamingPreview,
} from "./threadRunProjection";
import { useStickyAutoScroll } from "./useStickyAutoScroll";

interface UseAgentThreadStateInput {
  thread: StoredThread | null;
  loadingStore: boolean;
  modelId: string;
  thinkingLevel: string;
  pendingPrompt: { attachments?: MessageAttachment[]; id: string; content: string; targetThreadId: string } | null;
  onPromptConsumed: (id: string) => void;
  onThreadActivity: () => void;
}

export function useAgentThreadState({
  thread,
  loadingStore,
  modelId,
  thinkingLevel,
  pendingPrompt,
  onPromptConsumed,
  onThreadActivity,
}: UseAgentThreadStateInput) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [loadingThread, setLoadingThread] = useState(true);
  const [recentRun, setRecentRun] = useState<StoredRun | null>(null);
  const {
    scrollRef,
    scrollbar,
    updateFloatingScrollbar,
    handleScroll: handleScrollbarVisibility,
    handleThumbPointerDown,
  } = useFloatingScrollbar();
  const consumedPromptRef = useRef<string | null>(null);
  const sendGenerationRef = useRef(0);
  // True while a prompt is in flight for this thread. The agent rejects a second
  // concurrent prompt for the same session, so guard every send path with it.
  const sendingRef = useRef(false);
  const threadId = thread?.id ?? null;

  // Sticky auto-scroll: follow streaming output only while pinned near the
  // bottom; re-pins on thread switch and follows the growing message list.
  const { handleScroll, scrollToLatest, showJumpToLatest } = useStickyAutoScroll({
    scrollRef,
    resetKey: threadId,
    contentKey: messages,
    onScroll: handleScrollbarVisibility,
    onContentSettled: () => updateFloatingScrollbar(false),
  });

  // The run this thread is currently executing, if any. Runs stream server-side
  // and persist their events regardless of which thread is in the foreground, so
  // this is the anchor for re-attaching a live preview to a conversation that was
  // started, backgrounded, and returned to (or picked up after an app reload) —
  // the in-flight `handleSend` only drives the view while its own thread stays
  // foreground. Guarded on `threadId` because `recentRun` lags a thread switch by
  // one poll, and a stale run from the previous thread must not leak into this
  // one. Null once the run settles.
  const activeRunId
    = recentRun && recentRun.threadId === threadId && !matchesSettledRun(recentRun.status)
      ? recentRun.id
      : null;
  // Epoch-ms anchor for the live elapsed timer of a re-attached run. Stable while
  // the run stays active (derived from persisted run times), so it doesn't churn
  // the resume effect the way the `recentRun` object identity would.
  const activeRunStartedAt = activeRunId ? (recentRun?.startedAt ?? recentRun?.createdAt ?? null) : null;
  const prevActiveRunIdRef = useRef<string | null>(null);

  // Guard against overlapping refreshes (poll tick, send, thread switch) where a
  // slow response lands after a newer one and writes stale run state — e.g. a
  // previous thread's run after switching. Newest call wins.
  const recentRunGenRef = useRef(0);
  const refreshRecentRun = useCallback(async (threadId: string, workspaceId?: string | null) => {
    const generation = ++recentRunGenRef.current;
    try {
      const runs = await listRuns(threadId);
      if (generation !== recentRunGenRef.current) {
        return;
      }
      const latestRun = runs[0] ?? null;
      setRecentRun(latestRun);
      if (latestRun) {
        upsertFutureReferenceData(workspaceId, "run", latestRun.id, latestRun);
      }
    }
    catch {
      // Run-status refresh is best-effort: a failure here must not blank the
      // thread (it runs alongside listMessages in loadThreadMessages via
      // Promise.all) or abort an in-flight send. Keep the previous recentRun
      // until the next poll. The waiting-approval prompt is rendered separately
      // by AgentThread from `activeApproval`, so no message rewrite is needed.
    }
  }, []);

  // Reload a thread's messages from the store without flipping the full-screen
  // loading state — used to swap a synthetic streaming bubble for the persisted
  // assistant message once a background run settles.
  const reloadMessagesQuiet = useCallback(async (targetThreadId: string) => {
    try {
      const storedMessages = await listMessages(targetThreadId);
      const agentMessages = storedMessages.map(toAgentMessage);
      const restoredMessages = await restoreMessageActivities(agentMessages, targetThreadId);
      setMessages(restoredMessages);
    }
    catch {
      // Best-effort refresh: keep the current messages on failure.
    }
  }, []);

  const handleSend = useCallback(async ({ attachments, content }: ComposerSendPayload) => {
    if (!thread)
      return;
    // One prompt at a time per session. `sendingRef` guards a send this
    // view started; `activeRunId` guards a run already in flight that this view
    // is only re-attached to (backgrounded, reloaded, or remote-driven). Every
    // send path — composer, `recover-run`, pendingPrompt — funnels through here,
    // so a second prompt while one runs would be rejected by the agent
    // ("already running") and leave a stray failed run. Reject it up front, with
    // visible feedback instead of a silent drop.
    if (sendingRef.current || activeRunId) {
      emitFutureEvent("toast", { message: i18n.t("agent:thread.alreadyRunning"), tone: "info" });
      return;
    }
    sendingRef.current = true;

    const sendGeneration = sendGenerationRef.current + 1;
    sendGenerationRef.current = sendGeneration;
    const isCurrentSend = () => sendGenerationRef.current === sendGeneration;
    // Timer handle is local to this send closure, not a shared ref: a prior fix
    // regression let one thread's send clear another thread's stream timer,
    // freezing the live bubble. Ownership stays with the closure.
    let streamTimer: number | null = null;
    const clearStreamTimer = () => {
      if (streamTimer !== null) {
        window.clearInterval(streamTimer);
        streamTimer = null;
      }
    };
    const optimisticUserId = clientId("pending_user");
    const pendingId = clientId("pending");
    const runStartAnchorMs = Date.now();
    const optimisticUserMessage: AgentMessage = {
      id: optimisticUserId,
      role: "user",
      author: i18n.t("agent:author.you"),
      authorKey: "author.you",
      content,
      status: "complete",
      createdAt: new Date().toISOString(),
      attachments,
    };
    const assistantMessage: AgentMessage = {
      id: pendingId,
      role: "assistant",
      author: i18n.t("agent:author.researchCopilot"),
      authorKey: "author.researchCopilot",
      content: "",
      status: "streaming",
      createdAt: new Date(runStartAnchorMs).toISOString(),
      // Mid-reasoning from the outset; the footer shows a "thinking…" hint (when
      // show-thinking is off) instead of a top-of-message activity line.
      thinkingActive: true,
      modelId,
      runStartedAt: runStartAnchorMs,
    };
    setMessages(current => [...current, optimisticUserMessage, assistantMessage]);
    onThreadActivity();

    let run: StoredRun | null = null;

    try {
      const importedAttachments = await withImageThumbnails(
        await importChatAttachments(thread, attachments),
      );

      // Extract PDF/text into the model-facing prompt only; keep the visible
      // bubble (messageContent) free of the bulky inlined text. inlineContext is
      // persisted in the stored message so a resend can reuse it.
      const inlineContext = await buildInlineAttachmentContext(importedAttachments);
      const messageContent = importedAttachments.length > 0
        ? stringifyMessageContent(content, importedAttachments, inlineContext)
        : content;
      const promptContent = await buildReferencePrompt(
        thread.workspaceId,
        content,
        inlineContext ? `${content}${inlineContext}` : content,
      );

      if (isCurrentSend()) {
        patchMessage(setMessages, optimisticUserId, { attachments: importedAttachments });
      }

      const storedUserMessage = await appendMessage({
        threadId: thread.id,
        role: "user",
        contentType: importedAttachments.length > 0 ? "mixed" : "markdown",
        content: messageContent,
        status: "complete",
      });

      if (isCurrentSend()) {
        patchMessage(setMessages, optimisticUserId, {
          id: storedUserMessage.id,
          createdAt: storedTimeToIso(storedUserMessage.createdAt),
        });
      }

      run = await createRun({
        threadId: thread.id,
        triggerMessageId: storedUserMessage.id,
        modelId,
      });

      if (isCurrentSend()) {
        setRecentRun(run);
        upsertFutureReferenceData(thread.workspaceId, "run", run.id, run);
        patchMessage(setMessages, pendingId, { runId: run?.id ?? null });
      }

      clearStreamTimer();
      if (isCurrentSend()) {
        streamTimer = window.setInterval(() => {
          if (run && isCurrentSend()) {
            void updatePendingMessageFromRunEvents(run.id, pendingId, setMessages, isCurrentSend);
          }
        }, 220);
      }

      const agentSessionId = thread.agentSessionId?.trim() || thread.id;
      const reply = await sendPromptToFutureAgent(
        promptContent,
        thread.id,
        agentSessionId,
        run.id,
        modelId,
        // Always forward image attachments. The per-model supportsImages flag
        // comes from the provider catalog's modality data, which is unreliable
        // for the Future provider (capable models like Qwen-VL were mis-flagged
        // text-only, so images were silently dropped). A genuinely text-only
        // model returns a clear API error instead — better than the model
        // replying "I don't see an image".
        imageAttachmentPaths(importedAttachments),
        thinkingLevel,
      );
      clearStreamTimer();

      // Chat attachments were copied into the artifact store, so the pasted temp
      // originals are now redundant. (Guarded to our temp dir; user-picked files
      // are rejected and left untouched. Workspace threads keep temps for resend.)
      if (thread.mode === "chat") {
        void Promise.all(attachments.map(item => deleteTempAttachment(item.path).catch(() => {})));
      }

      const currentRun = await loadCurrentRun(thread.id, run.id);
      if (currentRun && matchesSettledRun(currentRun.status)) {
        // The run settled while we awaited the agent. For a user abort
        // (`cancelled`) the agent stopped mid-reply: keep the partial text so it
        // survives a reload instead of vanishing, and finalize the streaming
        // bubble in place. Other settled statuses were finalized by whoever set
        // them — just release the lock (via `finally`).
        if (currentRun.status === "cancelled") {
          const partial = reply.content.trim();
          if (partial) {
            // Persist the partial reply regardless of whether this view still owns
            // the send: aborting and then immediately switching threads flips
            // `isCurrentSend()` false, and gating persistence on it dropped the
            // already-generated text permanently. This local send is the
            // only writer for the run, so there's no double-insert; on return to
            // the thread the reload restores it (stopped, per run.status).
            const storedAssistantMessage = await appendMessage({
              threadId: thread.id,
              runId: run.id,
              role: "assistant",
              contentType: "markdown",
              content: partial,
              status: "complete",
            });
            if (isCurrentSend()) {
              const abortedRender = deriveRenderFields(
                await safeListRunEvents(run.id),
                storedAssistantMessage.content,
              );
              patchMessage(setMessages, pendingId, {
                id: storedAssistantMessage.id,
                runId: storedAssistantMessage.runId,
                content: abortedRender.content,
                segments: abortedRender.segments,
                status: storedAssistantMessage.status,
                createdAt: storedTimeToIso(storedAssistantMessage.createdAt),
                outputTokens: abortedRender.outputTokens,
                stopped: true,
              });
              onThreadActivity();
            }
          }
          else if (isCurrentSend()) {
            // Aborted before any text landed (e.g. still in the thinking phase).
            // There's nothing to persist, but the pending bubble must leave
            // "streaming" — otherwise `isSending` stays true, so the composer is
            // stuck on the stop button and the generating/thinking indicators
            // linger. Finalize it in place (keeping any thinking the poll
            // accumulated), drop the thinking flag, and mark it stopped.
            patchMessage(setMessages, pendingId, {
              status: "complete",
              thinkingActive: false,
              stopped: true,
            });
            onThreadActivity();
          }
        }
        return;
      }
      // The stream closed before the agent signalled a clean end: the text is a
      // truncated prefix, not a finished answer. Persist it (so the partial isn't
      // lost) but finalize the run and bubble as failed rather than silently
      // presenting a cut-off reply as complete.
      if (!reply.complete) {
        const interruptedMessage = i18n.t("agent:thread.responseInterrupted");
        await updateRunStatusSafe(run.id, "failed", interruptedMessage);
        if (isCurrentSend()) {
          await refreshRecentRun(thread.id, thread.workspaceId);
        }
        const storedAssistantMessage = await appendMessage({
          threadId: thread.id,
          runId: run.id,
          role: "assistant",
          contentType: "markdown",
          content: reply.content.trim() || buildAgentFailureContent(interruptedMessage),
          status: "failed",
        });
        const partialRender = deriveRenderFields(
          await safeListRunEvents(run.id),
          storedAssistantMessage.content,
        );
        if (isCurrentSend()) {
          patchMessage(setMessages, pendingId, {
            id: storedAssistantMessage.id,
            runId: storedAssistantMessage.runId,
            content: partialRender.content,
            segments: partialRender.segments,
            status: storedAssistantMessage.status,
            createdAt: storedTimeToIso(storedAssistantMessage.createdAt),
            outputTokens: partialRender.outputTokens,
          });
          onThreadActivity();
        }
        return;
      }
      await updateRunStatusSafe(run.id, "completed");
      if (isCurrentSend()) {
        await refreshRecentRun(thread.id, thread.workspaceId);
      }
      const storedAssistantMessage = await appendMessage({
        threadId: thread.id,
        runId: run.id,
        role: "assistant",
        contentType: "markdown",
        content: reply.content.trim() || i18n.t("agent:thread.agentDoneNoText"),
        status: "complete",
      });

      // Streaming polls can lag the final text tail; re-project the now-complete
      // events so the inline segments match the persisted reply exactly.
      const finalRender = deriveRenderFields(
        await safeListRunEvents(run.id),
        storedAssistantMessage.content,
      );
      const settledRun = await loadCurrentRun(thread.id, run.id);
      const durationMs = runDurationMs(settledRun, runStartAnchorMs);

      if (isCurrentSend()) {
        patchMessage(setMessages, pendingId, {
          id: storedAssistantMessage.id,
          runId: storedAssistantMessage.runId,
          content: finalRender.content,
          segments: finalRender.segments,
          status: storedAssistantMessage.status,
          createdAt: storedTimeToIso(storedAssistantMessage.createdAt),
          modelId: settledRun?.modelId ?? modelId,
          durationMs,
          outputTokens: finalRender.outputTokens,
        });
        onThreadActivity();
      }
    }
    catch (error) {
      clearStreamTimer();

      const message = errorMessage(error);
      if (run) {
        const currentRun = await loadCurrentRun(thread.id, run.id);
        if (!currentRun || !matchesSettledRun(currentRun.status)) {
          await updateRunStatusSafe(run.id, "failed", message);
        }
        if (isCurrentSend()) {
          await refreshRecentRun(thread.id, thread.workspaceId);
        }
      }
      const storedAssistantMessage = run
        ? await appendMessage({
            threadId: thread.id,
            runId: run.id,
            role: "assistant",
            contentType: "markdown",
            content: buildAgentFailureContent(message),
            status: "failed",
          })
        : null;
      if (isCurrentSend()) {
        patchMessage(setMessages, pendingId, previous => ({
          id: storedAssistantMessage?.id ?? previous.id,
          runId: storedAssistantMessage?.runId ?? previous.runId,
          content: storedAssistantMessage?.content ?? buildAgentFailureContent(message),
          status: storedAssistantMessage?.status ?? "failed",
          createdAt: storedAssistantMessage
            ? storedTimeToIso(storedAssistantMessage.createdAt)
            : previous.createdAt,
        }));
        onThreadActivity();
      }
    }
    finally {
      // Release the in-flight lock — but only if a newer send/thread switch
      // hasn't already taken over (it owns the flag then).
      if (isCurrentSend())
        sendingRef.current = false;
    }
  }, [activeRunId, modelId, onThreadActivity, refreshRecentRun, thinkingLevel, thread]);

  // Interrupt the in-flight run for this thread. Best-effort: the backend stops
  // the agent and marks the run `cancelled`; the in-flight `handleSend` then
  // settles the streaming bubble to the partial reply (see the cancelled branch
  // above), and refreshing `recentRun` clears `activeRunId` so the resume effect
  // reconciles. Safe to call when nothing is running (resolves to a no-op).
  const handleAbort = useCallback(async () => {
    if (!threadId)
      return;
    const runId
      = recentRun && recentRun.threadId === threadId && !matchesSettledRun(recentRun.status)
        ? recentRun.id
        : messages.find(message => message.role === "assistant" && message.status === "streaming")?.runId ?? null;
    if (!runId)
      return;
    try {
      await abortRun({ threadId, runId });
    }
    catch {
      // The run may already have finished; the refresh below still reconciles.
    }
    await refreshRecentRun(threadId, thread?.workspaceId);
    onThreadActivity();
  }, [messages, onThreadActivity, recentRun, refreshRecentRun, thread?.workspaceId, threadId]);

  useEffect(() => {
    return () => {
      sendGenerationRef.current += 1;
      // A switch away abandons any in-flight send for this thread; let the new
      // thread send freely (its run is a different session). The abandoned send's
      // stream timer is a closure-local handle now, and its interval
      // callback no-ops once `isCurrentSend()` turns false, so there's nothing to
      // clear here — it stops on its own when that send's await returns.
      sendingRef.current = false;
      // Drop the run tracked for the outgoing thread so the incoming thread's
      // settle detection doesn't fire against a stale run id.
      prevActiveRunIdRef.current = null;
    };
  }, [threadId]);

  // Re-attach a live preview to a run that this view didn't start (or is no
  // longer driving): a conversation started, backgrounded, and returned to while
  // still running, or one picked up after an app reload. While a local send owns
  // the view (`sendingRef`), that path renders the stream itself, so skip.
  //
  // The poll UPSERTS the streaming bubble every tick (not a one-time insert) so it
  // survives a `loadThreadMessages` array-replace that lands mid-stream — the next
  // tick simply re-inserts it. That resilience is what makes a returned-to run
  // reconnect instead of showing an empty bubble the reload silently dropped.
  useEffect(() => {
    if (!threadId || !activeRunId || sendingRef.current)
      return;

    const runId = activeRunId;
    const startedAt = activeRunStartedAt;
    let cancelled = false;
    const tick = () => {
      if (cancelled)
        return;
      void upsertStreamingPreview(runId, startedAt, setMessages, () => !cancelled);
    };
    tick();
    const timer = window.setInterval(tick, 220);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeRunId, activeRunStartedAt, threadId]);

  // When a run this view was previewing (but did not itself start) settles,
  // reload the thread so the synthetic streaming bubble is replaced by the
  // persisted assistant message.
  useEffect(() => {
    const previous = prevActiveRunIdRef.current;
    prevActiveRunIdRef.current = activeRunId;
    if (previous && !activeRunId && !sendingRef.current && threadId) {
      void reloadMessagesQuiet(threadId);
    }
  }, [activeRunId, reloadMessagesQuiet, threadId]);

  // A remote (phone/web) client can drive this thread's session in the
  // background. This view never started that run, and the recent-run poll below
  // only self-sustains once a run is already in flight — so a fresh remote run
  // on an idle foreground thread would otherwise go unnoticed here (only the
  // sidebar's independent run-status poll would spin). On the backend's
  // remote-activity signal for THIS thread, pull the new run (which arms the
  // live-preview + settle-reload machinery) and reload messages so the phone's
  // user bubble shows immediately. Skip while a local send owns the view.
  useEffect(() => {
    if (!threadId)
      return;
    let cancelled = false;
    const unlisten = listen<string>("remote-activity", (event) => {
      if (cancelled || event.payload !== threadId || sendingRef.current)
        return;
      void refreshRecentRun(threadId, thread?.workspaceId);
      void reloadMessagesQuiet(threadId);
    });
    return () => {
      cancelled = true;
      void unlisten.then(stop => stop());
    };
  }, [refreshRecentRun, reloadMessagesQuiet, thread?.workspaceId, threadId]);

  useEffect(() => {
    // Hand-rolled cancel guard (not useAsyncResource): loads messages, refreshes
    // the recent run, and restores activities into several states at once, which
    // doesn't map onto the primitive's single-resource shape. See gui/CLAUDE.md §4.
    let cancelled = false;

    async function loadThreadMessages() {
      if (!threadId) {
        setMessages([]);
        setLoadingThread(false);
        return;
      }

      setLoadingThread(true);
      try {
        const [storedMessages] = await Promise.all([listMessages(threadId), refreshRecentRun(threadId, thread?.workspaceId)]);
        const agentMessages = storedMessages.map(toAgentMessage);
        const restoredMessages = await restoreMessageActivities(agentMessages, threadId);
        if (!cancelled) {
          setMessages(restoredMessages);
        }
      }
      catch (error) {
        const message = errorMessage(error);
        if (!cancelled) {
          setMessages([
            {
              id: "store_error",
              role: "assistant",
              author: i18n.t("agent:author.system"),
              authorKey: "author.system",
              content: i18n.t("agent:thread.messagesLoadFailed", { message }),
              createdAt: new Date().toISOString(),
            },
          ]);
        }
      }
      finally {
        if (!cancelled) {
          setLoadingThread(false);
        }
      }
    }

    void loadThreadMessages();

    return () => {
      cancelled = true;
    };
  }, [refreshRecentRun, thread?.workspaceId, threadId]);

  useEffect(() => {
    if (!threadId || !recentRun || matchesSettledRun(recentRun.status))
      return;

    const timer = window.setInterval(() => {
      void refreshRecentRun(threadId, thread?.workspaceId);
    }, 1500);

    return () => window.clearInterval(timer);
  }, [recentRun, refreshRecentRun, thread?.workspaceId, threadId]);

  useEffect(() => {
    if (!thread || loadingThread || loadingStore || !pendingPrompt)
      return;
    if (consumedPromptRef.current === pendingPrompt.id)
      return;
    // Only deliver the prompt to the thread it was composed for. A fast thread
    // switch during the (async) message load can make `thread` the newly-opened
    // conversation while this prompt still targets the one just created — sending
    // here would drop the first message (and its attachments) into the wrong
    // chat and persist it there. Wait for the target thread to be active.
    if (pendingPrompt.targetThreadId !== thread.id)
      return;

    consumedPromptRef.current = pendingPrompt.id;
    onPromptConsumed(pendingPrompt.id);
    void handleSend({ attachments: pendingPrompt.attachments ?? [], content: pendingPrompt.content });
  }, [handleSend, loadingStore, loadingThread, onPromptConsumed, pendingPrompt, thread]);

  return {
    handleAbort,
    handleScroll,
    handleSend,
    handleThumbPointerDown,
    loadingThread,
    messages,
    recentRun,
    scrollRef,
    scrollbar,
    scrollToLatest,
    showJumpToLatest,
  };
}
