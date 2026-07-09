import type {
  StoredRun,
  StoredToolCall,
  StoredToolOutput,
} from "../../integrations/storage/threadStore";
import { ArrowLeft, RotateCcw, Search, StepForward, Terminal, Wrench } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { CopyablePre } from "../../components/ui/CopyablePre";
import { TextInput } from "../../components/ui/TextInput";
import {
  listToolOutputs,
  storedTimeToIso,
} from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { formatDuration, formatTime } from "../../lib/date";
import { emitFutureEvent } from "../../lib/futureEvents";
import { isRecord } from "../../lib/objects";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { formatRunStatus, runTone, shortId, toolStatusLabel } from "./runDisplayFormatters";
import { RunError } from "./RunError";
import { numberOrStringField, parseJsonish, recordOf, stringField } from "./toolInput";

interface RunInspectPanelProps {
  run: StoredRun;
  tools: StoredToolCall[];
  onBack: () => void;
  // Single-tool detail (opened from the Runs panel): drop the run summary /
  // search / "tool calls" chrome and let the one tool card fill the height.
  compact?: boolean;
}

interface RunDetails {
  outputsByTool: Record<string, StoredToolOutput[]>;
}

export function RunInspectPanel({ compact = false, onBack, run, tools }: RunInspectPanelProps) {
  const { i18n, t } = useTranslation("runs");
  const [query, setQuery] = useState("");
  const sortedTools = useMemo(
    () => [...tools].sort((left, right) => (left.startedAt ?? left.createdAt) - (right.startedAt ?? right.createdAt)),
    [tools],
  );
  const { data: details, error } = useAsyncResource<RunDetails>(
    async () => {
      const outputEntries = await Promise.all(sortedTools.map(async (tool) => {
        try {
          return [tool.id, await listToolOutputs(tool.id)] as const;
        }
        catch {
          return [tool.id, [] as StoredToolOutput[]] as const;
        }
      }));
      return { outputsByTool: Object.fromEntries(outputEntries) };
    },
    [sortedTools],
    { outputsByTool: {} },
  );
  const outputsByTool = details.outputsByTool;
  const filteredTools = useMemo(
    () => sortedTools.filter(tool => eventMatchesQuery(toolSearchText(tool, outputsByTool[tool.id] ?? []), query)),
    [outputsByTool, query, sortedTools],
  );
  const primaryTool = sortedTools[0];

  return (
    <div className={cn(compact ? "flex h-full min-h-0 flex-col gap-3" : "space-y-3")}>
      <button
        className="inline-flex h-8 shrink-0 items-center gap-1.5 self-start rounded-md px-1.5 text-xs font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink"
        onClick={onBack}
        type="button"
      >
        <ArrowLeft className="size-3.5" />
        {t("runInspect.back")}
      </button>

      {compact
        ? (
            primaryTool
              ? (
                  <ToolCallDetail
                    fill
                    outputs={outputsByTool[primaryTool.id] ?? []}
                    tool={primaryTool}
                  />
                )
              : <div className="rounded-md border border-dashed border-line-soft p-3 text-sm text-ink-muted">{t("runInspect.noToolCalls")}</div>
          )
        : (
            <>
              <section className="rounded-md border border-line-soft bg-surface p-3">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <h3 className="truncate text-sm font-semibold text-ink">{shortId(run.id)}</h3>
                    <div className="mt-1 text-xs text-ink-muted">
                      {run.startedAt ? formatTime(storedTimeToIso(run.startedAt), i18n.language) : formatTime(storedTimeToIso(run.createdAt), i18n.language)}
                      {run.endedAt ? ` - ${formatTime(storedTimeToIso(run.endedAt), i18n.language)}` : ""}
                    </div>
                  </div>
                  <Badge tone={runTone(run.status)}>{formatRunStatus(run.status)}</Badge>
                </div>
                <dl className="mt-3 grid grid-cols-2 gap-2 text-xs">
                  <div>
                    <dt className="text-ink-muted">{t("runInspect.model")}</dt>
                    <dd className="mt-0.5 truncate text-ink-soft">{run.modelId ?? "-"}</dd>
                  </div>
                  <div>
                    <dt className="text-ink-muted">{t("runInspect.tools")}</dt>
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
                          {t("runInspect.retry")}
                        </Button>
                        <Button
                          leftIcon={<StepForward className="size-3.5" />}
                          onClick={() => dispatchRunRecovery(run, "continue")}
                          size="sm"
                          variant="toolbar"
                        >
                          {t("runInspect.continue")}
                        </Button>
                      </div>
                    )
                  : null}
              </section>

              <section className="space-y-2">
                <label className="relative block">
                  <span className="sr-only">{t("runInspect.searchLabel")}</span>
                  <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-muted" />
                  <TextInput
                    className="h-8 pl-8 pr-2 hover:border-line"
                    onChange={event => setQuery(event.target.value)}
                    placeholder={t("runInspect.searchPlaceholder")}
                    value={query}
                  />
                </label>
              </section>

              <section className="space-y-2">
                <div className="flex items-center gap-1.5 text-xs font-medium text-ink-muted">
                  <Wrench className="size-3.5" />
                  {t("runInspect.toolCalls")}
                </div>
                {error ? <div className="rounded-md border border-danger-line bg-danger-soft p-2 text-xs leading-5 text-danger">{error}</div> : null}
                {filteredTools.length === 0
                  ? <div className="rounded-md border border-dashed border-line-soft p-3 text-sm text-ink-muted">{sortedTools.length === 0 ? t("runInspect.noToolCalls") : t("runInspect.noMatchingToolCalls")}</div>
                  : filteredTools.map(tool => (
                      <ToolCallDetail
                        key={tool.id}
                        outputs={outputsByTool[tool.id] ?? []}
                        tool={tool}
                      />
                    ))}
              </section>
            </>
          )}
    </div>
  );
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
  fill = false,
  outputs,
  tool,
}: {
  fill?: boolean;
  outputs: StoredToolOutput[];
  tool: StoredToolCall;
}) {
  const { t } = useTranslation("runs");
  const details = toolDetails(tool, outputs);
  const inputText = details.command ?? details.path ?? tool.input ?? t("runInspect.noInput");
  const rawOutputs = outputs.filter(output => !isStructuredOutput(output));
  // For write/edit the interesting artifact is what was written, not the
  // "Written to …" acknowledgement — show the content and drop the output.
  const isFileEdit = displayToolName(tool) === "write" || displayToolName(tool) === "edit";
  const writtenText = isFileEdit ? writtenContent(recordOf(tool.input)) : null;

  return (
    <div className={cn("rounded-md border border-line-soft bg-surface p-3", fill && "flex min-h-0 flex-col")}>
      <div className="flex shrink-0 items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <Terminal className="size-4 shrink-0 text-ink-muted" />
          <span className="truncate text-xs font-medium text-ink">{tool.name || t("runInspect.toolFallback")}</span>
        </div>
        <Badge tone={tool.status === "completed" ? "success" : tool.status === "failed" ? "danger" : "neutral"}>
          {toolStatusLabel(tool.status)}
        </Badge>
      </div>
      <ToolDetailFields details={details} tool={tool} />
      <div className="mt-2 shrink-0">
        <div className="mb-1 text-[11px] font-medium text-ink-muted">
          {details.command ? t("runInspect.command") : details.path ? t("runInspect.target") : t("runInspect.input")}
        </div>
        <CopyablePre maxHeightClassName="max-h-40" text={inputText} />
      </div>
      {isFileEdit
        ? (writtenText
            ? (
                <div className={cn("mt-2", fill ? "flex min-h-0 flex-col" : "shrink-0")}>
                  <div className="mb-1 shrink-0 text-[11px] font-medium text-ink-muted">{t("runInspect.content")}</div>
                  <CopyablePre fill={fill} maxHeightClassName="max-h-96" text={writtenText} />
                </div>
              )
            : null)
        : (
            <>
              {details.stdout
                ? (
                    <div className={cn("mt-2", fill && rawOutputs.length === 0 ? "flex min-h-0 flex-col" : "shrink-0")}>
                      <div className="mb-1 shrink-0 text-[11px] font-medium text-ink-muted">stdout</div>
                      <CopyablePre
                        fill={fill && rawOutputs.length === 0}
                        maxHeightClassName="max-h-40"
                        text={details.stdout}
                      />
                    </div>
                  )
                : null}
              {details.stderr
                ? (
                    <div className="mt-2 shrink-0">
                      <div className="mb-1 text-[11px] font-medium text-danger">stderr</div>
                      <CopyablePre maxHeightClassName="max-h-40" text={details.stderr} />
                    </div>
                  )
                : null}
              {rawOutputs.length > 0
                ? (
                    <div className={cn("mt-2 space-y-2", fill && "flex min-h-0 flex-col")}>
                      {rawOutputs.map(output => (
                        <ToolOutputPreview fill={fill} key={output.id} output={output} />
                      ))}
                    </div>
                  )
                : null}
            </>
          )}
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
  const { i18n, t } = useTranslation("runs");
  const fields = [
    ["kind", t("runInspect.field.kind"), tool.kind],
    ["started", t("runInspect.field.started"), tool.startedAt ? formatTime(storedTimeToIso(tool.startedAt), i18n.language) : null],
    ["ended", t("runInspect.field.ended"), tool.endedAt ? formatTime(storedTimeToIso(tool.endedAt), i18n.language) : null],
    ["duration", t("runInspect.field.duration"), details.duration],
    ["cwd", t("runInspect.field.cwd"), details.cwd],
    ["exit", t("runInspect.field.exit"), details.exitStatus],
    ["path", t("runInspect.field.path"), details.path],
  ].filter((field): field is [string, string, string] => Boolean(field[2]));

  if (fields.length === 0)
    return null;

  return (
    <dl className="mt-2 grid shrink-0 grid-cols-2 gap-2 rounded-md bg-surface-subtle p-2 text-[11px]">
      {fields.map(([key, label, value]) => (
        <div className={key === "cwd" || key === "path" ? "col-span-2 min-w-0" : "min-w-0"} key={key}>
          <dt className="text-ink-muted">{label}</dt>
          <dd className="mt-0.5 truncate text-ink-soft" title={value}>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function ToolOutputPreview({ fill = false, output }: { fill?: boolean; output: StoredToolOutput }) {
  const { t } = useTranslation("runs");
  const [expanded, setExpanded] = useState(false);
  const text = output.content ?? output.kind;
  const long = text.length > 800 || text.split("\n").length > 8;

  return (
    <div className={cn(fill && "flex min-h-0 flex-col")}>
      <div className="mb-1 flex shrink-0 items-center justify-between gap-2">
        <span className="truncate text-[11px] font-medium text-ink-muted">{outputKindLabel(output.kind, t)}</span>
        {!fill && long
          ? (
              <button
                className="h-6 rounded px-1.5 text-[11px] font-medium text-ink-muted transition-colors hover:bg-surface hover:text-ink"
                onClick={() => setExpanded(value => !value)}
                type="button"
              >
                {expanded ? t("runInspect.collapse") : t("runInspect.expand")}
              </button>
            )
          : null}
      </div>
      <CopyablePre
        fill={fill}
        maxHeightClassName={expanded ? "max-h-96" : "max-h-32"}
        text={text}
      />
    </div>
  );
}

function displayToolName(tool: StoredToolCall) {
  return tool.name.trim().toLowerCase();
}

/**
 * The content a write/edit tool put on disk: `content` for write, the new text
 * for edit (single `newText`/`newString`, or every hunk of an `edits[]` batch).
 */
function writtenContent(input: Record<string, unknown> | null): string | null {
  const direct = stringField(input, ["content", "newText", "newString", "new_string", "newContent", "new_content"]);
  if (direct)
    return direct;

  const edits = input?.edits;
  if (Array.isArray(edits)) {
    const parts = edits
      .map(edit => (isRecord(edit) ? stringField(edit, ["newText", "newString", "new_string"]) : null))
      .filter((part): part is string => Boolean(part));
    if (parts.length > 0)
      return parts.join("\n\n");
  }
  return null;
}

/** Human-readable label for a tool output's `kind` (`text` → Output, …). */
function outputKindLabel(kind: string, t: (key: string) => string) {
  if (kind === "text")
    return t("runInspect.output");
  if (kind === "error")
    return t("runInspect.outputError");
  return kind;
}

function toolDetails(tool: StoredToolCall, outputs: StoredToolOutput[]): ToolDetails {
  const input = recordOf(tool.input);
  const outputObjects = outputs.map(output => toolOutputObject(output)).filter(isRecord);
  const firstOutput = firstRecord(outputObjects);
  const durationMs = tool.startedAt && tool.endedAt ? tool.endedAt - tool.startedAt : null;

  return {
    command: stringField(input, ["command", "cmd", "shellCommand", "shell_command"]),
    cwd: stringField(input, ["cwd", "workingDirectory", "working_directory", "workdir"])
      ?? stringField(firstOutput, ["cwd", "workingDirectory", "working_directory", "workdir"]),
    duration: durationMs !== null ? formatDuration(durationMs, { subSecond: true }) : durationField(firstOutput),
    exitStatus: numberOrStringField(firstOutput, ["exitStatus", "exit_status", "exitCode", "exit_code", "statusCode", "status_code"]),
    path: stringField(input, ["path", "filePath", "file_path", "targetPath", "target_path", "target"]),
    stderr: stringField(firstOutput, ["stderr", "standardError", "standard_error", "error"]),
    stdout: stringField(firstOutput, ["stdout", "standardOutput", "standard_output", "text", "result"]),
  };
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
  return Number.isFinite(numeric) ? formatDuration(numeric, { subSecond: true }) : raw;
}
