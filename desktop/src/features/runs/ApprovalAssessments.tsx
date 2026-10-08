import type { StoredRun } from "../../integrations/storage/threadStore";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { CopyablePre } from "../../components/ui/CopyablePre";
import { listApprovalAssessments } from "../../integrations/storage/runs";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { usePolling } from "../../lib/usePolling";

export function ApprovalAssessments({ run, toolCallId }: { run: StoredRun; toolCallId?: string }) {
  const { t } = useTranslation("runs");
  const { data, error, reload } = useAsyncResource(() => listApprovalAssessments(run.id), [run.id, run.status], []);
  usePolling(reload, 1500, { enabled: run.status === "running", deps: [run.id] });
  const entries = data.filter(entry => entry.runId === run.id && (!toolCallId || entry.toolCallId === toolCallId));
  if (error)
    return <p className="text-xs text-danger">{t("autoApproval.loadError")}</p>;
  if (!entries.length)
    return null;
  return (
    <section className="shrink-0 space-y-2" aria-label={t("autoApproval.title")}>
      <h3 className="text-xs font-medium text-ink-muted">{t("autoApproval.title")}</h3>
      {entries.map(entry => (
        <details className="rounded-md border border-line-soft bg-surface p-2 text-xs" key={entry.id}>
          <summary className="flex cursor-pointer flex-wrap items-center gap-2 text-ink">
            <Badge tone={entry.status === "approved" ? "success" : "danger"}>{t(`autoApproval.status.${entry.status}`)}</Badge>
            <span>
              {t("autoApproval.riskLabel")}
              :
              {" "}
              {t(`autoApproval.risk.${entry.payload.effective.risk ?? "unknown"}`)}
            </span>
            <span>
              {t("autoApproval.authorizationLabel")}
              :
              {" "}
              {t(`autoApproval.authorization.${entry.payload.effective.authorization ?? "unknown"}`)}
            </span>
          </summary>
          <div className="mt-2 space-y-2 text-ink-soft">
            <p>{entry.payload.reported ? t(`autoApproval.reason.${entry.payload.reported.reason_code}`) : t("autoApproval.noClassification")}</p>
            {entry.payload.error_code ? <p>{t("autoApproval.reviewError")}</p> : null}
            <p>
              {t("autoApproval.confidenceLabel")}
              :
              {" "}
              {Object.entries(entry.payload.confidence).map(([key, value]) => `${t(`autoApproval.choice.${key}`)} ${Math.round(value * 100)}%`).join(" · ")}
            </p>
            <p>
              {t("autoApproval.modelLabel")}
              :
              {" "}
              {entry.payload.model ?? "—"}
              {" "}
              ·
              {" "}
              {entry.payload.duration_ms}
              {" "}
              ms
            </p>
            <CopyablePre maxHeightClassName="max-h-40" text={JSON.stringify(entry.payload.action, null, 2)} />
            <p className="break-all text-[11px] text-ink-muted">
              {t("autoApproval.auditLabel")}
              :
              {" "}
              {entry.payload.action_digest}
              {" "}
              ·
              {" "}
              {entry.payload.prompt_version}
              /
              {entry.payload.reason_catalog_version}
              /
              {entry.payload.policy_version}
              {" "}
              ·
              {" "}
              {entry.payload.provider_request_id ?? entry.id}
            </p>
          </div>
        </details>
      ))}
    </section>
  );
}
