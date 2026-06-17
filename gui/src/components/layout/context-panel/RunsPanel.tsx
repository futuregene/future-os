import type { StoredRun, StoredToolCall } from "../../../integrations/storage/threadStore";
import { CircleStop, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { EmptyState } from "./ContextEmptyState";

interface RunsPanelProps {
  runs: StoredRun[];
  toolsByRun: Record<string, StoredToolCall[]>;
  onClearFinished: () => Promise<void>;
  onTerminateRun: (run: StoredRun) => Promise<void>;
}

export function RunsPanel({ onClearFinished, onTerminateRun, runs, toolsByRun }: RunsPanelProps) {
  const [confirmRunId, setConfirmRunId] = useState<string | null>(null);
  const [busyRunId, setBusyRunId] = useState<string | null>(null);
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
    try {
      await onTerminateRun(run);
      setConfirmRunId(null);
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
        <button
          className="inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-surface px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-white hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
          disabled={clearing || finishedRuns.length === 0}
          onClick={() => void clearFinished()}
          type="button"
        >
          <Trash2 className="size-3.5" />
          {clearing ? "Clearing" : "Clear finished"}
        </button>
      </div>
      <div className="space-y-2">
        {orderedRuns.map(run => (
          <RunRow
            busy={busyRunId === run.id}
            confirming={confirmRunId === run.id}
            key={run.id}
            run={run}
            tools={toolsByRun[run.id] ?? []}
            onCancelConfirm={() => setConfirmRunId(null)}
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
  onCancelConfirm,
  onRequestTerminate,
  onTerminate,
  run,
  tools,
}: {
  busy: boolean;
  confirming: boolean;
  onCancelConfirm: () => void;
  onRequestTerminate: () => void;
  onTerminate: () => void;
  run: StoredRun;
  tools: StoredToolCall[];
}) {
  const active = isActiveRun(run);
  const displayTool = commandToolCall(tools);
  const command = displayTool ? toolCommand(displayTool) : null;
  const toolMeta = displayTool ? `${toolLabel(displayTool)} ${toolStatusLabel(displayTool)}` : runStatusLabel(run);

  if (!command)
    return null;

  return (
    <div className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2.5">
        <span className={active ? "mt-1.5 size-2.5 shrink-0 rounded-full bg-blue-500" : "mt-1.5 size-2.5 shrink-0 rounded-full bg-slate-300"} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <div
              className="min-w-0 flex-1 whitespace-pre-wrap break-words text-sm font-normal leading-5 text-ink"
              title={command}
            >
              {command}
            </div>
          </div>
          <div className="mt-2 text-xs font-medium text-ink-muted">
            {toolMeta}
          </div>
          {run.errorMessage && !active
            ? <div className="mt-2 line-clamp-2 text-xs leading-5 text-red-600">{run.errorMessage}</div>
            : null}
          {active
            ? (
                <div className="mt-3 flex justify-end">
                  {confirming
                    ? (
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-ink-muted">Terminate this program?</span>
                          <button
                            className="h-7 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                            disabled={busy}
                            onClick={onCancelConfirm}
                            type="button"
                          >
                            Cancel
                          </button>
                          <button
                            className="inline-flex h-7 items-center gap-1.5 rounded-md bg-red-600 px-2 text-xs font-medium text-white transition-colors hover:bg-red-700 disabled:cursor-not-allowed disabled:opacity-60"
                            disabled={busy}
                            onClick={onTerminate}
                            type="button"
                          >
                            <CircleStop className="size-3.5" />
                            {busy ? "Stopping" : "Terminate"}
                          </button>
                        </div>
                      )
                    : (
                        <button
                          className="inline-flex h-7 items-center gap-1.5 rounded-md border border-red-200 bg-red-50 px-2 text-xs font-medium text-red-700 transition-colors hover:bg-red-100"
                          onClick={onRequestTerminate}
                          type="button"
                        >
                          <CircleStop className="size-3.5" />
                          Terminate
                        </button>
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

function runStatusLabel(run: StoredRun) {
  switch (run.status) {
    case "completed":
      return "Success";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    case "waiting_approval":
      return "Waiting";
    case "queued":
      return "Queued";
    default:
      return "Running";
  }
}

function commandToolCall(tools: StoredToolCall[]) {
  return [...tools]
    .filter(tool => toolCommand(tool))
    .sort((left, right) => (right.startedAt ?? right.createdAt) - (left.startedAt ?? left.createdAt))[0]
    ?? null;
}

function toolCommand(tool: StoredToolCall) {
  const input = tool.input?.trim();
  if (!input)
    return null;

  const command = stringField(parseToolInput(input), "command");
  if (command)
    return command;
  return null;
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

function parseToolInput(input: string) {
  let current: unknown = input;
  for (let index = 0; index < 3; index += 1) {
    if (isRecord(current))
      return current;
    if (typeof current !== "string")
      return null;

    try {
      current = JSON.parse(current) as unknown;
    }
    catch {
      return null;
    }
  }

  return isRecord(current) ? current : null;
}

function stringField(record: Record<string, unknown> | null, key: string) {
  const value = record?.[key];
  return typeof value === "string" && value.trim() ? value : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
