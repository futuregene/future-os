import type { StoredRun, StoredRunEvent, StoredToolCall, StoredToolOutput } from "../../../integrations/storage/threadStore";
import { Activity, ChevronDown, ChevronRight, Wrench } from "lucide-react";
import { useState } from "react";
import {

  storedTimeToIso,

} from "../../../integrations/storage/threadStore";
import { cn } from "../../../lib/cn";
import { formatTime } from "../../../lib/date";
import { Badge } from "../../ui/Badge";
import { EmptyState } from "./ContextEmptyState";
import { formatRunStatus, runTone, shortId, summarizePayload } from "./contextPanelFormatters";

interface RunsPanelProps {
  eventsByRun: Record<string, StoredRunEvent[]>;
  outputsByTool: Record<string, StoredToolOutput[]>;
  runs: StoredRun[];
  toolsByRun: Record<string, StoredToolCall[]>;
}

export function RunsPanel({ eventsByRun, outputsByTool, runs, toolsByRun }: RunsPanelProps) {
  if (runs.length === 0) {
    return <EmptyState title="No runs yet" detail="Send a message to create the first Agent run." />;
  }

  return (
    <div className="space-y-3">
      {runs.map(run => (
        <RunCard
          key={run.id}
          events={eventsByRun[run.id] ?? []}
          outputsByTool={outputsByTool}
          run={run}
          tools={toolsByRun[run.id] ?? []}
        />
      ))}
    </div>
  );
}

function RunCard({
  events,
  outputsByTool,
  run,
  tools,
}: {
  events: StoredRunEvent[];
  outputsByTool: Record<string, StoredToolOutput[]>;
  run: StoredRun;
  tools: StoredToolCall[];
}) {
  const [open, setOpen] = useState(run.status === "running" || run.status === "failed");

  return (
    <div className="rounded-md border border-line-soft bg-surface">
      <button
        className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left"
        onClick={() => setOpen(value => !value)}
        type="button"
      >
        <span className="flex min-w-0 items-center gap-2">
          <Activity className="size-4 shrink-0 text-accent" />
          <span className="min-w-0 truncate text-sm font-medium text-ink">{shortId(run.id)}</span>
          <Badge tone={runTone(run.status)}>{formatRunStatus(run.status)}</Badge>
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
              <div className="mb-3 grid grid-cols-2 gap-2 text-xs text-ink-muted">
                <div>
                  Started
                  {run.startedAt ? formatTime(storedTimeToIso(run.startedAt)) : "-"}
                </div>
                <div>
                  {tools.length}
                  {" "}
                  tools /
                  {events.length}
                  {" "}
                  events
                </div>
              </div>
              {run.errorMessage
                ? (
                    <div className="mb-3 rounded-md border border-red-200 bg-red-50 p-2 text-xs leading-5 text-red-700">
                      {run.errorMessage}
                    </div>
                  )
                : null}
              {tools.length > 0
                ? (
                    <div className="mb-3 space-y-2">
                      {tools.map(tool => (
                        <ToolCallItem key={tool.id} outputs={outputsByTool[tool.id] ?? []} tool={tool} />
                      ))}
                    </div>
                  )
                : null}
              <div className="space-y-2 border-t border-line-soft pt-3">
                {events.length === 0 ? <div className="text-xs text-ink-muted">No events recorded.</div> : null}
                {events.map(event => (
                  <div key={event.id} className="grid grid-cols-[16px_1fr] gap-2">
                    <span className="mt-1 size-2 rounded-full bg-accent-soft ring-1 ring-accent" />
                    <div className="min-w-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate text-xs font-medium text-ink">{event.eventType}</span>
                        <span className="shrink-0 text-[11px] text-ink-muted">
                          {formatTime(storedTimeToIso(event.createdAt))}
                        </span>
                      </div>
                      {event.payload
                        ? (
                            <pre className="mt-1 max-h-28 overflow-auto rounded-md bg-surface-subtle p-2 text-[11px] leading-4 text-ink-soft">
                              <code>{summarizePayload(event.payload)}</code>
                            </pre>
                          )
                        : null}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )
        : null}
    </div>
  );
}

function ToolCallItem({ outputs, tool }: { outputs: StoredToolOutput[]; tool: StoredToolCall }) {
  const [open, setOpen] = useState(tool.status === "running" || tool.status === "failed");

  return (
    <div className="rounded-md border border-line-soft bg-surface-subtle">
      <button
        className="flex w-full items-center justify-between gap-2 px-2.5 py-2 text-left"
        onClick={() => setOpen(value => !value)}
        type="button"
      >
        <span className="flex min-w-0 items-center gap-2">
          <Wrench className="size-3.5 shrink-0 text-ink-soft" />
          <span className="min-w-0 truncate text-xs font-semibold text-ink">{tool.name}</span>
          <Badge tone={tool.status === "failed" ? "danger" : tool.status === "running" ? "accent" : "neutral"}>
            {tool.status}
          </Badge>
        </span>
        {open
          ? (
              <ChevronDown className="size-3.5 text-ink-muted" />
            )
          : (
              <ChevronRight className="size-3.5 text-ink-muted" />
            )}
      </button>
      {open
        ? (
            <div className="border-t border-line-soft px-2.5 py-2">
              {tool.input
                ? (
                    <pre className="max-h-28 overflow-auto rounded-md bg-surface p-2 text-[11px] leading-4 text-ink-soft">
                      <code>{summarizePayload(tool.input)}</code>
                    </pre>
                  )
                : null}
              {outputs.length > 0
                ? (
                    <div className="mt-2 space-y-2">
                      {outputs.map(output => (
                        <pre
                          className={cn(
                            "max-h-32 overflow-auto rounded-md p-2 text-[11px] leading-4",
                            output.kind === "error" ? "bg-red-50 text-red-700" : "bg-surface text-ink-soft",
                          )}
                          key={output.id}
                        >
                          <code>{summarizePayload(output.content ?? "")}</code>
                        </pre>
                      ))}
                    </div>
                  )
                : null}
              {!tool.input && outputs.length === 0
                ? (
                    <div className="text-xs text-ink-muted">No structured payload recorded.</div>
                  )
                : null}
            </div>
          )
        : null}
    </div>
  );
}
