import type { StoredApprovalRequest } from "../../../integrations/storage/threadStore";
import { useCallback, useMemo } from "react";
import i18n from "../../../i18n";
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
 * Owns the pending-approval queue for the active thread: a 1.5s poll and the
 * derived "active" (oldest pending) approval. A load is dropped (and the list
 * cleared) on error, matching the previous inline behavior. There is no
 * frontend auto-approve engine — the "off" tier suppresses approval requests at
 * the agent, so nothing reaches this queue when the user opts out.
 */
export function useApprovals(activeThreadId: string | null): ApprovalsState {
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
