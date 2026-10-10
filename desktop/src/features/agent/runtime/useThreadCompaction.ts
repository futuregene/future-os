import type { StoredThread } from "../../../integrations/storage/threadStore";
import { useCallback, useLayoutEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { compactThreadContext } from "../../../integrations/agent/agentClient";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";

const COMPACTION_TIMEOUT_MS = 30 * 60 * 1000;
const TERMINAL_EVENTS = new Set(["compaction_committed", "compaction_failed", "compaction_unchanged"]);

interface TerminalCompactionEvent {
  eventType: string;
  payload: Record<string, unknown>;
}

function cancelledWait() {
  // DOMException does not inherit from Error in every WebView/test runtime.
  const error = new Error("Compaction wait cancelled");
  error.name = "AbortError";
  return error;
}

/** Own the request, terminal event, timeout and cancellation as one operation. */
export function useThreadCompaction(thread: StoredThread | null) {
  const { t } = useTranslation("agent");
  const cancelRef = useRef<(() => void) | null>(null);
  useLayoutEffect(() => () => cancelRef.current?.(), [thread?.id, thread?.agentSessionId]);

  return useCallback(async () => {
    if (!thread)
      return;
    cancelRef.current?.();
    let active = true;
    let expectedOperationId: string | undefined;
    const buffered = new Map<string, TerminalCompactionEvent>();
    let resolveTerminal!: (event: TerminalCompactionEvent) => void;
    const terminalPromise = new Promise<TerminalCompactionEvent>((resolve) => {
      resolveTerminal = resolve;
    });
    let rejectWait!: (error: Error) => void;
    const interrupted = new Promise<never>((_resolve, reject) => {
      rejectWait = reject;
    });
    const cancel = () => {
      active = false;
      rejectWait(cancelledWait());
    };
    cancelRef.current = cancel;
    const timeout = setTimeout(() => {
      active = false;
      rejectWait(new Error(t("composer.compactionWaitTimedOut")));
    }, COMPACTION_TIMEOUT_MS);
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<TerminalCompactionEvent & { threadId: string; sessionId: string }>).detail;
      if (!active || !detail || detail.threadId !== thread.id
        || detail.sessionId !== thread.agentSessionId
        || !detail.payload || !TERMINAL_EVENTS.has(detail.eventType)) {
        return;
      }
      const operationId = detail.payload.operation_id;
      if (typeof operationId !== "string")
        return;
      if (operationId === expectedOperationId)
        resolveTerminal(detail);
      else if (!expectedOperationId)
        buffered.set(operationId, detail);
    };
    window.addEventListener("future:agent-event", handler);
    try {
      const operation = compactThreadContext(thread.id).then((result) => {
        if (!active)
          throw cancelledWait();
        expectedOperationId = result.operationId;
        const terminal = buffered.get(expectedOperationId);
        if (terminal)
          resolveTerminal(terminal);
        buffered.clear();
        return terminalPromise;
      });
      // Bind both rejection handlers immediately, including while the RPC waits.
      const terminal = await Promise.race([operation, interrupted]);
      if (terminal.eventType === "compaction_failed") {
        throw new Error(typeof terminal.payload.error === "string"
          ? terminal.payload.error
          : t("failure.unknown"));
      }
      if (terminal.eventType === "compaction_unchanged") {
        emitFutureEvent("toast", {
          message: t(terminal.payload.already_compacted
            ? "composer.compactionNoNewContent"
            : "composer.compactionNotNeeded"),
          tone: "info",
        });
      }
    }
    catch (error) {
      if (error instanceof Error && error.name === "AbortError")
        return;
      emitFutureEvent("toast", {
        message: t("composer.compactionRequestFailed", { message: errorMessage(error) }),
        tone: "error",
      });
    }
    finally {
      active = false;
      clearTimeout(timeout);
      window.removeEventListener("future:agent-event", handler);
      if (cancelRef.current === cancel)
        cancelRef.current = null;
    }
  }, [thread, t]);
}
