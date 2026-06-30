import type { StoredApprovalRequest } from "../../../integrations/storage/threadStore";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { decideApprovalRequest, listApprovalRequests } from "../../../integrations/storage/threadStore";
import { useAsyncResource } from "../../../lib/useAsyncResource";
import { usePolling } from "../../../lib/usePolling";

const NO_APPROVALS: StoredApprovalRequest[] = [];

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
 * Owns the pending-approval queue for the active thread: a 1.5s poll, the
 * derived "active" (oldest pending) approval, and the auto-approve engine that
 * resolves each new request once when the setting is on. A load is dropped (and
 * the list cleared) on error, matching the previous inline behavior.
 */
export function useApprovals(activeThreadId: string | null, autoApprove: boolean): ApprovalsState {
  const { data: pendingApprovals, reload } = useAsyncResource(
    async () => {
      if (!activeThreadId) {
        return NO_APPROVALS;
      }
      try {
        const approvals = await listApprovalRequests(activeThreadId);
        return approvals.filter(approval => approval.status === "pending");
      }
      catch {
        return NO_APPROVALS;
      }
    },
    [activeThreadId],
    NO_APPROVALS,
  );

  usePolling(reload, 1500, { enabled: activeThreadId !== null, deps: [activeThreadId] });

  const activeApproval = useMemo(
    // The loader already returns only pending requests; just take the oldest.
    () => [...pendingApprovals].sort((left, right) => left.createdAt - right.createdAt)[0] ?? null,
    [pendingApprovals],
  );

  const autoApprovingRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    if (!autoApprove) {
      return;
    }
    for (const approval of pendingApprovals) {
      if (autoApprovingRef.current.has(approval.id)) {
        continue;
      }
      autoApprovingRef.current.add(approval.id);
      void decideApprovalRequest({
        approvalRequestId: approval.id,
        decisionNote: "Auto-approved by settings.",
        status: "approved",
      })
        .catch(() => undefined)
        .finally(() => {
          autoApprovingRef.current.delete(approval.id);
          reload();
        });
    }
  }, [autoApprove, pendingApprovals, reload]);

  const decideApproval = useCallback(
    async (approval: StoredApprovalRequest, status: "approved" | "rejected") => {
      await decideApprovalRequest({
        approvalRequestId: approval.id,
        decisionNote: status === "approved" ? "Approved in GUI." : "Rejected in GUI.",
        status,
      });
      reload();
    },
    [reload],
  );

  return { activeApproval, decideApproval, reloadApprovals: reload };
}
