import type { AgentActivityItem, AgentActivityKind } from "./agentThreadTypes";
import { Brain, ChevronLeft, ChevronRight, FileText, Pencil, TerminalSquare } from "lucide-react";
import { useState } from "react";
import i18n from "../../i18n";
import { cn } from "../../lib/cn";

interface AgentActivityListProps {
  items?: AgentActivityItem[];
  workspacePath?: string | null;
}

export function AgentActivityList({ items, workspacePath }: AgentActivityListProps) {
  const visibleItems = items?.filter(item => item.status === "running" || item.status === "completed" || item.status === "failed") ?? [];
  if (visibleItems.length === 0)
    return null;

  return (
    <div className="my-4 space-y-3">
      {visibleItems.map(item => (
        <AgentActivityLine item={item} key={item.id} workspacePath={workspacePath} />
      ))}
    </div>
  );
}

export function AgentActivityLine({ item, workspacePath }: { item: AgentActivityItem; workspacePath?: string | null }) {
  const label = labelForActivity(item);
  const failed = item.status === "failed";
  const running = item.status === "running";
  const displayTarget = item.target ? relativizeTarget(item.kind, item.target, workspacePath) : undefined;
  // The path is hidden by default to keep the transcript quiet; clicking the
  // icon+label toggles it. Chevron points right (expand) when collapsed, left
  // (collapse) when open.
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronLeft : ChevronRight;

  return (
    <div
      className={cn(
        // One uniform size for icon + label + target so the row reads as a
        // single line; `items-center` keeps the mono target vertically centred
        // against the sans label.
        "flex min-w-0 items-center gap-2 text-[13px] leading-6",
        failed ? "text-danger" : "text-ink-muted",
      )}
    >
      {displayTarget
        ? (
            <button
              type="button"
              onClick={() => setOpen(value => !value)}
              className="flex shrink-0 cursor-pointer items-center gap-2"
              aria-expanded={open}
            >
              {renderActivityIcon(item.kind, running)}
              <span>{label}</span>
              <Chevron className="-ml-2 size-3 shrink-0" />
            </button>
          )
        : (
            <>
              {renderActivityIcon(item.kind, running)}
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

/**
 * Files inside the active workspace show as a workspace-relative path; anything
 * outside keeps its absolute path so it stays unambiguous. Bash targets are the
 * command itself, never a path, so they're left untouched.
 */
function relativizeTarget(kind: AgentActivityKind, target: string, workspacePath?: string | null) {
  if (kind === "bash" || !workspacePath)
    return target;

  const root = workspacePath.replace(/\/+$/, "");
  if (target === root)
    return target;
  if (target.startsWith(`${root}/`))
    return target.slice(root.length + 1);
  return target;
}

function renderActivityIcon(kind: AgentActivityKind, running: boolean) {
  const className = cn("size-3.5 shrink-0", running && kind === "thinking" && "animate-pulse");
  switch (kind) {
    case "bash":
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
    if (item.kind === "bash")
      return i18n.t("agent:activity.runCommands", { prefix, count });
    if (item.kind === "write")
      return i18n.t("agent:activity.writeFiles", { prefix, count });
    return i18n.t("agent:activity.editFiles", { prefix, count });
  }

  switch (item.kind) {
    case "read":
      return i18n.t("agent:activity.read", { prefix });
    case "bash":
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
    case "bash":
      return i18n.t("agent:activity.failed.bash");
    case "edit":
      return i18n.t("agent:activity.failed.edit");
    case "read":
      return i18n.t("agent:activity.failed.read");
    case "write":
      return i18n.t("agent:activity.failed.write");
  }
}
