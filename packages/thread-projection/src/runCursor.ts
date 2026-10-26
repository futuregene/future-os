import { contentRangeStart } from "./contentRange";

/**
 * Per-run integrity cursor for gap detection and prefix completeness.
 *
 * A remote client receives a run's events over core NATS (at-most-once). Under
 * jitter, events may arrive out of order or be silently dropped — and a client
 * that joins a run mid-flight sees its tail only, with no guarantee the prefix
 * will arrive before it is needed.
 *
 * Each cursor entry tracks two facts the timeline cache needs:
 *   highWater       — highest idx applied in order for this run
 *   prefixComplete  — whether the cache provably contains [0, highWater]
 *
 * prefixComplete is the anti-H3/H5 invariant: a run whose first seen event is
 * idx 0, or that was established by a full replay (`get_events_since` from
 * -1), is prefix-complete; a run whose first seen event has idx > 0 is not, and
 * the client must reconcile it from -1 before it can be trusted.
 *
 * Both remote clients keep this cursor — the phone
 * (`mobile/src/remote/syncEngine.ts`) and the desktop's remote client
 * (`desktop/src/features/remote-peer`). It lives here because "how far a run's
 * log has been applied, and whether its beginning is provably present" is a fact
 * about the host's wire contract, not about either client's rendering.
 *
 * Pure functions — no side effects, fully unit-testable.
 */

export type CursorEvent =
  | { kind: "apply"; idx: number }
  | { kind: "dup" }
  | { kind: "overlap"; fromIdx: number }
  | { kind: "gap"; fromIdx: number }
  | { kind: "untracked" };

/** Map of runId → per-run cursor. Capped to MAX_RUNS entries. */
export type RunCursor = Map<string, RunCursorEntry>;

export interface RunCursorEntry {
  highWater: number;
  prefixComplete: boolean;
}

const MAX_RUNS = 8;

export function newCursor(): RunCursor {
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
 * - idx > high-water + 1 with the covered range starting at high-water + 1 →
 *   "apply" (a coalesced range that continues the cursor).
 * - a coalesced range starting *below* the high-water → "overlap": part of its
 *   content is already applied and the merge hides where that head ends.
 * - idx > high-water + 1 otherwise → "gap" (fromIdx = current high-water).
 *
 * `coalescedCount` marks an event a host merged from several source events
 * (`event_coalescing_v1`): its `idx` is the END of that range and its content covers all of it,
 * so the jump is declared rather than lost and is applied without a gap.
 */
export function nextEvent(
  cursor: RunCursor,
  runId: string | undefined | null,
  idx: number | undefined | null,
  coalescedCount?: number,
  options?: { omitIndices?: boolean; data?: string },
): CursorEvent {
  if (!runId || idx == null) return { kind: "untracked" };

  const canonicalStart = contentRangeStart(options?.data);
  const covered = canonicalStart !== undefined && canonicalStart <= idx ? { start: canonicalStart } : coveredFrom(idx, coalescedCount);
  const entry = cursor.get(runId);
  if (entry === undefined) {
    if (canonicalStart !== undefined && canonicalStart > 0 && !options?.omitIndices) return { kind: "gap", fromIdx: -1 };
    // First event for this run — accept and start tracking. A run that begins
    // above idx 0 has an unknown prefix (H3): the caller reconciles from -1.
    advanceCursor(cursor, runId, idx, covered.start === 0);
    return { kind: "apply", idx };
  }
  if (idx <= entry.highWater) return { kind: "dup" };
  // A merged range is only applicable when it continues exactly where the
  // cursor sits. Its text is one opaque concatenation, so a range that starts
  // below the high-water covers text the cursor already applied and cannot be
  // trimmed — that head was merged with content we still need. Applying it
  // would append the overlap a second time, which reads as a duplicated
  // fragment and a fence reopened mid-message. The journal still holds every
  // source event, so the caller recovers the range from replay instead.
  if (covered.start === entry.highWater + 1 || (canonicalStart !== undefined && covered.start <= entry.highWater + 1)) {
    cursor.set(runId, { ...entry, highWater: idx });
    return { kind: "apply", idx };
  }
  if (covered.start <= entry.highWater) return { kind: "overlap", fromIdx: entry.highWater };
  // A feed that omits indices on purpose — the client declared a trimmed lane
  // (`lean_events_v1`) — advances over the hole instead of treating it as loss.
  // Reconciling cannot recover slices the peer is not sending, so a gap verdict
  // here would retry the same unrecoverable range forever (the phone shows a
  // sync notice it can never clear).
  if (options?.omitIndices) {
    cursor.set(runId, { ...entry, highWater: idx });
    return { kind: "apply", idx };
  }
  // idx > high-water + 1 with nothing covering the hole → gap
  return { kind: "gap", fromIdx: entry.highWater };
}

/** The source-index range an event covers. A plain event covers exactly `idx`. */
function coveredFrom(idx: number, coalescedCount?: number): { start: number } {
  const count = Number.isSafeInteger(coalescedCount) && (coalescedCount as number) > 0
    ? Math.min(coalescedCount as number, idx + 1)
    : 1;
  return { start: idx - count + 1 };
}

/**
 * Advance the cursor after successfully applying events out-of-band (e.g.
 * backfill/reconcile results). `completePrefix` marks the run's prefix
 * [0, highWater] as fully present — true for a full replay, false for a
 * tail-only top-up (which never establishes prefix completeness).
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
  } else if (idx > entry.highWater) {
    cursor.set(runId, {
      ...entry,
      highWater: idx,
      prefixComplete: entry.prefixComplete || completePrefix,
    });
  } else if (completePrefix && !entry.prefixComplete) {
    // Same high-water, but the run's prefix is now provably complete (a
    // whole-run replay landed exactly where the cursor sat).
    cursor.set(runId, { ...entry, prefixComplete: true });
  }
  // Evict oldest entries when over capacity (Map preserves insertion order).
  while (cursor.size > MAX_RUNS) {
    const oldest = cursor.keys().next().value;
    if (oldest !== undefined) cursor.delete(oldest);
  }
}

export function cursorHighWater(cursor: RunCursor, runId: string | undefined | null): number {
  if (!runId) return -1;
  return cursor.get(runId)?.highWater ?? -1;
}

export function isPrefixComplete(cursor: RunCursor, runId: string | undefined | null): boolean {
  if (!runId) return true;
  return cursor.get(runId)?.prefixComplete ?? false;
}
