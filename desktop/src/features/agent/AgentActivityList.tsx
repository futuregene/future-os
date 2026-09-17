import type { AgentActivityItem, AgentActivityKind } from "@future-os/thread-projection";
import { Brain, ChevronDown, ChevronUp, FileText, Pencil, TerminalSquare, TriangleAlert } from "lucide-react";
import { useCallback, useState } from "react";
import i18n from "../../i18n";
import { cn } from "../../lib/cn";
import { emitFutureEvent } from "../../lib/futureEvents";
import { relativizeWorkspacePath } from "../../lib/workspacePath";

interface AgentActivityListProps {
  items?: AgentActivityItem[];
  workspacePath?: string | null;
  runId?: string | null;
}

export function AgentActivityList({ items, workspacePath, runId }: AgentActivityListProps) {
  const visibleItems = items?.filter(item => item.status === "running" || item.status === "completed" || item.status === "failed") ?? [];
  if (visibleItems.length === 0)
    return null;

  return (
    <div className="my-4 space-y-3">
      {visibleItems.map(item => (
        <AgentActivityLine item={item} key={item.id} workspacePath={workspacePath} runId={runId} />
      ))}
    </div>
  );
}

// Pure dispatcher (no hooks) so the leaf and group branches can each own their
// expand state without breaking the rules-of-hooks.
interface AgentActivityLineProps {
  item: AgentActivityItem;
  workspacePath?: string | null;
  runId?: string | null;
  /** An expanded mixed-step summary places its child headers in the reading column. */
  inSteps?: boolean;
}

export function AgentActivityLine({ item, workspacePath, runId, inSteps }: AgentActivityLineProps) {
  if ((item.children?.length ?? 0) > 0)
    return <AgentActivityGroupLine item={item} workspacePath={workspacePath} runId={runId} inSteps={inSteps} />;
  return <AgentActivitySingleLine item={item} workspacePath={workspacePath} runId={runId} inSteps={inSteps} />;
}

function AgentActivitySingleLine({ item, workspacePath, runId, inSteps }: AgentActivityLineProps) {
  const label = labelForActivity(item);
  const failed = item.status === "failed";
  const running = item.status === "running";
  const displayTarget = item.target ? relativizeTarget(item.kind, item.target, workspacePath) : undefined;
  // Only collapsed standalone steps use the right rail. Revealed rows return
  // to the reading column; long commands and paths get their own wrapping line.
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronUp : ChevronDown;

  const handleInspect = useCallback(() => {
    if (runId)
      emitFutureEvent("inspect-tool", { runId, toolId: item.id });
  }, [runId, item.id]);

  const handleToggle = () => {
    setOpen(value => !value);
    handleInspect();
  };

  return (
    <div className="flex min-w-0 flex-col gap-1 text-[13px] leading-6 text-ink-muted">
      <div className={cn("flex max-w-full items-center gap-2", !inSteps && !open ? "self-end" : "self-start")}>
        {runId || displayTarget
          ? (
              <button
                type="button"
                className="flex min-w-0 cursor-pointer items-center gap-2 text-left hover:text-ink"
                aria-expanded={open}
                title={runId ? i18n.t("agent:activity.inspectRun") : undefined}
                onClick={handleToggle}
              >
                {renderActivityIcon(item.kind, running, failed)}
                <span>{label}</span>
                <Chevron className="size-3 shrink-0" />
              </button>
            )
          : (
              <>
                {renderActivityIcon(item.kind, running, failed)}
                <span>{label}</span>
              </>
            )}
        {typeof item.additions === "number" || typeof item.deletions === "number"
          ? (
              <span className="shrink-0 font-mono text-xs">
                {typeof item.additions === "number" ? `+${item.additions}` : ""}
                {typeof item.deletions === "number" ? ` -${item.deletions}` : ""}
              </span>
            )
          : null}
      </div>
      {displayTarget && open
        ? (
            <div
              className="min-w-0 whitespace-pre-wrap wrap-anywhere pl-6 text-left font-mono text-ink-soft"
              title={item.detail ?? item.target}
            >
              {displayTarget}
            </div>
          )
        : null}
    </div>
  );
}

// A collapsed burst ("Ran 4 commands"). Collapsed, it's just the summary label —
// no inline preview, since a truncated command reads as noise. Clicking expands
// it into every child call as an indented, selectable sub-line. Grouping only
// happens for completed bursts, so a group is never running or failed.
function AgentActivityGroupLine({ item, workspacePath, runId, inSteps }: AgentActivityLineProps) {
  const label = labelForActivity(item);
  const children = item.children ?? [];
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronUp : ChevronDown;

  return (
    <div
      className="flex min-w-0 flex-col gap-1 text-[13px] leading-6 text-ink-muted"
    >
      <button
        type="button"
        className={cn("flex max-w-full cursor-pointer items-center gap-2 text-left hover:text-ink", !inSteps && !open ? "self-end" : "self-start")}
        aria-expanded={open}
        onClick={() => setOpen(v => !v)}
      >
        {renderActivityIcon(item.kind, false)}
        <span>{label}</span>
        <Chevron className="size-3 shrink-0" />
      </button>
      {open
        ? (
            <div className="flex flex-col gap-1 pl-6">
              {children.map(child => (
                <div
                  className="flex min-w-0 items-baseline gap-2 text-ink-soft"
                  key={child.id}
                >
                  {renderActivityIcon(child.kind, false)}
                  {runId
                    ? (
                        <button
                          type="button"
                          className="min-w-0 cursor-pointer whitespace-pre-wrap wrap-anywhere text-left font-mono hover:text-ink"
                          title={i18n.t("agent:activity.inspectRun")}
                          onClick={() => emitFutureEvent("inspect-tool", { runId, toolId: child.id })}
                        >
                          {child.target ? relativizeTarget(child.kind, child.target, workspacePath) : ""}
                        </button>
                      )
                    : (
                        <span
                          className="min-w-0 select-text whitespace-pre-wrap wrap-anywhere text-left font-mono"
                          title={child.detail ?? child.target}
                        >
                          {child.target ? relativizeTarget(child.kind, child.target, workspacePath) : ""}
                        </span>
                      )}
                </div>
              ))}
            </div>
          )
        : null}
    </div>
  );
}

// Shell targets are the command itself, never a path, so they're left as-is;
// file targets get the shared workspace-relative treatment.
function relativizeTarget(kind: AgentActivityKind, target: string, workspacePath?: string | null) {
  if (kind === "shell")
    return target;
  return relativizeWorkspacePath(target, workspacePath);
}

function renderActivityIcon(kind: AgentActivityKind, running: boolean, failed = false) {
  const className = cn("size-3.5 shrink-0", running && kind === "thinking" && "animate-pulse");
  // A failed call always gets the alert glyph, regardless of tool kind.
  if (failed)
    return <TriangleAlert className={className} />;
  switch (kind) {
    case "shell":
      return <TerminalSquare className={className} />;
    case "edit":
    case "write":
      return <Pencil className={className} />;
    case "read":
      return <FileText className={className} />;
    case "thinking":
      return <Brain className={className} />;
  }
}

function labelForActivity(item: AgentActivityItem) {
  if (item.kind === "thinking")
    return i18n.t("agent:activity.thinking");

  const count = item.count ?? 0;
  if (item.status === "failed")
    return failedLabel(item.kind);

  // Collapsed bursts only ever group completed calls, so group labels are
  // always past tense and self-contained ("Edited 3 files") — a shared
  // "Ran "/"已" prefix can't conjugate across tool kinds.
  if (count > 1) {
    if (item.kind === "shell")
      return i18n.t("agent:activity.runCommands", { count });
    if (item.kind === "write")
      return i18n.t("agent:activity.writeFiles", { count });
    if (item.kind === "read")
      return i18n.t("agent:activity.readFiles", { count });
    return i18n.t("agent:activity.editFiles", { count });
  }

  const running = item.status === "running";
  switch (item.kind) {
    case "read":
      return i18n.t(running ? "agent:activity.reading" : "agent:activity.readCompleted");
    case "shell":
      return i18n.t(running ? "agent:activity.runningCommand" : "agent:activity.runCompleted");
    case "write":
      return i18n.t(running ? "agent:activity.writing" : "agent:activity.writeCompleted");
    case "edit":
      return i18n.t(running ? "agent:activity.editing" : "agent:activity.editCompleted");
  }
}

function failedLabel(kind: Exclude<AgentActivityKind, "thinking">) {
  switch (kind) {
    case "shell":
      return i18n.t("agent:activity.failed.shell");
    case "edit":
      return i18n.t("agent:activity.failed.edit");
    case "read":
      return i18n.t("agent:activity.failed.read");
    case "write":
      return i18n.t("agent:activity.failed.write");
  }
}
