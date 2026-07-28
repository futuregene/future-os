/**
 * Per-run high-watermark cursor for gap detection.
 *
 * The mobile client receives streaming events over core NATS (at-most-once).
 * Under jitter, events may arrive out of order or be silently dropped.
 * This module tracks the highest idx applied per run so the context layer
 * can detect gaps and trigger incremental backfill before applying.
 *
 * Pure functions — no side effects, fully unit-testable.
 */

export type CursorEvent =
  | { kind: "apply"; idx: number }
  | { kind: "dup" }
  | { kind: "gap"; fromIdx: number }
  | { kind: "untracked" };

/** Map of runId → highest idx applied in order. Capped to MAX_RUNS entries. */
export type RunCursor = Map<string, number>;

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
 * - idx = high-water + 1, or first event for this run → "apply" (advance cursor).
 * - idx > high-water + 1 → "gap" (fromIdx = current high-water).
 */
export function nextEvent(
  cursor: RunCursor,
  runId: string | undefined | null,
  idx: number | undefined | null,
): CursorEvent {
  if (!runId || idx == null) return { kind: "untracked" };

  const high = cursor.get(runId);
  if (high === undefined) {
    // First event for this run — accept and start tracking.
    advanceCursor(cursor, runId, idx);
    return { kind: "apply", idx };
  }
  if (idx <= high) return { kind: "dup" };
  if (idx === high + 1) {
    advanceCursor(cursor, runId, idx);
    return { kind: "apply", idx };
  }
  // idx > high + 1 → gap
  return { kind: "gap", fromIdx: high };
}

/**
 * Advance the cursor after successfully applying an event (or a batch).
 * Call this after applying events out-of-band (e.g. backfill results).
 */
export function advanceCursor(cursor: RunCursor, runId: string, idx: number): void {
  const current = cursor.get(runId);
  if (current === undefined || idx > current) {
    cursor.set(runId, idx);
    // Evict oldest entries when over capacity (Map preserves insertion order).
    while (cursor.size > MAX_RUNS) {
      const oldest = cursor.keys().next().value;
      if (oldest !== undefined) cursor.delete(oldest);
    }
  }
}

/**
 * Rebuild cursor state from a set of already-applied events (e.g. after
 * a full resync or backfill). Pass the run's events; the cursor is set to
 * the max idx seen.
 */
export function rebuildCursorFromEvents(
  cursor: RunCursor,
  events: Array<{ runId?: string | null; idx?: number | null }>,
): void {
  for (const event of events) {
    if (event.runId && event.idx != null) {
      advanceCursor(cursor, event.runId, event.idx);
    }
  }
}
