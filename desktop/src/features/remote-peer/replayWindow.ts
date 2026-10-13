/**
 * Per-run integrity cursor + replay window check, ported from the mobile
 * lane.  Pure logic, no React/Tauri side effects.
 *
 * The desktop remote timeline keeps a set of run cursors in memory so that,
 * after a link drop, it can ask the host for only the events it missed via
 * `get_events_since` instead of replaying the whole newest page.  NATS Core
 * is at-most-once: events may be lost, reordered or duplicated, so the replay
 * window must be validated before it is applied.  When validation fails the
 * client falls back to the existing whole-page history read.
 */

export type CursorEvent
  = | { kind: "apply"; idx: number }
    | { kind: "dup" }
    | { kind: "overlap"; fromIdx: number }
    | { kind: "gap"; fromIdx: number }
    | { kind: "untracked" };

/** Map of runId → per-run cursor.  Capped to MAX_RUNS entries. */
export type RunCursor = Map<string, RunCursorEntry>;

export interface RunCursorEntry {
  highWater: number;
  prefixComplete: boolean;
}

/** How many runs' cursors are kept. */
export const MAX_RUNS = 8;

/** Page size for an incremental replay request. */
export const REPLAY_PAGE_EVENTS = 1000;

export function newRunCursor(): RunCursor {
  return new Map();
}

/**
 * Classify an incoming event against the cursor.
 *
 * Rules:
 * - No runId or idx → "untracked" (apply as-is, no tracking).
 * - idx ≤ high-water → "dup" (already applied).
 * - idx = high-water + 1, or first event for this run → "apply".
 *   A first event with idx > 0 is accepted for live rendering but recorded
 *   prefix-incomplete; the caller must reconcile the prefix.
 * - A coalesced range that continues exactly from high-water + 1 → "apply".
 * - A coalesced range that starts below high-water → "overlap".
 * - idx > high-water + 1 otherwise → "gap" (fromIdx = current high-water).
 */
export function nextEvent(
  cursor: RunCursor,
  runId: string | undefined | null,
  idx: number | undefined | null,
  coalescedCount?: number,
  options?: { omitIndices?: boolean },
): CursorEvent {
  if (!runId || idx == null)
    return { kind: "untracked" };

  const covered = coveredFrom(idx, coalescedCount);
  const entry = cursor.get(runId);
  if (entry === undefined) {
    advanceCursor(cursor, runId, idx, covered.start === 0);
    return { kind: "apply", idx };
  }
  if (idx <= entry.highWater)
    return { kind: "dup" };
  if (covered.start === entry.highWater + 1) {
    cursor.set(runId, { ...entry, highWater: idx });
    return { kind: "apply", idx };
  }
  if (covered.start <= entry.highWater)
    return { kind: "overlap", fromIdx: entry.highWater };
  if (options?.omitIndices) {
    cursor.set(runId, { ...entry, highWater: idx });
    return { kind: "apply", idx };
  }
  return { kind: "gap", fromIdx: entry.highWater };
}

function coveredFrom(idx: number, coalescedCount?: number): { start: number } {
  const count = Number.isSafeInteger(coalescedCount) && (coalescedCount as number) > 0
    ? Math.min(coalescedCount as number, idx + 1)
    : 1;
  return { start: idx - count + 1 };
}

/**
 * Advance the cursor after applying events out-of-band (reconcile/backfill).
 * `completePrefix` marks [0, highWater] as fully present.
 */
export function advanceCursor(
  cursor: RunCursor,
  runId: string,
  idx: number,
  completePrefix = false,
): void {
  const entry = cursor.get(runId);
  if (entry === undefined) {
    cursor.set(runId, { highWater: idx, prefixComplete: completePrefix });
  }
  else if (idx > entry.highWater) {
    cursor.set(runId, {
      ...entry,
      highWater: idx,
      prefixComplete: entry.prefixComplete || completePrefix,
    });
  }
  else if (completePrefix && !entry.prefixComplete) {
    cursor.set(runId, { ...entry, prefixComplete: true });
  }
  while (cursor.size > MAX_RUNS) {
    const oldest = cursor.keys().next().value;
    if (oldest !== undefined)
      cursor.delete(oldest);
  }
}

export function cursorHighWater(cursor: RunCursor, runId: string | undefined | null): number {
  if (!runId)
    return -1;
  return cursor.get(runId)?.highWater ?? -1;
}

export function isPrefixComplete(cursor: RunCursor, runId: string | undefined | null): boolean {
  if (!runId)
    return true;
  return cursor.get(runId)?.prefixComplete ?? false;
}

/**
 * Whether a fetched tail covers the range `since+1 .. watermark`.
 *
 * `rawEvents` is the peer's statement of how many source events that range
 * held *before* lean trim removed any.  An untrimmed peer produces exactly one
 * event per index, so its events must match index-for-index.  A trimmed peer
 * sends holes by design — the client asked for a feed it does not need those
 * slices of — so counting what arrived would fail every time.  The raw count
 * proves the tail reached the watermark, while strict ordering within the range
 * still catches a garbled or reordered reply.
 */
export function tailCoversRange(
  events: readonly { idx?: number }[],
  since: number,
  watermark: number | undefined,
  rawEvents: number | undefined,
): boolean {
  const trimmed = Number.isSafeInteger(rawEvents) && (rawEvents as number) >= 0;
  const raw = trimmed ? (rawEvents as number) : events.length;
  if (watermark !== undefined && watermark !== since + raw)
    return false;
  if (!trimmed)
    return !events.some((event, index) => event.idx !== since + 1 + index);
  let previous = since;
  return events.every((event) => {
    const idx = event.idx;
    if (!Number.isSafeInteger(idx) || (idx as number) <= previous)
      return false;
    previous = idx as number;
    return watermark === undefined || (idx as number) <= watermark;
  });
}

/** Result of validating and absorbing a replay page. */
export interface ReplayApplyResult {
  /** Events that should be appended, or `null` if the page cannot be trusted. */
  apply: ReplayEvent[] | null;
  /** Highest event index now known for each affected run. */
  highWaters: Record<string, number>;
  /** Whether every run covered by the page is prefix-complete afterwards. */
  prefixComplete: boolean;
}

/** Minimal replay event shape used by the validator. */
export interface ReplayEvent {
  id: string;
  runId?: string | null;
  idx?: number;
}

/**
 * Validate a replay page against the current cursor.
 *
 * - Rejects (`apply: null`) when the page is not contiguous: either the host
 *   watermark does not match the events/rawEvents, or an event would introduce
 *   a gap in any run the cursor already knows about.
 * - Updates the cursors for runs that pass validation.
 * - Treats an `untracked` event (no runId/idx) as always applicable.
 */
export function applyReplayPage(
  cursor: RunCursor,
  events: readonly ReplayEvent[],
  watermark: number | undefined,
  rawEvents: number | undefined,
  since: number,
): ReplayApplyResult {
  if (!tailCoversRange(events, since, watermark, rawEvents)) {
    return { apply: null, highWaters: {}, prefixComplete: false };
  }

  const sorted = [...events].sort((a, b) => {
    if (a.runId && b.runId && a.runId !== b.runId)
      return a.runId.localeCompare(b.runId);
    if ((a.idx ?? -1) !== (b.idx ?? -1))
      return (a.idx ?? -1) - (b.idx ?? -1);
    return a.id.localeCompare(b.id);
  });

  const apply: ReplayEvent[] = [];
  let prefixComplete = true;
  const omitIndices = Number.isSafeInteger(rawEvents) && (rawEvents as number) >= 0;

  for (const event of sorted) {
    const runId = event.runId ?? null;
    const idx = event.idx ?? null;
    if (!runId || idx === null) {
      apply.push(event);
      continue;
    }
    const verdict = nextEvent(cursor, runId, idx, undefined, { omitIndices });
    switch (verdict.kind) {
      case "apply":
      case "untracked":
        apply.push(event);
        break;
      case "dup":
        // Already rendered; skip without treating it as a failure.
        break;
      case "overlap":
      case "gap":
        return { apply: null, highWaters: {}, prefixComplete: false };
      default:
        break;
    }
  }

  for (const entry of cursor.values()) {
    if (entry.prefixComplete === false)
      prefixComplete = false;
  }

  return { apply, highWaters: collectHighWaters(cursor), prefixComplete };
}

/**
 * The highest event index across all runs in the cursor.
 * Returns -1 when no run is tracked.
 */
export function highestWatermark(cursor: RunCursor): number {
  let max = -1;
  for (const entry of cursor.values()) {
    if (entry.highWater > max)
      max = entry.highWater;
  }
  return max;
}

function collectHighWaters(cursor: RunCursor): Record<string, number> {
  const highWaters: Record<string, number> = {};
  for (const [runId, entry] of cursor) {
    highWaters[runId] = entry.highWater;
  }
  return highWaters;
}
