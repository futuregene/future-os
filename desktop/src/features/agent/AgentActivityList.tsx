import type { AgentActivityItem, AgentActivityKind } from "@future-os/thread-projection";
import { Brain, ChevronDown, ChevronUp, FileText, Pencil, TerminalSquare, TriangleAlert } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import i18n from "../../i18n";
import { cn } from "../../lib/cn";
import { emitFutureEvent } from "../../lib/futureEvents";
import { relativizeWorkspacePath } from "../../lib/workspacePath";
import { FusionActivity } from "./FusionActivity";

/**
 * Where a row's target comes from when the row arrived without one.
 *
 * A remote transcript is read from a lean history page, which gives a shell call
 * its identity and no arguments — the command is on the other machine and this
 * is how it is asked for. The row asks *when the reader opens it*, which is the
 * same moment and the same one command the phone spends: a page of a real
 * conversation holds dozens of calls, and reading them all on open cost one
 * round trip per call before the first line could be drawn.
 */
export type ActivityTargetResolver = (input: {
  runId: string;
  toolCallId: string;
  fusion?: boolean;
  stepId?: string;
  offset?: number;
}) => Promise<string | null>;

interface AgentActivityListProps {
  items?: AgentActivityItem[];
  workspacePath?: string | null;
  runId?: string | null;
  /** Where a row's target comes from when the row arrived without one. */
  resolveTarget?: ActivityTargetResolver;
}

export function AgentActivityList({ items, resolveTarget, workspacePath, runId }: AgentActivityListProps) {
  const visibleItems = items?.filter(item => item.status === "running" || item.status === "completed" || item.status === "failed") ?? [];
  if (visibleItems.length === 0)
    return null;

  return (
    <div className="my-4 space-y-3">
      {visibleItems.map(item => (
        <AgentActivityLine item={item} key={item.id} resolveTarget={resolveTarget} workspacePath={workspacePath} runId={runId} />
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
  resolveTarget?: ActivityTargetResolver;
}

export function AgentActivityLine({ item, resolveTarget, workspacePath, runId }: AgentActivityLineProps) {
  if (item.kind === "fusion")
    return <FusionActivity item={item} runId={runId} resolveTarget={resolveTarget} />;
  if ((item.children?.length ?? 0) > 0)
    return <AgentActivityGroupLine item={item} resolveTarget={resolveTarget} workspacePath={workspacePath} runId={runId} />;
  return <AgentActivitySingleLine item={item} resolveTarget={resolveTarget} workspacePath={workspacePath} runId={runId} />;
}

/** The target to draw for an item: its own, or what an expanded row fetched. */
function shownTarget(item: AgentActivityItem, resolved: string | null): string | null {
  const own = item.target?.trim();
  return own ? item.target! : resolved;
}

/** A call that has to be asked for, because the row carries no target. */
function missingCalls(item: AgentActivityItem): { runId: string | null; toolCallId: string }[] {
  if ((item.target ?? "").trim() !== "" || !item.toolCallId)
    return [];
  return [{ runId: item.runId ?? null, toolCallId: item.toolCallId }];
}

/**
 * A tool call's target, fetched when the row is opened.
 *
 * Nothing is asked while the row is collapsed: a lean page gives a shell call its
 * identity and no arguments, and the command it ran is a *read* the reader pays
 * for by opening the row — the phone's own bargain, and the reason a conversation
 * opens on one page rather than on one request per call it happens to contain.
 *
 * A host that cannot answer leaves the row as it is: the label is still the truth
 * about the call, and the fetch behind `resolveTarget` remembers a failure rather
 * than asking again on every render or every re-open.
 */
function useActivityTarget(input: {
  enabled: boolean;
  item: AgentActivityItem;
  resolveTarget: ActivityTargetResolver | undefined;
  runId?: string | null;
}): string | null {
  const { enabled, item, resolveTarget, runId } = input;
  const [resolved, setResolved] = useState<string | null>(null);
  const toolCallId = item.toolCallId;
  // The run the row belongs to is the item's own where it has one; the enclosing
  // message's run is the fallback a projected live reply relies on.
  const callRunId = item.runId ?? runId;
  // Whether there is anything to ask for at all: a row the page already carries a
  // target for has nothing to read, and the resolver exists for the calls a trim
  // left identity-only.
  const needsTarget = missingCalls(item).length > 0;

  useEffect(() => {
    if (!enabled || !needsTarget || !resolveTarget || !toolCallId || !callRunId)
      return;
    let cancelled = false;
    void resolveTarget({ runId: callRunId, toolCallId })
      .catch(() => null)
      .then((value) => {
        if (!cancelled && value)
          setResolved(value);
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, needsTarget, resolveTarget, callRunId, toolCallId]);

  return shownTarget(item, resolved);
}

function AgentActivitySingleLine({ item, resolveTarget, workspacePath, runId }: AgentActivityLineProps) {
  const label = labelForActivity(item);
  const failed = item.status === "failed";
  const running = item.status === "running";
  // Headers stay in the reading column; long commands and paths get their
  // own wrapping line when expanded.
  const [open, setOpen] = useState(false);
  const target = useActivityTarget({ enabled: open, item, resolveTarget, runId });
  const displayTarget = target ? relativizeTarget(item.kind, target, workspacePath) : undefined;
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
    <div className="flex min-w-0 flex-col gap-1 text-13 leading-6 text-ink-muted">
      <div className="flex max-w-full items-center gap-2 self-start">
        {runId || displayTarget || canBeRevealed(item, resolveTarget)
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
              title={item.detail ?? target ?? undefined}
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
function AgentActivityGroupLine({ item, resolveTarget, workspacePath, runId }: AgentActivityLineProps) {
  const label = labelForActivity(item);
  const children = item.children ?? [];
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronUp : ChevronDown;

  return (
    <div
      className="flex min-w-0 flex-col gap-1 text-13 leading-6 text-ink-muted"
    >
      <button
        type="button"
        className="flex max-w-full cursor-pointer items-center gap-2 self-start text-left hover:text-ink"
        aria-expanded={open}
        onClick={() => setOpen(v => !v)}
      >
        {renderActivityIcon(item.kind, false)}
        <span>{label}</span>
        <Chevron className="size-3 shrink-0" />
      </button>
      {open
        ? (
            <AgentActivityChildren
              items={children}
              resolveTarget={resolveTarget}
              runId={runId}
              workspacePath={workspacePath}
            />
          )
        : null}
    </div>
  );
}

/**
 * An expanded burst's children, each with its own target.
 *
 * The burst is one gesture, so its commands are read as one: every child the
 * group draws is already on screen the moment it opens, and asking for them one
 * at a time would mean the reader watches each row fill in separately. The calls
 * it asks for are the ones this page did not carry — a page that carried them
 * (a local run, a host that does not trim) asks for nothing at all.
 */
function AgentActivityChildren({
  items,
  resolveTarget,
  runId,
  workspacePath,
}: {
  items: AgentActivityItem[];
  resolveTarget?: ActivityTargetResolver;
  runId?: string | null;
  workspacePath?: string | null;
}) {
  const [found, setFound] = useState<ReadonlyMap<string, string>>(() => new Map());
  // The calls this page did not carry, as `run::call` keys: the effect runs when
  // the *set* of them changes rather than once per message re-projection, and the
  // keys are what it splits back into the requests it makes.
  const keys = useMemo(
    () => items.flatMap(item => missingCalls(item)
      .map(call => `${call.runId ?? runId ?? ""}::${call.toolCallId}`)
      .filter(key => !key.startsWith("::"))),
    [items, runId],
  );
  const key = keys.join(",");

  useEffect(() => {
    if (!resolveTarget || key === "")
      return;
    let cancelled = false;
    const asked = key.split(",").map((entry) => {
      const separator = entry.indexOf("::");
      return { runId: entry.slice(0, separator), toolCallId: entry.slice(separator + 2) };
    });
    void Promise.all(asked.map(async (call): Promise<readonly [string, string] | null> => {
      const target = await resolveTarget(call).catch(() => null);
      return target ? [call.toolCallId, target] as const : null;
    })).then((answers) => {
      if (cancelled)
        return;
      const landed = answers.filter((answer): answer is readonly [string, string] => answer !== null);
      if (landed.length > 0)
        setFound(new Map(landed));
    });
    return () => {
      cancelled = true;
    };
  }, [key, resolveTarget]);

  return (
    <div className="flex flex-col gap-1 pl-6">
      {items.map((child) => {
        const target = shownTarget(child, child.toolCallId ? found.get(child.toolCallId) ?? null : null);
        const shown = target ? relativizeTarget(child.kind, target, workspacePath) : "";
        return (
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
                    {shown}
                  </button>
                )
              : (
                  <span
                    className="min-w-0 select-text whitespace-pre-wrap wrap-anywhere text-left font-mono"
                    title={child.detail ?? target ?? undefined}
                  >
                    {shown}
                  </span>
                )}
          </div>
        );
      })}
    </div>
  );
}

/** Whether opening this row could reveal anything, given a way to ask. */
function canBeRevealed(item: AgentActivityItem, resolveTarget: ActivityTargetResolver | undefined): boolean {
  return Boolean(resolveTarget && missingCalls(item).length > 0);
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
    case "fusion":
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
    case "fusion":
      return i18n.t(running ? "agent:fusion.discussing" : "agent:fusion.discussion");
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
    case "fusion":
      return i18n.t("agent:fusion.states.failed");
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
