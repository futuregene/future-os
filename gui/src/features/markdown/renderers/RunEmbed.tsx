import type { StoredRun } from "../../../integrations/storage/types";
import type { FutureReference } from "../futureMarkdownTypes";
import { Maximize2, PlayCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../../components/ui/Badge";
import { Button } from "../../../components/ui/Button";
import { storedTimeToIso } from "../../../integrations/storage/threadStore";
import { formatTime } from "../../../lib/date";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { formatRunStatus, runTone, shortId } from "../../runs/runDisplayFormatters";

export function RunEmbed({
  reference,
  run,
}: {
  reference: FutureReference;
  run: StoredRun;
}) {
  const { t } = useTranslation("markdown");
  function inspectRun() {
    emitFutureEvent("inspect-run", { runId: run.id });
  }

  return (
    <article className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        <PlayCircle className="mt-0.5 size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <h4 className="truncate text-sm font-semibold text-ink">{reference.label ?? shortId(run.id)}</h4>
            <Badge tone={runTone(run.status)}>{formatRunStatus(run.status)}</Badge>
          </div>
          <div className="mt-1 text-xs text-ink-muted">
            {run.startedAt ? formatTime(storedTimeToIso(run.startedAt)) : formatTime(storedTimeToIso(run.createdAt))}
          </div>
          <div className="mt-2 grid grid-cols-2 gap-2 text-xs text-ink-soft">
            <div>
              <span className="text-ink-muted">{t("runEmbed.model")}</span>
              <div className="truncate">{run.modelId ?? "-"}</div>
            </div>
            <div>
              <span className="text-ink-muted">{t("runEmbed.run")}</span>
              <div className="truncate">{shortId(run.id)}</div>
            </div>
          </div>
          {run.errorMessage
            ? <p className="mt-2 rounded-md bg-danger-soft p-2 text-xs leading-5 text-danger">{run.errorMessage}</p>
            : null}
          <Button
            className="mt-3"
            leftIcon={<Maximize2 className="size-3.5" />}
            onClick={inspectRun}
            size="xs"
            variant="toolbar"
          >
            {t("runEmbed.inspect")}
          </Button>
        </div>
      </div>
    </article>
  );
}
