import type { StoredRun } from "../../integrations/storage/threadStore";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { listApprovalAssessments } from "../../integrations/storage/runs";
import { cn } from "../../lib/cn";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { usePolling } from "../../lib/usePolling";

export function ApprovalAssessments({ className, run, toolCallId }: { className?: string; run: StoredRun; toolCallId?: string }) {
  const { t } = useTranslation("runs");
  const { data, error, reload } = useAsyncResource(() => listApprovalAssessments(run.id), [run.id, run.status], []);
  usePolling(reload, 1500, { enabled: run.status === "running", deps: [run.id] });
  const entries = data.filter(entry => entry.runId === run.id && (!toolCallId || entry.toolCallId === toolCallId));
  if (error)
    return <p className={cn("text-xs text-danger", className)}>{t("autoApproval.loadError")}</p>;
  if (!entries.length)
    return null;
  return (
    <section className={cn("shrink-0 space-y-1", className)} aria-label={t("autoApproval.title")}>
      <h3 className="text-[11px] font-medium text-ink-muted">{t("autoApproval.title")}</h3>
      {entries.map(entry => (
        <div className="rounded-md border border-line-soft bg-surface/50 p-2 text-[11px]" key={entry.id}>
          <div className="flex flex-wrap items-center gap-2 text-ink-soft">
            <Badge tone={entry.status === "approved" ? "success" : "danger"}>{t(`autoApproval.status.${entry.status}`)}</Badge>
            {entry.payload.effective.risk && (
              <span>
                {t("autoApproval.riskLabel")}
                :
                {" "}
                {t(`autoApproval.risk.${entry.payload.effective.risk}`)}
              </span>
            )}
            {entry.payload.effective.authorization && (
              <span>
                {t("autoApproval.authorizationLabel")}
                :
                {" "}
                {t(`autoApproval.authorization.${entry.payload.effective.authorization}`)}
              </span>
            )}
          </div>
          {entry.status !== "approved" && (
            <p className="mt-1 text-ink-muted">
              {entry.status === "rejected" && entry.payload.reported && entry.payload.reported.reason_code !== "routine_bounded_action"
                ? t(`autoApproval.reason.${entry.payload.reported.reason_code}`)
                : t(`autoApproval.message.${entry.status}`)}
            </p>
          )}
        </div>
      ))}
    </section>
  );
}
