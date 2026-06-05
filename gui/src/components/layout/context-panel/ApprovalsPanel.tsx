import type { StoredApprovalRequest } from "../../../integrations/storage/threadStore";
import { AlertTriangle, Check, ShieldCheck, X } from "lucide-react";
import { useState } from "react";
import {
  decideApprovalRequest,
} from "../../../integrations/storage/threadStore";
import { Badge } from "../../ui/Badge";
import { EmptyState } from "./ContextEmptyState";

export function ApprovalsPanel({
  approvals,
  onDecision,
}: {
  approvals: StoredApprovalRequest[];
  onDecision: () => void;
}) {
  if (approvals.length === 0) {
    return <EmptyState title="No approval requests" detail="Risky actions will appear here for review." />;
  }

  return (
    <div className="space-y-3">
      {approvals.map(approval => (
        <ApprovalCard key={approval.id} approval={approval} onDecision={onDecision} />
      ))}
    </div>
  );
}

function ApprovalCard({
  approval,
  onDecision,
}: {
  approval: StoredApprovalRequest;
  onDecision: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [deciding, setDeciding] = useState<"approved" | "rejected" | null>(null);

  async function decide(status: "approved" | "rejected") {
    if (deciding)
      return;

    setError(null);
    setDeciding(status);
    try {
      await decideApprovalRequest({
        approvalRequestId: approval.id,
        status,
        decisionNote: status === "approved" ? "Approved in GUI." : "Rejected in GUI.",
      });
      onDecision();
    }
    catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
    finally {
      setDeciding(null);
    }
  }

  return (
    <div className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        {approval.status === "pending"
          ? (
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600" />
            )
          : (
              <ShieldCheck className="mt-0.5 size-4 shrink-0 text-ink-muted" />
            )}
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <div className="truncate text-sm font-semibold text-ink">{approval.title}</div>
            <Badge tone={approval.status === "pending" ? "warning" : "neutral"}>{approval.status}</Badge>
          </div>
          <div className="mt-1 text-xs text-ink-muted">{approval.kind}</div>
          {approval.summary ? <p className="mt-2 text-sm leading-5 text-ink-soft">{approval.summary}</p> : null}
          {approval.requestedAction
            ? (
                <pre className="mt-2 max-h-32 overflow-auto rounded-md bg-surface-subtle p-2 text-xs leading-5 text-ink-soft">
                  <code>{approval.requestedAction}</code>
                </pre>
              )
            : null}
          {approval.status === "pending"
            ? (
                <div className="mt-3 flex justify-end gap-2">
                  <button
                    className="inline-flex h-8 items-center gap-1.5 rounded-md border border-red-200 bg-red-50 px-2 text-sm font-medium text-red-700 transition-colors hover:bg-red-100 disabled:cursor-not-allowed disabled:opacity-60"
                    disabled={deciding !== null}
                    onClick={() => void decide("rejected")}
                    type="button"
                  >
                    <X className="size-3.5" />
                    {deciding === "rejected" ? "Rejecting" : "Reject"}
                  </button>
                  <button
                    className="inline-flex h-8 items-center gap-1.5 rounded-md border border-green-200 bg-green-50 px-2 text-sm font-medium text-green-700 transition-colors hover:bg-green-100 disabled:cursor-not-allowed disabled:opacity-60"
                    disabled={deciding !== null}
                    onClick={() => void decide("approved")}
                    type="button"
                  >
                    <Check className="size-3.5" />
                    {deciding === "approved" ? "Approving" : "Approve"}
                  </button>
                </div>
              )
            : null}
          {error ? <div className="mt-2 text-xs leading-5 text-red-600">{error}</div> : null}
        </div>
      </div>
    </div>
  );
}
