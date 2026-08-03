import type { StoredApprovalRequest } from "../../../integrations/storage/threadStore";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef } from "react";
import i18n from "../../../i18n";
import { decideApprovalRequest, listApprovalRequests } from "../../../integrations/storage/threadStore";
import { useAsyncResource } from "../../../lib/useAsyncResource";
import { usePolling } from "../../../lib/usePolling";

const NO_APPROVALS: StoredApprovalRequest[] = [];

/** Payload of the backend's `approvals-updated` push (see lib.rs). */
interface ApprovalsUpdatedPayload {
  threadId: string;
  approvalRequestId: string;
}

export interface ApprovalsState {
  activeApproval: StoredApprovalRequest | null;
  /** Persist a manual decision (notifying the agent), then refetch the queue. */
  decideApproval: (
    approval: StoredApprovalRequest,
    status: "approved" | "rejected",
  ) => Promise<void>;
  /** Refetch pending approvals for the active thread. */
  reloadApprovals: () => void;
}

/**
 * Owns the pending-approval queue for the active thread: the backend pushes
 * `approvals-updated` the moment an approval row is written or decided, and
 * the derived "active" (oldest pending) approval comes from that queue. The
 * slow poll is only a backstop for a lost push (mirrors the run-status push +
 * reconcile pattern in useThreadStore). On a transient load error the previous
 * list is preserved (the loader lets the error propagate, and useAsyncResource
 * keeps the last good data) so the approval card doesn't flicker out and back
 * — a real risk since it sits right above the composer. There is no frontend
 * auto-approve engine — the "off" tier suppresses approval requests at the
 * agent, so nothing reaches this queue when off.
 */
export function useApprovals(activeThreadId: string | null): ApprovalsState {
  const { data: pendingApprovals, reload } = useAsyncResource(
    async () => {
      if (!activeThreadId) {
        return NO_APPROVALS;
      }
      const approvals = await listApprovalRequests(activeThreadId);
      return approvals.filter(approval => approval.status === "pending");
    },
    [activeThreadId],
    NO_APPROVALS,
    {
      isEqual: (prev, next) =>
        prev.length === next.length
        && prev.every((item, idx) =>
          item.id === next[idx]!.id && item.status === next[idx]!.status,
        ),
    },
  );

  // Mirror the active id into a ref so the listener registers once and still
  // filters against the current thread.
  const activeThreadIdRef = useRef(activeThreadId);
  activeThreadIdRef.current = activeThreadId;

  useEffect(() => {
    const unlisten = listen<ApprovalsUpdatedPayload>("approvals-updated", (event) => {
      // Only this thread's queue is rendered here; other threads' approvals
      // surface through the sidebar badge hook.
      if (event.payload.threadId === activeThreadIdRef.current) {
        reload();
      }
    });
    return () => {
      void unlisten.then(stop => stop());
    };
  }, [reload]);

  // No `activeThreadId` in the poll deps: useAsyncResource already reloads when
  // the thread changes, so restarting the poll on switch too would fire a
  // duplicate fetch every switch. The interval calls the stable `reload`.
  usePolling(reload, 15_000, { enabled: activeThreadId !== null });

  const activeApproval = useMemo(
    // The loader already returns only pending requests; just take the oldest.
    () => [...pendingApprovals].sort((left, right) => left.createdAt - right.createdAt)[0] ?? null,
    [pendingApprovals],
  );

  const decideApproval = useCallback(
    async (approval: StoredApprovalRequest, status: "approved" | "rejected") => {
      await decideApprovalRequest({
        approvalRequestId: approval.id,
        decisionNote: status === "approved" ? i18n.t("layout:approvals.approvedInGui") : i18n.t("layout:approvals.rejectedInGui"),
        status,
      });
      reload();
    },
    [reload],
  );

  return { activeApproval, decideApproval, reloadApprovals: reload };
}
