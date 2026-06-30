import type {
  StoredRun,
  StoredRunEvent,
  StoredToolCall,
  StoredToolOutput,
} from "../../integrations/storage/threadStore";
import { ArrowLeft, History, RotateCcw, Search, StepForward, Terminal, Wrench } from "lucide-react";
import { useMemo, useState } from "react";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { CopyablePre } from "../../components/ui/CopyablePre";
import { Select } from "../../components/ui/Select";
import {
  listRunEvents,
  listToolOutputs,
  storedTimeToIso,
} from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { formatTime } from "../../lib/date";
import { emitFutureEvent } from "../../lib/futureEvents";
import { isRecord } from "../../lib/objects";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { formatRunStatus, runTone, shortId, summarizePayload } from "./runDisplayFormatters";
import { RunError } from "./RunError";

interface RunInspectPanelProps {
  run: StoredRun;
  tools: StoredToolCall[];
  onBack: () => void;
}

interface RunDetails {
  events: StoredRunEvent[];
  outputsByTool: Record<string, StoredToolOutput[]>;
}

type EventFilter = "all" | "approval" | "artifact" | "error" | "review" | "text" | "tool";

const eventFilters: Array<{ label: string; value: EventFilter }> = [
  { label: "All", value: "all" },
  { label: "Text", value: "text" },
  { label: "Tools", value: "tool" },
  { label: "Approvals", value: "approval" },
  { label: "Review", value: "review" },
  { label: "Artifacts", value: "artifact" },
  { label: "Errors", value: "error" },
];

export function RunInspectPanel({ onBack, run, tools }: RunInspectPanelProps) {
  const [eventFilter, setEventFilter] = useState<EventFilter>("all");
  const [query, setQuery] = useState("");
  const sortedTools = useMemo(
    () => [...tools].sort((left, right) => (left.startedAt ?? left.createdAt) - (right.startedAt ?? right.createdAt)),
    [tools],
  );
  const { data: details, error, loading } = useAsyncResource<RunDetails>(
    async () => {
      const [nextEvents, outputEntries] = await Promise.all([
        listRunEvents(run.id),
        Promise.all(sortedTools.map(async (tool) => {
          try {
            return [tool.id, await listToolOutputs(tool.id)] as const;
          }
          catch {
            return [tool.id, [] as StoredToolOutput[]] as const;
          }
        })),
      ]);
      return { events: nextEvents, outputsByTool: Object.fromEntries(outputEntries) };
    },
    [run.id, sortedTools],
    { events: [], outputsByTool: {} },
  );
  const events = details.events;
  const outputsByTool = details.outputsByTool;
  const filteredEvents = useMemo(
    () => events
      .filter(event => eventMatchesFilter(event, eventFilter))
      .filter(event => eventMatchesQuery(eventSearchText(event), query)),
    [eventFilter, events, query],
  );
  const filteredTools = useMemo(
    () => sortedTools.filter(tool => eventMatchesQuery(toolSearchText(tool, outputsByTool[tool.id] ?? []), query)),
    [outputsByTool, query, sortedTools],
  );

  return (
    <div className="space-y-3">
      <button
        className="inline-flex h-8 items-center gap-1.5 rounded-md px-1.5 text-xs font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink"
        onClick={onBack}
        type="button"
      >
        <ArrowLeft className="size-3.5" />
        Runs
      </button>

      <section className="rounded-md border border-line-soft bg-surface p-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h3 className="truncate text-sm font-semibold text-ink">{shortId(run.id)}</h3>
            <div className="mt-1 text-xs text-ink-muted">
              {run.startedAt ? formatTime(storedTimeToIso(run.startedAt)) : formatTime(storedTimeToIso(run.createdAt))}
              {run.endedAt ? ` - ${formatTime(storedTimeToIso(run.endedAt))}` : ""}
            </div>
          </div>
          <Badge tone={runTone(run.status)}>{formatRunStatus(run.status)}</Badge>
        </div>
        <dl className="mt-3 grid grid-cols-2 gap-2 text-xs">
          <div>
            <dt className="text-ink-muted">Model</dt>
            <dd className="mt-0.5 truncate text-ink-soft">{run.modelId ?? "-"}</dd>
          </div>
          <div>
            <dt className="text-ink-muted">Tools</dt>
            <dd className="mt-0.5 text-ink-soft">{sortedTools.length}</dd>
          </div>
        </dl>
        {run.errorMessage ? <RunError errorMessage={run.errorMessage} errorType={run.errorType} variant="banner" /> : null}
        {canRecoverRun(run)
          ? (
              <div className="mt-3 flex flex-wrap gap-2">
                <Button
                  leftIcon={<RotateCcw className="size-3.5" />}
                  onClick={() => dispatchRunRecovery(run, "retry")}
                  size="sm"
                  variant="toolbar"
                >
                  Retry
                </Button>
                <Button
                  leftIcon={<StepForward className="size-3.5" />}
                  onClick={() => dispatchRunRecovery(run, "continue")}
                  size="sm"
                  variant="toolbar"
                >
                  Continue
                </Button>
              </div>
            )
          : null}
      </section>

      <section className="space-y-2">
        <label className="relative block">
          <span className="sr-only">Search run details</span>
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-muted" />
          <input
            className="h-8 w-full rounded-md border border-line-soft bg-surface pl-8 pr-2 text-sm text-ink outline-none transition-colors placeholder:text-ink-muted hover:border-line focus:border-focus focus:ring-2 focus:ring-focus"
            onChange={event => setQuery(event.target.value)}
            placeholder="Search run details..."
            value={query}
          />
        </label>
      </section>

      <section className="space-y-2">
        <div className="flex items-center gap-1.5 text-xs font-medium text-ink-muted">
          <Wrench className="size-3.5" />
          Tool Calls
        </div>
        {error ? <div className="rounded-md border border-danger-line bg-danger-soft p-2 text-xs leading-5 text-danger">{error}</div> : null}
        {filteredTools.length === 0
          ? <div className="rounded-md border border-dashed border-line-soft p-3 text-sm text-ink-muted">{sortedTools.length === 0 ? "No tool calls recorded." : "No matching tool calls."}</div>
          : filteredTools.map(tool => (
              <ToolCallDetail
                key={tool.id}
                outputs={outputsByTool[tool.id] ?? []}
                tool={tool}
              />
            ))}
      </section>

      <section className="space-y-2">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-1.5 text-xs font-medium text-ink-muted">
            <History className="size-3.5" />
            Timeline
          </div>
          <Select
            aria-label="Filter run events"
            className="w-auto text-ink-soft"
            onChange={event => setEventFilter(event.target.value as EventFilter)}
            size="xs"
            value={eventFilter}
            wrapperClassName="w-auto"
          >
            {eventFilters.map(filter => (
              <option key={filter.value} value={filter.value}>{filter.label}</option>
            ))}
          </Select>
        </div>
        {loading
          ? <div className="rounded-md border border-line-soft bg-surface p-3 text-sm text-ink-muted">Loading run details...</div>
          : filteredEvents.length === 0
            ? <div className="rounded-md border border-dashed border-line-soft p-3 text-sm text-ink-muted">No events recorded.</div>
            : filteredEvents.map(event => <TimelineEvent event={event} key={event.id} />)}
      </section>
    </div>
  );
}

function TimelineEvent({ event }: { event: StoredRunEvent }) {
  const [expanded, setExpanded] = useState(false);
  const payload = event.payload ?? "";
  const category = eventCategory(event.eventType);

  return (
    <div className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <span className={eventCategoryClass(category)}>{category}</span>
          <div className="truncate text-xs font-medium text-ink">{event.eventType}</div>
        </div>
        <div className="text-[11px] text-ink-muted">{formatTime(storedTimeToIso(event.createdAt))}</div>
      </div>
      {payload
        ? (
            <>
              <pre
                className={cn(
                  "mt-2 overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 text-[11px] leading-4 text-ink-soft",
                  expanded ? "max-h-96" : "max-h-40",
                )}
              >
                <code>{expanded ? payload : summarizePayload(payload)}</code>
              </pre>
              <button
                className="mt-2 h-6 rounded px-1.5 text-[11px] font-medium text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink"
                onClick={() => setExpanded(value => !value)}
                type="button"
              >
                {expanded ? "Show summary" : "Show raw payload"}
              </button>
            </>
          )
        : null}
    </div>
  );
}

function eventMatchesFilter(event: StoredRunEvent, filter: EventFilter) {
  if (filter === "all")
    return true;

  return eventCategory(event.eventType) === filter;
}

function eventCategory(eventType: string): Exclude<EventFilter, "all"> {
  const type = eventType.toLowerCase();
  if (type.includes("approval"))
    return "approval";
  if (type.includes("artifact"))
    return "artifact";
  if (type.includes("review") || type.includes("diff"))
    return "review";
  if (type.includes("error") || type.includes("fail"))
    return "error";
  if (type.includes("tool"))
    return "tool";
  return "text";
}

function eventCategoryClass(category: Exclude<EventFilter, "all">) {
  const base = "shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium";
  switch (category) {
    case "approval":
      return `${base} bg-warning-soft text-warning`;
    // artifact/review have no semantic equivalent — intentional category colors (COLOR.md).
    case "artifact":
      return `${base} bg-purple-50 text-purple-700`;
    case "error":
      return `${base} bg-danger-soft text-danger`;
    case "review":
      return `${base} bg-orange-50 text-orange-700`;
    case "tool":
      return `${base} bg-info-soft text-info`;
    default:
      return `${base} bg-surface-subtle text-ink-muted`;
  }
}

function eventSearchText(event: StoredRunEvent) {
  return `${event.eventType}\n${event.payload ?? ""}`;
}

function toolSearchText(tool: StoredToolCall, outputs: StoredToolOutput[]) {
  return [
    tool.name,
    tool.kind,
    tool.status,
    tool.input ?? "",
    ...outputs.map(output => `${output.kind}\n${output.content ?? ""}`),
  ].join("\n");
}

function eventMatchesQuery(value: string, query: string) {
  const normalized = query.trim().toLowerCase();
  if (!normalized)
    return true;

  return value.toLowerCase().includes(normalized);
}

function canRecoverRun(run: StoredRun) {
  return run.status === "failed" || run.status === "cancelled";
}

function dispatchRunRecovery(run: StoredRun, action: "continue" | "retry") {
  emitFutureEvent("recover-run", { action, runId: run.id, triggerMessageId: run.triggerMessageId });
}

function ToolCallDetail({
  outputs,
  tool,
}: {
  outputs: StoredToolOutput[];
  tool: StoredToolCall;
}) {
  const details = toolDetails(tool, outputs);
  const inputText = details.command ?? details.path ?? tool.input ?? "No input";
  const rawOutputs = outputs.filter(output => !isStructuredOutput(output));

  return (
    <div className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        <Terminal className="mt-0.5 size-4 shrink-0 text-ink-muted" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <div className="truncate text-xs font-medium text-ink">{tool.name || "Tool"}</div>
            <Badge tone={tool.status === "completed" ? "success" : tool.status === "failed" ? "danger" : "neutral"}>
              {tool.status}
            </Badge>
          </div>
          <ToolDetailFields details={details} tool={tool} />
          <div className="mt-2">
            <div className="mb-1 text-[11px] font-medium text-ink-muted">
              {details.command ? "Command" : details.path ? "Target" : "Input"}
            </div>
            <CopyablePre maxHeightClassName="max-h-40" text={inputText} />
          </div>
          {details.stdout
            ? (
                <div className="mt-2">
                  <div className="mb-1 text-[11px] font-medium text-ink-muted">stdout</div>
                  <CopyablePre maxHeightClassName="max-h-40" text={details.stdout} />
                </div>
              )
            : null}
          {details.stderr
            ? (
                <div className="mt-2">
                  <div className="mb-1 text-[11px] font-medium text-danger">stderr</div>
                  <CopyablePre maxHeightClassName="max-h-40" text={details.stderr} />
                </div>
              )
            : null}
          {rawOutputs.length > 0
            ? (
                <div className="mt-2 space-y-2">
                  {rawOutputs.map(output => (
                    <ToolOutputPreview key={output.id} output={output} />
                  ))}
                </div>
              )
            : null}
        </div>
      </div>
    </div>
  );
}

interface ToolDetails {
  command?: string | null;
  cwd?: string | null;
  duration?: string | null;
  exitStatus?: string | null;
  path?: string | null;
  stderr?: string | null;
  stdout?: string | null;
}

function ToolDetailFields({
  details,
  tool,
}: {
  details: ToolDetails;
  tool: StoredToolCall;
}) {
  const fields = [
    ["Kind", tool.kind],
    ["Started", tool.startedAt ? formatTime(storedTimeToIso(tool.startedAt)) : null],
    ["Ended", tool.endedAt ? formatTime(storedTimeToIso(tool.endedAt)) : null],
    ["Duration", details.duration],
    ["CWD", details.cwd],
    ["Exit", details.exitStatus],
    ["Path", details.path],
  ].filter((field): field is [string, string] => Boolean(field[1]));

  if (fields.length === 0)
    return null;

  return (
    <dl className="mt-2 grid grid-cols-2 gap-2 rounded-md bg-surface-subtle p-2 text-[11px]">
      {fields.map(([label, value]) => (
        <div className={label === "CWD" || label === "Path" ? "col-span-2 min-w-0" : "min-w-0"} key={label}>
          <dt className="text-ink-muted">{label}</dt>
          <dd className="mt-0.5 truncate text-ink-soft" title={value}>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function ToolOutputPreview({ output }: { output: StoredToolOutput }) {
  const [expanded, setExpanded] = useState(false);
  const text = output.content ?? output.kind;
  const long = text.length > 800 || text.split("\n").length > 8;

  return (
    <div className="rounded-md bg-surface-subtle">
      <div className="flex items-center justify-between gap-2 px-2 pt-2">
        <span className="truncate text-[11px] font-medium text-ink-muted">{output.kind}</span>
        {long
          ? (
              <button
                className="h-6 rounded px-1.5 text-[11px] font-medium text-ink-muted transition-colors hover:bg-surface hover:text-ink"
                onClick={() => setExpanded(value => !value)}
                type="button"
              >
                {expanded ? "Collapse" : "Expand"}
              </button>
            )
          : null}
      </div>
      <CopyablePre
        className="mt-1"
        maxHeightClassName={expanded ? "max-h-96" : "max-h-32"}
        text={text}
      />
    </div>
  );
}

function toolDetails(tool: StoredToolCall, outputs: StoredToolOutput[]): ToolDetails {
  const input = toolInputObject(tool.input);
  const outputObjects = outputs.map(output => toolOutputObject(output)).filter(isRecord);
  const firstOutput = firstRecord(outputObjects);
  const durationMs = tool.startedAt && tool.endedAt ? tool.endedAt - tool.startedAt : null;

  return {
    command: stringField(input, ["command", "cmd", "shellCommand", "shell_command"]),
    cwd: stringField(input, ["cwd", "workingDirectory", "working_directory", "workdir"])
      ?? stringField(firstOutput, ["cwd", "workingDirectory", "working_directory", "workdir"]),
    duration: durationMs !== null ? formatDuration(durationMs) : durationField(firstOutput),
    exitStatus: numberOrStringField(firstOutput, ["exitStatus", "exit_status", "exitCode", "exit_code", "statusCode", "status_code"]),
    path: stringField(input, ["path", "filePath", "file_path", "targetPath", "target_path", "target"]),
    stderr: stringField(firstOutput, ["stderr", "standardError", "standard_error", "error"]),
    stdout: stringField(firstOutput, ["stdout", "standardOutput", "standard_output", "text", "result"]),
  };
}

function toolInputObject(input: string | null | undefined) {
  const value = parseJsonish(input);
  return isRecord(value) ? value : null;
}

function toolOutputObject(output: StoredToolOutput) {
  return parseJsonish(output.content);
}

function isStructuredOutput(output: StoredToolOutput) {
  const value = toolOutputObject(output);
  if (!isRecord(value))
    return false;
  return Boolean(
    stringField(value, ["stdout", "standardOutput", "standard_output", "stderr", "standardError", "standard_error"])
    || numberOrStringField(value, ["exitStatus", "exit_status", "exitCode", "exit_code", "statusCode", "status_code"]),
  );
}

function firstRecord(values: unknown[]) {
  return values.find(isRecord) ?? null;
}

function durationField(value: Record<string, unknown> | null) {
  const raw = numberOrStringField(value, ["durationMs", "duration_ms", "elapsedMs", "elapsed_ms", "duration"]);
  if (!raw)
    return null;
  const numeric = Number(raw);
  return Number.isFinite(numeric) ? formatDuration(numeric) : raw;
}

function numberOrStringField(value: Record<string, unknown> | null, keys: string[]) {
  if (!value)
    return null;
  for (const key of keys) {
    const field = value[key];
    if (typeof field === "string" && field.trim())
      return field;
    if (typeof field === "number" && Number.isFinite(field))
      return String(field);
  }
  return null;
}

function stringField(value: Record<string, unknown> | null, keys: string[]) {
  if (!value)
    return null;
  for (const key of keys) {
    const field = value[key];
    if (typeof field === "string" && field.trim())
      return field;
  }
  return null;
}

function parseJsonish(value: unknown) {
  let current = value;
  for (let index = 0; index < 3; index += 1) {
    if (isRecord(current))
      return current;
    if (typeof current !== "string")
      return current;
    const trimmed = current.trim();
    if (!trimmed)
      return null;
    try {
      current = JSON.parse(trimmed) as unknown;
    }
    catch {
      return current;
    }
  }
  return current;
}

function formatDuration(milliseconds: number) {
  if (milliseconds < 1000)
    return `${milliseconds}ms`;
  if (milliseconds < 60_000)
    return `${(milliseconds / 1000).toFixed(1)}s`;
  const minutes = Math.floor(milliseconds / 60_000);
  const seconds = Math.round((milliseconds % 60_000) / 1000);
  return `${minutes}m ${seconds}s`;
}
