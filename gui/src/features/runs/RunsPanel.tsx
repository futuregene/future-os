import type { StoredRun, StoredToolCall } from "../../integrations/storage/threadStore";
import { CircleStop, Maximize2, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { Button } from "../../components/ui/Button";
import { EmptyState } from "../../components/ui/EmptyState";
import { runStatusLabel } from "./runDisplayFormatters";
import { RunError } from "./RunError";
import { toolCommand } from "./toolInput";

interface RunsPanelProps {
  runs: StoredRun[];
  toolsByRun: Record<string, StoredToolCall[]>;
  onClearFinished: () => Promise<void>;
  onInspectRun: (runId: string) => void;
  onTerminateRun: (run: StoredRun) => Promise<void>;
}

export function RunsPanel({ onClearFinished, onInspectRun, onTerminateRun, runs, toolsByRun }: RunsPanelProps) {
  const [confirmRunId, setConfirmRunId] = useState<string | null>(null);
  const [busyRunId, setBusyRunId] = useState<string | null>(null);
  const [actionErrors, setActionErrors] = useState<Record<string, string | undefined>>({});
  const [clearing, setClearing] = useState(false);
  const visibleRuns = useMemo(
    () => runs.filter(run => commandToolCall(toolsByRun[run.id] ?? [])),
    [runs, toolsByRun],
  );
  const runningRuns = visibleRuns.filter(isActiveRun);
  const finishedRuns = visibleRuns.filter(run => !isActiveRun(run));
  const orderedRuns = useMemo(
    () => [
      ...[...runningRuns].sort(compareRunTimeDesc),
      ...[...finishedRuns].sort(compareRunTimeDesc),
    ],
    [finishedRuns, runningRuns],
  );

  if (visibleRuns.length === 0) {
    return <EmptyState title="No background programs" detail="Agent work will appear here while it is running." />;
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
        [run.id]: error instanceof Error ? error.message : String(error),
      }));
    }
    finally {
      setBusyRunId(null);
    }
  }

  async function clearFinished() {
    if (clearing || finishedRuns.length === 0)
      return;

    setClearing(true);
    try {
      await onClearFinished();
    }
    finally {
      setClearing(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="text-xs text-ink-muted">
          {runningRuns.length}
          {" "}
          running /
          {" "}
          {finishedRuns.length}
          {" "}
          finished
        </div>
        <Button
          disabled={clearing || finishedRuns.length === 0}
          leftIcon={<Trash2 className="size-3.5" />}
          onClick={() => void clearFinished()}
          size="xs"
          variant="toolbar"
        >
          {clearing ? "Clearing" : "Clear finished"}
        </Button>
      </div>
      <div className="space-y-2">
        {orderedRuns.map(run => (
          <RunRow
            busy={busyRunId === run.id}
            confirming={confirmRunId === run.id}
            key={run.id}
            run={run}
            actionError={actionErrors[run.id]}
            tools={toolsByRun[run.id] ?? []}
            onCancelConfirm={() => setConfirmRunId(null)}
            onInspect={() => onInspectRun(run.id)}
            onRequestTerminate={() => setConfirmRunId(run.id)}
            onTerminate={() => void terminate(run)}
          />
        ))}
      </div>
    </div>
  );
}

function RunRow({
  busy,
  confirming,
  actionError,
  onCancelConfirm,
  onInspect,
  onRequestTerminate,
  onTerminate,
  run,
  tools,
}: {
  actionError?: string;
  busy: boolean;
  confirming: boolean;
  onCancelConfirm: () => void;
  onInspect: () => void;
  onRequestTerminate: () => void;
  onTerminate: () => void;
  run: StoredRun;
  tools: StoredToolCall[];
}) {
  const active = isActiveRun(run);
  const displayTool = commandToolCall(tools);
  const command = displayTool ? toolCommand(displayTool.input) : null;
  // A run can fire several commands; the card previews the latest one, so call
  // out the total to stay consistent with the thread's "ran N commands" line.
  const commandCount = tools.filter(tool => toolCommand(tool.input)).length;
  const toolMeta = displayTool
    ? [
        toolLabel(displayTool),
        commandCount > 1 ? `${commandCount} commands` : null,
        toolStatusLabel(displayTool),
      ].filter(Boolean).join(" · ")
    : runStatusLabel(run.status);

  if (!command)
    return null;

  return (
    <div className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2.5">
        <span className={active ? "mt-1.5 size-2.5 shrink-0 rounded-full bg-accent" : "mt-1.5 size-2.5 shrink-0 rounded-full bg-line"} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <div
              className="min-w-0 flex-1 whitespace-pre-wrap break-words text-sm font-normal leading-5 text-ink"
              title={command}
            >
              {command}
            </div>
            <button
              aria-label="Inspect run"
              className="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink"
              onClick={onInspect}
              title="Inspect run"
              type="button"
            >
              <Maximize2 className="size-3.5" />
            </button>
          </div>
          <div className="mt-2 text-xs font-medium text-ink-muted">
            {toolMeta}
          </div>
          {run.errorMessage && !active
            ? <RunError errorMessage={run.errorMessage} errorType={run.errorType} variant="summary" />
            : null}
          {actionError
            ? <div className="mt-2 line-clamp-3 text-xs leading-5 text-danger">{actionError}</div>
            : null}
          {active
            ? (
                <div className="mt-3 flex justify-end">
                  {confirming
                    ? (
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-ink-muted">Terminate this program?</span>
                          <Button disabled={busy} onClick={onCancelConfirm} size="xs" variant="ghost">
                            Cancel
                          </Button>
                          <Button
                            disabled={busy}
                            leftIcon={<CircleStop className="size-3.5" />}
                            onClick={onTerminate}
                            size="xs"
                            variant="danger"
                          >
                            {busy ? "Stopping" : "Terminate"}
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
                          Terminate
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

function isActiveRun(run: StoredRun) {
  return run.status === "queued" || run.status === "running" || run.status === "waiting_approval";
}

function compareRunTimeDesc(left: StoredRun, right: StoredRun) {
  return (right.startedAt ?? right.createdAt) - (left.startedAt ?? left.createdAt);
}

function commandToolCall(tools: StoredToolCall[]) {
  return [...tools]
    .filter(tool => toolCommand(tool.input))
    .sort((left, right) => (right.startedAt ?? right.createdAt) - (left.startedAt ?? left.createdAt))[0]
    ?? null;
}

function toolLabel(tool: StoredToolCall) {
  const name = tool.name.trim();
  if (!name)
    return "Tool";

  return name.slice(0, 1).toUpperCase() + name.slice(1);
}

function toolStatusLabel(tool: StoredToolCall) {
  switch (tool.status) {
    case "completed":
      return "Completed";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    case "running":
      return "Running";
    default:
      return tool.status ? tool.status : "Unknown";
  }
}
