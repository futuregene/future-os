import type { AgentActivityItem, AgentActivityKind } from "./agentThreadTypes";
import { Brain, FileText, Pencil, TerminalSquare } from "lucide-react";
import i18n from "../../i18n";
import { cn } from "../../lib/cn";

interface AgentActivityListProps {
  items?: AgentActivityItem[];
}

export function AgentActivityList({ items }: AgentActivityListProps) {
  const visibleItems = items?.filter(item => item.status === "running" || item.status === "completed" || item.status === "failed") ?? [];
  if (visibleItems.length === 0)
    return null;

  return (
    <div className="my-4 space-y-3">
      {visibleItems.map(item => (
        <AgentActivityLine item={item} key={item.id} />
      ))}
    </div>
  );
}

export function AgentActivityLine({ item }: { item: AgentActivityItem }) {
  const label = labelForActivity(item);
  const failed = item.status === "failed";
  const running = item.status === "running";

  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-2 text-sm leading-6",
        failed ? "text-danger" : running ? "text-ink-muted" : "text-ink-soft",
      )}
    >
      {renderActivityIcon(item.kind, running)}
      <span className="shrink-0 font-medium">{label}</span>
      {item.target
        ? (
            <code
              className="min-w-0 truncate rounded-md bg-surface-subtle px-1.5 py-0.5 font-mono text-[0.9em] text-ink"
              title={item.detail ?? item.target}
            >
              {item.target}
            </code>
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

function renderActivityIcon(kind: AgentActivityKind, running: boolean) {
  const className = cn("size-4 shrink-0", running && kind === "thinking" && "animate-pulse");
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
