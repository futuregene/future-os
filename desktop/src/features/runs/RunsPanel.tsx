import type { RunsContextScope } from "../../components/layout/hooks/useContextData";
import type {
  StoredRun,
  StoredToolCall,
} from "../../integrations/storage/threadStore";
import {
  Archive,
  ChevronRight,
  CircleStop,
  Pencil,
  TerminalSquare,
} from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { EmptyState } from "../../components/ui/EmptyState";
import { FloatingScrollbar } from "../../components/ui/FloatingScrollbar";
import i18n from "../../i18n";
import { cn } from "../../lib/cn";
import { errorMessage } from "../../lib/errors";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { relativizeWorkspacePath } from "../../lib/workspacePath";
import { toolStatusLabel } from "./runDisplayFormatters";
import { toolCommand, toolTarget } from "./toolInput";

interface RunsPanelProps {
  runs: StoredRun[];
  toolsByRun: Record<string, StoredToolCall[]>;
  onArchiveFinished: (threadId: string) => Promise<void>;
  onInspectTool: (toolId: string) => void;
  onTerminateRun: (threadId: string, run: StoredRun) => Promise<void>;
  scope: RunsContextScope | null;
}

interface ToolEntry {
  tool: StoredToolCall;
  run: StoredRun;
  // Exactly one row per active run carries the terminate control — its newest
  // still-running command — so a multi-command run isn't cluttered with
  // duplicate stop buttons, and a completed command never shows one (its run
  // may still be generating its reply, but interrupting that is the composer's
  // stop button, not a per-command row).
  terminable: boolean;
}

export function RunsPanel({
  onArchiveFinished,
  onInspectTool,
  onTerminateRun,
  runs,
  scope,
  toolsByRun,
}: RunsPanelProps) {
  const { t } = useTranslation("runs");
  const [confirmRunId, setConfirmRunId] = useState<string | null>(null);
  const [busyRunId, setBusyRunId] = useState<string | null>(null);
  const [actionErrors, setActionErrors] = useState<
    Record<string, string | undefined>
  >({});
  const [archiving, setArchiving] = useState(false);
  const [archiveError, setArchiveError] = useState<string | null>(null);
  const listScrollbar = useFloatingScrollbar();

  const allEntries = useMemo(
    () => buildToolEntries(runs, toolsByRun),
    [runs, toolsByRun],
  );
  const entries = useMemo(
    () => allEntries.filter(entry => !entry.run.archivedAt),
    [allEntries],
  );
  const { runningCount, finishedCount } = useMemo(
    () => countEntries(entries),
    [entries],
  );

  if (entries.length === 0) {
    const hasArchivedPrograms = allEntries.some(
      entry => entry.run.archivedAt,
    );
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="group relative min-h-0 flex-1 border-t border-line-soft/70">
          <div
            className="floating-scrollbar h-full overflow-x-hidden overflow-y-auto bg-surface/50"
            onScroll={listScrollbar.handleScroll}
            ref={listScrollbar.scrollRef}
          >
            <EmptyState
              className="m-4"
              detail={t(
                hasArchivedPrograms
                  ? "runsPanel.emptyDetailArchived"
                  : "runsPanel.emptyDetailNeverRun",
              )}
              title={t("runsPanel.emptyTitle")}
            />
          </div>
          <FloatingScrollbar
            onPointerDown={listScrollbar.handleThumbPointerDown}
            scrollbar={listScrollbar.scrollbar}
          />
        </div>
      </div>
    );
  }

  async function terminate(run: StoredRun) {
    setBusyRunId(run.id);
    setActionErrors(current => ({ ...current, [run.id]: undefined }));
    try {
      await onTerminateRun(scope?.threadId ?? run.threadId, run);
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

  async function archiveFinished() {
    if (archiving || finishedCount === 0)
      return;

    setArchiving(true);
    setArchiveError(null);
    try {
      if (scope)
        await onArchiveFinished(scope.threadId);
    }
    catch (error) {
      setArchiveError(errorMessage(error));
    }
    finally {
      setArchiving(false);
    }
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="shrink-0 px-4 py-1.5">
        <div className="flex items-center justify-between gap-3">
          <div className="text-xs text-ink-muted">
            {t("runsPanel.runningFinished", {
              running: runningCount,
              finished: finishedCount,
            })}
          </div>
          <Button
            disabled={archiving || finishedCount === 0}
            leftIcon={<Archive className="size-3.5" />}
            onClick={() => void archiveFinished()}
            size="sm"
            variant="toolbar"
          >
            {t("runsPanel.archiveFinished")}
          </Button>
        </div>
        {archiveError
          ? (
              <div className="mt-2 line-clamp-3 text-xs leading-5 text-danger">
                {archiveError}
              </div>
            )
          : null}
      </div>
      <div className="group relative min-h-0 flex-1 border-t border-line-soft/70">
        <div
          className="floating-scrollbar h-full overflow-x-hidden overflow-y-auto bg-surface/50"
          onScroll={listScrollbar.handleScroll}
          ref={listScrollbar.scrollRef}
        >
          <div>
            {entries.map(entry => (
              <ToolRow
                busy={busyRunId === entry.run.id}
                confirming={confirmRunId === entry.run.id}
                key={entry.tool.id}
                entry={entry}
                workspacePath={scope?.workspacePath}
                actionError={actionErrors[entry.run.id]}
                onCancelConfirm={() => setConfirmRunId(null)}
                onInspect={() => onInspectTool(entry.tool.id)}
                onRequestTerminate={() => setConfirmRunId(entry.run.id)}
                onTerminate={() => void terminate(entry.run)}
              />
            ))}
          </div>
        </div>
        <FloatingScrollbar
          onPointerDown={listScrollbar.handleThumbPointerDown}
          scrollbar={listScrollbar.scrollbar}
        />
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
  const isShell = name === "shell";
  const rawPrimary
    = (isShell ? toolCommand(tool.input) : toolTarget(tool.input))
      ?? toolCommand(tool.input)
      ?? toolTarget(tool.input)
      ?? fallbackPrimary(tool.input, toolLabel(tool));
  // Shell rows show the command verbatim; file rows (write/edit) get the
  // workspace-relative path, absolute kept for files outside the workspace.
  const primary = isShell
    ? rawPrimary
    : relativizeWorkspacePath(rawPrimary, workspacePath);
  // Show the tool's own status, never the run's. A tool still marked "running"
  // after its run has ended was interrupted — we can't tell a user abort from a
  // real failure, so treat it as failed rather than a perpetual "running".
  const status
    = tool.status === "running" && !isActiveRun(run) ? "failed" : tool.status;
  const running = status === "running";
  const meta = [toolLabel(tool), toolStatusLabel(status)]
    .filter(Boolean)
    .join(" · ");

  return (
    <div
      className="group/run-row relative cursor-pointer border-b border-line-soft px-4 py-3 transition-colors hover:bg-surface-subtle focus-visible:bg-surface-subtle focus-visible:outline-none"
      onClick={(event) => {
        if ((event.target as HTMLElement).closest("button"))
          return;
        onInspect();
      }}
      onKeyDown={(event) => {
        if ((event.target as HTMLElement).closest("button"))
          return;
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onInspect();
        }
      }}
      role="button"
      tabIndex={0}
    >
      <div className="flex items-start gap-2.5 pr-7">
        {/* Icon sits in a first-line-tall (h-5 == leading-5) box, so it remains
            aligned with the command's first line when that command wraps. */}
        <span className="flex h-5 shrink-0 items-center">
          {isShell
            ? (
                <TerminalSquare
                  className={cn(
                    "size-4",
                    running ? "text-accent" : "text-ink-muted",
                  )}
                />
              )
            : (
                <Pencil
                  className={cn(
                    "size-4",
                    running ? "text-accent" : "text-ink-muted",
                  )}
                />
              )}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-start gap-2">
            <div
              className={cn(
                // min-h-5 (one leading-5 line) reserves the target line's
                // height while a streaming tool call has no parseable target
                // yet, so the row doesn't jump when the target appears.
                "min-h-5 min-w-0 flex-1 wrap-break-word text-xs font-normal leading-5 text-ink",
                isShell ? "whitespace-pre-wrap" : "truncate font-mono",
              )}
              title={rawPrimary}
            >
              {primary}
            </div>
          </div>
          <div className="mt-2 text-xs font-medium text-ink-muted">{meta}</div>
          {actionError
            ? (
                <div className="mt-2 line-clamp-3 text-xs leading-5 text-danger">
                  {actionError}
                </div>
              )
            : null}
          {terminable
            ? (
                <div className="mt-3 flex justify-end">
                  {confirming
                    ? (
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-ink-muted">
                            {t("runsPanel.confirmTerminate")}
                          </span>
                          <Button
                            disabled={busy}
                            onClick={onCancelConfirm}
                            size="xs"
                            variant="ghost"
                          >
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
      <button
        aria-label={t("runsPanel.inspectTool")}
        className="absolute right-4 top-1/2 inline-flex size-7 -translate-y-1/2 items-center justify-center text-ink-muted opacity-0 group-hover/run-row:opacity-100 group-focus-within/run-row:opacity-100"
        onClick={onInspect}
        title={t("runsPanel.inspectTool")}
        type="button"
      >
        <ChevronRight className="size-3.5" />
      </button>
    </div>
  );
}

function displayName(tool: StoredToolCall) {
  return tool.name.trim().toLowerCase();
}

/**
 * The raw input is shown only when it is plain text (legacy records carry the
 * command as a bare string). A JSON-object input that yielded no field — above
 * all a still-streaming partial — renders as a blank line instead of a raw
 * JSON blob, so the row swaps in the parsed target later without noise.
 */
function fallbackPrimary(
  input: string | null | undefined,
  label: string,
): string {
  if (input === null || input === undefined)
    return label;
  const first = input.trimStart().charAt(0);
  return first === "{" || first === "[" ? "" : input;
}

function isActiveRun(run: StoredRun) {
  return (
    run.status === "queued"
    || run.status === "running"
    || run.status === "waiting_approval"
  );
}

function compareToolTimeDesc(left: StoredToolCall, right: StoredToolCall) {
  return (
    (right.startedAt ?? right.createdAt) - (left.startedAt ?? left.createdAt)
  );
}

/**
 * Flatten every run's shell/write/edit tool calls into one chronological list —
 * active runs' tools first, then finished ones — so each command is its own row
 * instead of collapsing a run into a single card.
 */
function buildToolEntries(
  runs: StoredRun[],
  toolsByRun: Record<string, StoredToolCall[]>,
): ToolEntry[] {
  const active: ToolEntry[] = [];
  const finished: ToolEntry[] = [];
  for (const run of runs) {
    const tools = toolsByRun[run.id] ?? [];
    const runActive = isActiveRun(run);

    // A Run tracks the model reply lifecycle, not a background program. Keep
    // it out of this tool-only panel until the Agent has emitted a tool event.
    if (tools.length === 0) {
      continue;
    }

    // The terminate control rides the run's newest still-running command. A
    // completed tool never carries it — otherwise a finished command shows a
    // "completed" label next to a stop button while the run merely generates
    // its reply. (filter() copies, so the sort can't reorder the caller's
    // array.)
    const runningTools = tools
      .filter(tool => tool.status === "running")
      .sort(compareToolTimeDesc);
    const latestRunningId = runningTools[0]?.id;
    for (const tool of tools) {
      const entry: ToolEntry = {
        tool,
        run,
        terminable: runActive && tool.id === latestRunningId,
      };
      (runActive ? active : finished).push(entry);
    }
  }
  active.sort((left, right) => compareToolTimeDesc(left.tool, right.tool));
  finished.sort((left, right) => compareToolTimeDesc(left.tool, right.tool));
  return [...active, ...finished];
}

// Count the rendered command rows, not the runs — the list gives each command
// its own row, so a header keyed off run count would undercount whenever a run
// carries more than one command. Bucket each row by the tool's own effective
// status (matching the card), not the enclosing run's activity: a finished tool
// inside a still-running run is "已结束", not "运行中".
function countEntries(entries: ToolEntry[]) {
  let runningCount = 0;
  let finishedCount = 0;
  for (const entry of entries) {
    if (entry.tool.status === "running" && isActiveRun(entry.run))
      runningCount += 1;
    else finishedCount += 1;
  }
  return { finishedCount, runningCount };
}

function toolLabel(tool: StoredToolCall) {
  const name = tool.name.trim();
  if (!name)
    return i18n.t("runs:runInspect.toolFallback");

  return name.slice(0, 1).toUpperCase() + name.slice(1);
}
