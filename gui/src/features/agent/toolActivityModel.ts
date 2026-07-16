import type { AgentActivityKind } from "./agentThreadTypes";
import { isRecord } from "../../lib/objects";

// Shared tool-activity model: the constants and folding rules that both the live
// streaming projection (agentActivity.ts) and the reload-from-JSONL projection
// (entryProjection.ts) build on. Keeping these in one place is what stops the two
// paths from drifting — a collapse or target-extraction tweak lands in both.

/** The four tool kinds the activity UI models (every other tool maps to "shell"). */
export type ToolKind = Exclude<AgentActivityKind, "thinking">;

const TOOL_KINDS = new Set<ToolKind>(["read", "shell", "edit", "write"]);

/** Whether `name` is one of the modeled tool kinds. */
export function isToolKind(name: string): name is ToolKind {
  return TOOL_KINDS.has(name as ToolKind);
}

/** Map a tool name to its kind, defaulting unknown tools to "shell". */
export function asToolKind(name: string): ToolKind {
  return isToolKind(name) ? name : "shell";
}

// shell/edit/write/read collapse into a single summary row when they run in an
// uninterrupted, same-kind, all-completed burst of more than one.
export const COLLAPSIBLE_KINDS = new Set<ToolKind>(["shell", "edit", "write", "read"]);

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

/** Coerce a tool's `arguments` (object, or a JSON string) to a plain record. */
export function normalizeArgs(value: unknown): Record<string, unknown> | null {
  if (isRecord(value))
    return value;
  if (typeof value !== "string")
    return null;
  try {
    const parsed = JSON.parse(value) as unknown;
    return isRecord(parsed) ? parsed : null;
  }
  catch {
    return null;
  }
}

/**
 * The activity's display target from a tool call's (complete) arguments: the
 * command for shell, else the file path. `args` is already normalized to a
 * record (or null) — see {@link normalizeArgs}.
 */
export function targetFromArgs(kind: ToolKind, args: Record<string, unknown> | null): string | undefined {
  if (kind === "shell")
    return stringValue(args?.command);
  return stringValue(args?.path) ?? stringValue(args?.file_path) ?? stringValue(args?.filePath);
}

/** Keep the first item per target (falling back to id), preserving order. */
export function dedupeByTarget<T extends { id: string; target?: string }>(items: T[]): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const item of items) {
    const key = item.target ?? item.id;
    if (seen.has(key))
      continue;
    seen.add(key);
    out.push(item);
  }
  return out;
}

/**
 * One slice of a collapse pass: either a single pass-through `item`, or a
 * collapsed `group` of >1 consecutive same-`kind` items.
 */
export type CollapseRun<T>
  = | { collapsed: false; item: T }
    | { collapsed: true; kind: ToolKind; group: T[] };

/**
 * Fold consecutive collapsible items into runs. `collapseKindOf` returns an
 * item's kind when it is an eligible (completed, collapsible-kind) unit, or
 * `null` when it must break the run and pass through untouched. A run of more
 * than one eligible same-kind item becomes a collapsed group; everything else
 * passes through individually. This is the single grouping rule shared by the
 * live and reload projections — each caller builds its own output from the runs.
 */
export function foldCollapsibleRuns<T>(
  items: T[],
  collapseKindOf: (item: T) => ToolKind | null,
): CollapseRun<T>[] {
  const runs: CollapseRun<T>[] = [];
  let i = 0;
  while (i < items.length) {
    const item = items[i]!;
    const kind = collapseKindOf(item);
    if (kind !== null) {
      const group = [item];
      let j = i + 1;
      while (j < items.length && collapseKindOf(items[j]!) === kind) {
        group.push(items[j]!);
        j += 1;
      }
      if (group.length > 1) {
        runs.push({ collapsed: true, kind, group });
        i = j;
        continue;
      }
    }
    runs.push({ collapsed: false, item });
    i += 1;
  }
  return runs;
}
