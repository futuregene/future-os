import type { StoredRun, StoredToolCall } from "../../integrations/storage/threadStore";
import { ChevronRight, UserRoundGroup } from "lucide-react";
import { useTranslation } from "react-i18next";
import { EmptyState } from "../../components/ui/EmptyState";
import { formatDateTime } from "../../lib/date";
import { toolStatusLabel } from "../runs/runDisplayFormatters";
import { recordOf, stringField } from "../runs/toolInput";
import { FusionDiscussionPanel } from "./FusionDiscussionPanel";

export interface FusionSelection {
  runId: string;
  toolCallId: string;
  status?: string;
}

export function FusionPanel({ runs, toolsByRun, selected, onSelect, onBack }: {
  runs: StoredRun[];
  toolsByRun: Record<string, StoredToolCall[]>;
  selected: FusionSelection | null;
  onSelect: (selection: FusionSelection) => void;
  onBack: () => void;
}) {
  const { t, i18n } = useTranslation("agent");
  if (selected) {
    return <FusionDiscussionPanel key={`${selected.runId}:${selected.toolCallId}`} {...selected} onBack={onBack} />;
  }
  // Archived parent runs remain available for discussion review.
  const entries = runs.flatMap(run => (toolsByRun[run.id] ?? [])
    .filter(tool => tool.name.trim().toLowerCase() === "fusion")
    .map(tool => ({ run, tool })))
    .sort((a, b) => (b.tool.startedAt ?? b.tool.createdAt) - (a.tool.startedAt ?? a.tool.createdAt));
  return (
    <div className="h-full overflow-y-auto border-t border-line-soft" data-fusion-panel="list">
      {entries.length === 0
        ? <EmptyState className="m-4" title={t("fusion.detail.emptyTitle")} detail={t("fusion.detail.emptyDetail")} />
        : entries.map(({ run, tool }) => {
            const question = stringField(recordOf(tool.input), "question");
            const status = tool.status === "running" && !["queued", "running", "waiting_approval"].includes(run.status) ? "interrupted" : tool.status;
            return (
              <button key={`${run.id}:${tool.id}`} type="button" className="flex w-full items-start gap-3 border-b border-line-soft px-4 py-4 text-left hover:bg-surface-subtle" onClick={() => onSelect({ runId: run.id, toolCallId: tool.id, status })}>
                <UserRoundGroup className="mt-0.5 size-4 shrink-0 text-ink-muted" />
                <span className="min-w-0 flex-1 space-y-2">
                  <span className="block wrap-break-word text-sm leading-6 text-ink">{question || t("fusion.title")}</span>
                  <span className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-ink-muted">
                    <span>{toolStatusLabel(status)}</span>
                    <span>{formatDateTime(new Date(tool.startedAt ?? tool.createdAt).toISOString(), i18n.language)}</span>
                  </span>
                </span>
                <ChevronRight className="mt-1 size-3.5 shrink-0 text-ink-muted" />
              </button>
            );
          })}
    </div>
  );
}
