import type { AgentActivityItem, AgentActivityKind } from "./agentThreadTypes";
import { Brain, FileText, Pencil, TerminalSquare } from "lucide-react";
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

function AgentActivityLine({ item }: { item: AgentActivityItem }) {
  const label = labelForActivity(item);
  const failed = item.status === "failed";
  const running = item.status === "running";

  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-2 text-sm leading-6",
        failed ? "text-red-500" : running ? "text-ink-muted" : "text-ink-soft",
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
    return "正在思考";

  const count = item.count ?? 0;
  if (item.status === "failed")
    return failedLabel(item.kind);

  const prefix = statusPrefix(item.status);
  if (count > 1) {
    if (item.kind === "bash")
      return `${prefix}运行 ${count} 条命令`;
    if (item.kind === "write")
      return `${prefix}写入 ${count} 个文件`;
    return `${prefix}编辑 ${count} 个文件`;
  }

  switch (item.kind) {
    case "read":
      return `${prefix}读取`;
    case "bash":
      return `${prefix}运行`;
    case "write":
      return `${prefix}写入`;
    case "edit":
      return `${prefix}编辑`;
  }
}

function statusPrefix(status: AgentActivityItem["status"]) {
  switch (status) {
    case "running":
      return "正在";
    case "completed":
      return "已";
    case "failed":
      return "";
  }
}

function failedLabel(kind: Exclude<AgentActivityKind, "thinking">) {
  switch (kind) {
    case "bash":
      return "运行失败";
    case "edit":
      return "编辑失败";
    case "read":
      return "读取失败";
    case "write":
      return "写入失败";
  }
}
