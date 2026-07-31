import type { StoredApprovalRequest } from "../../../integrations/storage/threadStore";
import { useMemo } from "react";
import { listPendingApprovalRequests } from "../../../integrations/storage/threadStore";
import { useAsyncResource } from "../../../lib/useAsyncResource";
import { usePolling } from "../../../lib/usePolling";

const NO_APPROVALS: StoredApprovalRequest[] = [];

/**
 * Global pending-approval counts keyed by threadId — the sidebar badge source.
 *
 * Distinct from `useApprovals`, which owns the *active* thread's queue and
 * drives the card above the composer. This one watches every thread, so an
 * approval raised in a background conversation (a GUI-started run, or one the
 * approval reconcile rebuilt after a restart) surfaces as a marker on its rail
 * item without opening the thread. The query returns pending rows only, so the
 * list is tiny and a full 2s refetch is cheap.
 */
export function usePendingApprovalCounts(): Map<string, number> {
  const { data, reload } = useAsyncResource(
    async () => listPendingApprovalRequests(),
    [],
    NO_APPROVALS,
    {
      isEqual: (prev, next) =>
        prev.length === next.length
        && prev.every((item, idx) => item.id === next[idx]!.id),
    },
  );

  usePolling(reload, 2000);

  return useMemo(() => {
    const counts = new Map<string, number>();
    for (const approval of data) {
      counts.set(approval.threadId, (counts.get(approval.threadId) ?? 0) + 1);
    }
    return counts;
  }, [data]);
}
