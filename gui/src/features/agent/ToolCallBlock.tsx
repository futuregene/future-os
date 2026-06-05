import type { ToolCall } from "./types";
import { ChevronDown, ChevronRight, TerminalSquare } from "lucide-react";
import { useState } from "react";
import { Badge } from "../../components/ui/Badge";
import { cn } from "../../lib/cn";

interface ToolCallBlockProps {
  tool: ToolCall;
}

export function ToolCallBlock({ tool }: ToolCallBlockProps) {
  const [open, setOpen] = useState(tool.status !== "completed");
  const tone = tool.status === "completed" ? "success" : tool.status === "failed" ? "danger" : "accent";

  return (
    <div className="mt-3 rounded-lg border border-line-soft bg-white">
      <button
        className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left"
        onClick={() => setOpen(value => !value)}
        type="button"
      >
        <span className="flex min-w-0 items-center gap-2">
          <TerminalSquare className="size-4 shrink-0 text-ink-muted" />
          <span className="truncate text-sm font-medium text-ink">{tool.name}</span>
          <Badge tone={tone}>{tool.status}</Badge>
        </span>
        {open
          ? (
              <ChevronDown className="size-4 text-ink-muted" />
            )
          : (
              <ChevronRight className="size-4 text-ink-muted" />
            )}
      </button>
      {open
        ? (
            <div className="border-t border-line-soft p-3">
              <p className="text-sm text-ink-soft">{tool.summary}</p>
              <pre className="mt-3 overflow-auto rounded-md border border-line-soft bg-surface-subtle p-3 text-xs leading-5 text-ink-soft">
                <code>{tool.input}</code>
              </pre>
              {tool.output
                ? (
                    <pre
                      className={cn(
                        "mt-2 overflow-auto rounded-md border border-green-200 bg-green-50 p-3 text-xs leading-5 text-green-800",
                      )}
                    >
                      <code>{tool.output}</code>
                    </pre>
                  )
                : null}
            </div>
          )
        : null}
    </div>
  );
}
