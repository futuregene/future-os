import type { StoredRun, StoredToolCall } from "../../integrations/storage/threadStore";
import { ChevronRight, CircleStop, Pencil, TerminalSquare, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { EmptyState } from "../../components/ui/EmptyState";
import i18n from "../../i18n";
import { cn } from "../../lib/cn";
import { errorMessage } from "../../lib/errors";
import { relativizeWorkspacePath } from "../../lib/workspacePath";
import { toolStatusLabel } from "./runDisplayFormatters";
import { toolCommand, toolTarget } from "./toolInput";

// Only these tool calls are worth a row of their own; `read`/`grep`/`ls` are
// navigation noise and stay collapsed out of the panel.
const DISPLAY_TOOLS = new Set(["bash", "write", "edit"]);

interface RunsPanelProps {
  runs: StoredRun[];
  toolsByRun: Record<string, StoredToolCall[]>;
  workspacePath?: string | null;
  onClearFinished: () => Promise<void>;
  onInspectTool: (toolId: string) => void;
  onTerminateRun: (run: StoredRun) => Promise<void>;
}

interface ToolEntry {
  tool: StoredToolCall;
  run: StoredRun;
  // Exactly one row per active run carries the terminate control — its latest
  // command — so a multi-command run isn't cluttered with duplicate stop buttons.
  terminable: boolean;
}

export function RunsPanel({ onClearFinished, onInspectTool, onTerminateRun, runs, toolsByRun, workspacePath }: RunsPanelProps) {
  const { t } = useTranslation("runs");
  const [confirmRunId, setConfirmRunId] = useState<string | null>(null);
  const [busyRunId, setBusyRunId] = useState<string | null>(null);
  const [actionErrors, setActionErrors] = useState<Record<string, string | undefined>>({});
  const [clearing, setClearing] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);

  const entries = useMemo(() => buildToolEntries(runs, toolsByRun), [runs, toolsByRun]);
  const { runningCount, finishedCount } = useMemo(() => countEntries(entries), [entries]);

  if (entries.length === 0) {
    return <EmptyState title={t("runsPanel.emptyTitle")} detail={t("runsPanel.emptyDetail")} />;
  }

  async function terminate(run: StoredRun) {
    setBusyRunId(run.id);
    setActionErrors(current => ({ ...current, [run.id]: undefined }));
    try {
      await onTerminateRun(run);
      setConfirmRunId(null);
    }
    catch (error) {
      setActionErrors(current => ({
        ...current,
        [run.id]: errorMessage(error),
      }));
    }
    finally {
      setBusyRunId(null);
    }
  }

  async function clearFinished() {
    if (clearing || finishedCount === 0)
      return;

    setClearing(true);
    setClearError(null);
    try {
      await onClearFinished();
    }
    catch (error) {
      setClearError(errorMessage(error));
    }
    finally {
      setClearing(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="text-xs text-ink-muted">
          {t("runsPanel.runningFinished", { running: runningCount, finished: finishedCount })}
        </div>
        <Button
          disabled={clearing || finishedCount === 0}
          leftIcon={<Trash2 className="size-3.5" />}
          onClick={() => void clearFinished()}
          size="xs"
          variant="toolbar"
        >
          {clearing ? t("runsPanel.clearing") : t("runsPanel.clearFinished")}
        </Button>
      </div>
      {clearError
        ? <div className="line-clamp-3 text-xs leading-5 text-danger">{clearError}</div>
        : null}
      <div className="space-y-2">
        {entries.map(entry => (
          <ToolRow
            busy={busyRunId === entry.run.id}
            confirming={confirmRunId === entry.run.id}
            key={entry.tool.id}
            entry={entry}
            workspacePath={workspacePath}
            actionError={actionErrors[entry.run.id]}
            onCancelConfirm={() => setConfirmRunId(null)}
            onInspect={() => onInspectTool(entry.tool.id)}
            onRequestTerminate={() => setConfirmRunId(entry.run.id)}
            onTerminate={() => void terminate(entry.run)}
          />
        ))}
      </div>
    </div>
  );
}

function ToolRow({
  busy,
  confirming,
  actionError,
  entry,
  workspacePath,
  onCancelConfirm,
  onInspect,
  onRequestTerminate,
  onTerminate,
}: {
  actionError?: string;
  busy: boolean;
  confirming: boolean;
  entry: ToolEntry;
  workspacePath?: string | null;
  onCancelConfirm: () => void;
  onInspect: () => void;
  onRequestTerminate: () => void;
  onTerminate: () => void;
}) {
  const { t } = useTranslation("runs");
  const { run, terminable, tool } = entry;
  const name = displayName(tool);
  const isBash = name === "bash";
  const rawPrimary = (isBash ? toolCommand(tool.input) : toolTarget(tool.input))
    ?? toolCommand(tool.input)
    ?? toolTarget(tool.input)
    ?? tool.input
    ?? toolLabel(tool);
  // Bash rows show the command verbatim; file rows (write/edit) get the
  // workspace-relative path, absolute kept for files outside the workspace.
  const primary = isBash ? rawPrimary : relativizeWorkspacePath(rawPrimary, workspacePath);
  // Show the tool's own status, never the run's. A tool still marked "running"
  // after its run has ended was interrupted — we can't tell a user abort from a
  // real failure, so treat it as failed rather than a perpetual "running".
  const status = tool.status === "running" && !isActiveRun(run) ? "failed" : tool.status;
  const running = status === "running" || terminable;
  const meta = [toolLabel(tool), toolStatusLabel(status)].filter(Boolean).join(" · ");

  return (
    <div className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2.5">
        {/* Icon + chevron sit in a first-line-tall (h-5 == leading-5) box and
            center within it, so both stay level with the text's first line
            whether the command wraps to one line or many. */}
        <span className="flex h-5 shrink-0 items-center">
          {isBash
            ? <TerminalSquare className={cn("size-4", running ? "text-accent" : "text-ink-muted")} />
            : <Pencil className={cn("size-4", running ? "text-accent" : "text-ink-muted")} />}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-2">
            <div
              className={cn(
                "min-w-0 flex-1 wrap-break-word text-sm font-normal leading-5 text-ink",
                isBash ? "whitespace-pre-wrap" : "truncate font-mono text-[0.85rem]",
              )}
              title={rawPrimary}
            >
              {primary}
            </div>
            <span className="flex h-5 shrink-0 items-center">
              <button
                aria-label={t("runsPanel.inspectTool")}
                className="-my-1 inline-flex size-7 items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink"
                onClick={onInspect}
                title={t("runsPanel.inspectTool")}
                type="button"
              >
                <ChevronRight className="size-4" />
              </button>
            </span>
          </div>
          <div className="mt-2 text-xs font-medium text-ink-muted">
            {meta}
          </div>
          {actionError
            ? <div className="mt-2 line-clamp-3 text-xs leading-5 text-danger">{actionError}</div>
            : null}
          {terminable
            ? (
                <div className="mt-3 flex justify-end">
                  {confirming
                    ? (
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-ink-muted">{t("runsPanel.confirmTerminate")}</span>
                          <Button disabled={busy} onClick={onCancelConfirm} size="xs" variant="ghost">
                            {t("runsPanel.cancel")}
                          </Button>
                          <Button
                            disabled={busy}
                            leftIcon={<CircleStop className="size-3.5" />}
                            onClick={onTerminate}
                            size="xs"
                            variant="danger"
                          >
                            {busy ? t("runsPanel.stopping") : t("runsPanel.terminate")}
                          </Button>
                        </div>
                      )
                    : (
                        <Button
                          leftIcon={<CircleStop className="size-3.5" />}
                          onClick={onRequestTerminate}
                          size="xs"
                          variant="danger-soft"
                        >
                          {t("runsPanel.terminate")}
                        </Button>
                      )}
                </div>
              )
            : null}
        </div>
      </div>
    </div>
  );
}

function displayName(tool: StoredToolCall) {
  return tool.name.trim().toLowerCase();
}

function isActiveRun(run: StoredRun) {
  return run.status === "queued" || run.status === "running" || run.status === "waiting_approval";
}

function compareToolTimeDesc(left: StoredToolCall, right: StoredToolCall) {
  return (right.startedAt ?? right.createdAt) - (left.startedAt ?? left.createdAt);
}

/**
 * Flatten every run's bash/write/edit tool calls into one chronological list —
 * active runs' tools first, then finished ones — so each command is its own row
 * instead of collapsing a run into a single card.
 */
function buildToolEntries(runs: StoredRun[], toolsByRun: Record<string, StoredToolCall[]>): ToolEntry[] {
  const active: ToolEntry[] = [];
  const finished: ToolEntry[] = [];
  for (const run of runs) {
    const tools = (toolsByRun[run.id] ?? []).filter(tool => DISPLAY_TOOLS.has(displayName(tool)));
    if (tools.length === 0)
      continue;

    const runActive = isActiveRun(run);
    const latestId = [...tools].sort(compareToolTimeDesc)[0]?.id;
    for (const tool of tools) {
      const entry: ToolEntry = { tool, run, terminable: runActive && tool.id === latestId };
      (runActive ? active : finished).push(entry);
    }
  }
  active.sort((left, right) => compareToolTimeDesc(left.tool, right.tool));
  finished.sort((left, right) => compareToolTimeDesc(left.tool, right.tool));
  return [...active, ...finished];
}

// Count the rendered command rows, not the runs — the list gives each command
// its own row, so a header keyed off run count would undercount whenever a run
// carries more than one command.
function countEntries(entries: ToolEntry[]) {
  let runningCount = 0;
  let finishedCount = 0;
  for (const entry of entries) {
    if (isActiveRun(entry.run))
      runningCount += 1;
    else
      finishedCount += 1;
  }
  return { finishedCount, runningCount };
}

function toolLabel(tool: StoredToolCall) {
  const name = tool.name.trim();
  if (!name)
    return i18n.t("runs:runInspect.toolFallback");

  return name.slice(0, 1).toUpperCase() + name.slice(1);
}
