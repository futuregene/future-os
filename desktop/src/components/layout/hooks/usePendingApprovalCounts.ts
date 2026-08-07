import type { StoredApprovalRequest } from "../../../integrations/storage/threadStore";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo } from "react";
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
 * item without opening the thread. The backend pushes `approvals-updated` the
 * moment a pending row is written or decided; the slow poll is only a backstop
 * for a lost push. The query returns pending rows only, so each refetch is
 * cheap.
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

  useEffect(() => {
    const unlisten = listen("approvals-updated", () => {
      // The push names one approval, but the query is global and tiny — a
      // change in any thread can shift every count, so reload the whole list.
      reload();
    });
    return () => {
      void unlisten.then(stop => stop());
    };
  }, [reload]);

  usePolling(reload, 15_000);

  return useMemo(() => {
    const counts = new Map<string, number>();
    for (const approval of data) {
      counts.set(approval.threadId, (counts.get(approval.threadId) ?? 0) + 1);
    }
    return counts;
  }, [data]);
}
