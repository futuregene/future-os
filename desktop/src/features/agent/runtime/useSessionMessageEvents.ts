import type { AgentMessage } from "@future-os/thread-projection";
import type { Dispatch, SetStateAction } from "react";
import {
  upsertUserMessage,
  userMessageFromEvent,
} from "@future-os/thread-projection";
import {
  useEffect,
  useRef,
} from "react";
import { invokeCommand } from "../../../integrations/tauri/invoke";
import { emitFutureEvent } from "../../../lib/futureEvents";

interface Options {
  threadId: string | null;
  agentSessionId: string | null;
  isRunActive: boolean;
  loadingRef: { current: boolean };
  refreshRecentRun: (threadId: string) => Promise<void>;
  reloadThreadHistory: (threadId: string) => Promise<void>;
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>;
}

export function useSessionMessageEvents({ threadId, agentSessionId, isRunActive, loadingRef, refreshRecentRun, reloadThreadHistory, setMessages }: Options) {
  // Remote runs are discovered from the already-open session event stream.
  // This replaces the old per-thread 2s get_state poll.
  const attachedRef = useRef(false);

  // ── Real-time user_message from StreamEvents observer ────────────
  // Inserts the user message directly from the Tauri event stream
  // for zero-latency display.  All other events (text_chunk, thinking,
  // tools, agent_end) continue through the synthetic run → useRunReattach
  // path to avoid conflicting with the existing streaming bubble logic.
  useEffect(() => {
    if (!threadId || !agentSessionId)
      return;
    let active = true;
    const handler = (ev: Event) => {
      const detail = (ev as CustomEvent).detail as
        | {
          threadId: string;
          sessionId: string;
          eventType: string;
          payload: Record<string, unknown>;
        }
        | undefined;
      // Only this conversation's events apply to this instance — other
      // conversations live on their own keyed AgentThread instances.
      if (
        !detail
        || detail.threadId !== threadId
        || detail.sessionId !== agentSessionId
      ) {
        return;
      }
      if (detail.eventType === "agent_end") {
        attachedRef.current = false;
        emitFutureEvent("agent_end", undefined);
        return;
      }
      if (detail.eventType === "agent_start") {
        if (isRunActive || attachedRef.current)
          return;
        attachedRef.current = true;
        void invokeCommand<{ runId?: string }>("attach_remote_stream", {
          threadId,
        })
          .then(async (result) => {
            if (!active || !result?.runId)
              return;
            await reloadThreadHistory(threadId);
            if (active)
              await refreshRecentRun(threadId);
          })
          .catch(() => {
            if (active)
              attachedRef.current = false;
          });
        return;
      }
      if (
        detail.eventType === "compaction_started"
        || detail.eventType === "compaction_committed"
        || detail.eventType === "compaction_failed"
      ) {
        // Run-scoped compaction is already projected from the persisted run
        // event log. This direct path is for standalone/manual and model-switch
        // compaction, which has no active run bubble to host its status.
        if (isRunActive || attachedRef.current)
          return;
        const operationId
          = typeof detail.payload.operation_id === "string"
            ? detail.payload.operation_id
            : `session_${Date.now()}`;
        const messageId = `compaction_${operationId}`;
        const checkpointId
          = typeof detail.payload.checkpoint_id === "string"
            ? detail.payload.checkpoint_id
            : operationId;
        const tokensBefore
          = typeof detail.payload.tokens_before === "number"
            ? detail.payload.tokens_before
            : undefined;
        const tokensAfter
          = typeof detail.payload.tokens_after === "number"
            ? detail.payload.tokens_after
            : undefined;
        const error
          = typeof detail.payload.error === "string"
            ? detail.payload.error
            : undefined;
        const trigger
          = typeof detail.payload.trigger === "string"
            ? detail.payload.trigger
            : undefined;
        const status
          = detail.eventType === "compaction_started"
            ? ("running" as const)
            : detail.eventType === "compaction_failed"
              ? ("failed" as const)
              : ("completed" as const);
        setMessages((prev) => {
          const segment = {
            id: checkpointId,
            kind: "compaction" as const,
            ...(tokensBefore != null && tokensBefore > 0
              ? { tokensBefore }
              : {}),
            ...(tokensAfter != null && tokensAfter > 0
              ? { tokensAfter }
              : {}),
            ...(trigger ? { trigger } : {}),
            ...(status !== "completed" ? { status } : {}),
            ...(error ? { error } : {}),
          };
          const existing = prev.findIndex(
            message => message.id === messageId,
          );
          const message: AgentMessage = {
            id: messageId,
            role: "assistant",
            authorKey: "author.researchCopilot",
            content: "",
            status: "complete",
            createdAt: new Date().toISOString(),
            segments: [segment],
          };
          if (existing < 0)
            return [...prev, message];
          const next = [...prev];
          next[existing] = message;
          return next;
        });
        return;
      }
      if (detail.eventType !== "user_message")
        return;

      // A user_message that lands while this thread's load is still in flight
      // would append onto the not-yet-committed base — dropping it is
      // lossless, the Agent history load carries the persisted entry.
      if (loadingRef.current)
        return;

      const user = userMessageFromEvent(detail.payload);
      if (!user) {
        // An identity-less event can only invalidate history; text is not a key.
        void reloadThreadHistory(threadId);
        return;
      }
      setMessages(prev => upsertUserMessage(prev, user));
    };
    window.addEventListener("future:agent-event", handler);
    return () => {
      active = false;
      window.removeEventListener("future:agent-event", handler);
    };
  }, [agentSessionId, isRunActive, refreshRecentRun, reloadThreadHistory, setMessages, threadId, loadingRef]);
}
