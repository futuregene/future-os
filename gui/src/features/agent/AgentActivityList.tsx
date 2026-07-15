import type { AgentActivityItem, AgentActivityKind } from "./agentThreadTypes";
import { Brain, ChevronLeft, ChevronRight, FileText, Pencil, TerminalSquare, TriangleAlert } from "lucide-react";
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
export function AgentActivityLine({ item, workspacePath, runId }: { item: AgentActivityItem; workspacePath?: string | null; runId?: string | null }) {
  if ((item.children?.length ?? 0) > 0)
    return <AgentActivityGroupLine item={item} workspacePath={workspacePath} runId={runId} />;
  return <AgentActivitySingleLine item={item} workspacePath={workspacePath} runId={runId} />;
}

function AgentActivitySingleLine({ item, workspacePath, runId }: { item: AgentActivityItem; workspacePath?: string | null; runId?: string | null }) {
  const label = labelForActivity(item);
  const failed = item.status === "failed";
  const running = item.status === "running";
  const displayTarget = item.target ? relativizeTarget(item.kind, item.target, workspacePath) : undefined;
  // The path is hidden by default to keep the transcript quiet; clicking the
  // icon+label toggles it. Chevron points right (expand) when collapsed, left
  // (collapse) when open.
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronLeft : ChevronRight;

  const handleInspect = useCallback(() => {
    if (runId)
      emitFutureEvent("inspect-tool", { runId, toolId: item.id });
  }, [runId, item.id]);

  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-2 text-[13px] leading-6 text-ink-muted",
        runId && "cursor-pointer hover:text-ink",
      )}
      onClick={runId ? () => { setOpen(value => !value); handleInspect(); } : undefined}
      title={runId ? i18n.t("agent:activity.inspectRun") : undefined}
      role={runId ? "button" : undefined}
    >
      {displayTarget
        ? (
            <button
              type="button"
              className="pointer-events-none flex shrink-0 cursor-pointer items-center gap-2"
              aria-expanded={open}
            >
              {renderActivityIcon(item.kind, running, failed)}
              <span>{label}</span>
              <Chevron className="-ml-2 size-3 shrink-0" />
            </button>
          )
        : (
            <>
              {renderActivityIcon(item.kind, running, failed)}
              <span className="shrink-0">{label}</span>
            </>
          )}
      {displayTarget && open
        ? (
            <span
              className="min-w-0 truncate font-mono"
              title={item.detail ?? item.target}
            >
              {displayTarget}
            </span>
          )
        : null}
      {typeof item.additions === "number" || typeof item.deletions === "number"
        ? (
            <span className="shrink-0 font-mono text-xs">
              {typeof item.additions === "number" ? `+${item.additions}` : ""}
              {typeof item.deletions === "number" ? ` -${item.deletions}` : ""}
            </span>
          )
        : null}
    </div>
  );
}

// A collapsed burst ("Ran 4 commands"). Collapsed, it's just the summary label —
// no inline preview, since a truncated command reads as noise. Clicking expands
// it into every child call as an indented, selectable sub-line. Grouping only
// happens for completed bursts, so a group is never running or failed.
function AgentActivityGroupLine({ item, workspacePath, runId }: { item: AgentActivityItem; workspacePath?: string | null; runId?: string | null }) {
  const label = labelForActivity(item);
  const children = item.children ?? [];
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronLeft : ChevronRight;

  const handleInspect = useCallback(() => {
    if (runId)
      emitFutureEvent("inspect-tool", { runId, toolId: item.id });
  }, [runId, item.id]);

  return (
    <div
      className="flex min-w-0 flex-col gap-1 text-[13px] leading-6 text-ink-muted"
      role={runId ? "button" : undefined}
      title={runId ? i18n.t("agent:activity.inspectRun") : undefined}
      onClick={runId ? () => { setOpen(value => !value); handleInspect(); } : undefined}
    >
      <button
        type="button"
        className="pointer-events-none flex min-w-0 cursor-pointer items-center gap-2 text-left"
        aria-expanded={open}
      >
        {renderActivityIcon(item.kind, false)}
        <span className="shrink-0">{label}</span>
        <Chevron className="-ml-1 size-3 shrink-0" />
      </button>
      {open
        ? (
            <div className="flex flex-col gap-1 pl-6">
              {children.map(child => (
                <div
                  className="flex min-w-0 cursor-pointer items-center gap-2 hover:text-ink"
                  key={child.id}
                  role="button"
                  title={i18n.t("agent:activity.inspectRun")}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (runId)
                      emitFutureEvent("inspect-tool", { runId, toolId: child.id });
                  }}
                >
                  {renderActivityIcon(child.kind, false)}
                  <span
                    className="min-w-0 select-text truncate font-mono"
                    title={child.detail ?? child.target}
                  >
                    {child.target ? relativizeTarget(child.kind, child.target, workspacePath) : ""}
                  </span>
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

  const prefix = statusPrefix(item.status);
  if (count > 1) {
    if (item.kind === "shell")
      return i18n.t("agent:activity.runCommands", { prefix, count });
    if (item.kind === "write")
      return i18n.t("agent:activity.writeFiles", { prefix, count });
    if (item.kind === "read")
      return i18n.t("agent:activity.readFiles", { prefix, count });
    return i18n.t("agent:activity.editFiles", { prefix, count });
  }

  switch (item.kind) {
    case "read":
      return i18n.t("agent:activity.read", { prefix });
    case "shell":
      return i18n.t("agent:activity.run", { prefix });
    case "write":
      return i18n.t("agent:activity.write", { prefix });
    case "edit":
      return i18n.t("agent:activity.edit", { prefix });
  }
}

function statusPrefix(status: AgentActivityItem["status"]) {
  switch (status) {
    case "running":
      return i18n.t("agent:activity.prefix.running");
    case "completed":
      return i18n.t("agent:activity.prefix.completed");
    case "failed":
      return "";
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
