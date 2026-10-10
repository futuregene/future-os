import type { RemoteClient } from "./client";
import { requestReadPage } from "./readPages";
import type { ReplayEventWire } from "./timeline";

// Token events are small: the desktop's generic 100-item default wastes a
// network round trip per ~100 tokens. Keep a bounded event count while relying
// on its independent 512-KiB wire budget (and chunked reads) for large tools.
const REPLAY_PAGE_EVENTS = 1000;

export interface EventsData {
  /** Raw replay events — the RPC serializes them with snake_case `run_id`. */
  events?: ReplayEventWire[];
  truncated?: boolean;
  /** Fixed replay boundary, also returned for a single/empty page. */
  watermark?: number;
  /**
   * How many source events the pages covered *before* the lean trim removed
   * any. A feed that omits indices (the lease client declared
   * `lean_events_v1`) cannot be checked by counting what arrived, so the peer
   * states what it covered and the check stays exact rather than relaxed.
   * Absent when the peer does not trim.
   */
  rawEvents?: number;
  /** Resumable semantic result for one run, from snapshot bootstrap or ring
   * overflow. Replaces that run's projection, not unrelated session history. */
  projection?: { run_id?: string; runId?: string; cursor?: number; events?: ReplayEventWire[] } | null;
}

/**
 * Whether a fetched tail covers the range `since+1 .. watermark`.
 *
 * `rawEvents` is the peer's statement of how many source events that range held
 * *before* its lean trim removed any. A peer that does not trim produces exactly
 * one event per index, so its events must match index-for-index. A peer that
 * does trim (the client declared `lean_events_v1`) sends holes by design — the
 * client asked for a feed it does not need the omitted slices of — so counting
 * what arrived would fail every time. There the raw count is what still proves
 * the tail reached the watermark, and strict ordering within the range is what
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
  if (watermark !== undefined && watermark !== since + raw) return false;
  if (!trimmed) return !events.some((event, index) => event.idx !== since + 1 + index);
  let previous = since;
  return events.every(event => {
    const idx = event.idx;
    if (!Number.isSafeInteger(idx) || (idx as number) <= previous) return false;
    previous = idx as number;
    return watermark === undefined || (idx as number) <= watermark;
  });
}

export interface EventsPage extends EventsData {
  /** New Desktop/Agent opt-in bootstrap; followed by one fixed-window tail. */
  runSnapshot?: boolean;
  nextSinceIdx?: number;
  hasMore?: boolean;
  nextOffset?: number;
}

/**
 * Prefer semantic bootstrap for a cold run, then fetch its new fixed-window
 * tail. Older peers fall back to raw `get_events_since`, looping paginated replies
 * until `hasMore=false`. The desktop pages replay events under the NATS payload
 * cap (a multi-MB journal tail must never ship as one oversized reply), so a
 * single-shot request would silently truncate. Legacy projection replies pass
 * through unchanged; an opt-in snapshot is extended through its verified tail
 * watermark and restored as one coherent whole-run replacement.
 */
export async function fetchEventsSince(
  client: RemoteClient,
  sessionId: string,
  runId: string,
  sinceIdx: number,
  isCurrent: () => boolean = () => true,
): Promise<EventsData> {
  const events: ReplayEventWire[] = [];
  let projection: EventsData["projection"] = null;
  let bootstrap: NonNullable<EventsData["projection"]> | undefined;
  let truncated = false;
  let offset = 0;
  let cursor = sinceIdx;
  let watermark: number | undefined;
  // Totals across the pages, so the completeness check at the end speaks for
  // the whole tail rather than one page. `rawEvents` only counts when every
  // page states it: a partial total would be a wrong total.
  let rawEvents = 0;
  let rawEventsComplete = true;
  for (;;) {
    // An in-flight request may finish, but a hidden/replaced lane must not
    // keep issuing pages or accumulating a replay nobody is displaying.
    if (!isCurrent()) throw new Error("stale_sync_lane");
    const page = (
      await requestReadPage<EventsPage>(client,
        {
          type: "get_events_since",
          sessionId,
          runId,
          sinceIdx: cursor,
          limit: REPLAY_PAGE_EVENTS,
          offset,
          ...(sinceIdx === -1 && cursor === -1 && offset === 0 && watermark === undefined && !bootstrap
            ? { preferSnapshot: true } : {}),
          ...(watermark === undefined ? {} : { replayUntilIdx: watermark }),
        },
        sessionId,
        isCurrent,
      )
    ).data;
    if (!isCurrent()) throw new Error("stale_sync_lane");
    if (page.runSnapshot) {
      const snapshot = page.projection;
      const boundary = snapshot?.cursor;
      let previous = -1;
      if (bootstrap || cursor !== -1 || offset !== 0 || watermark !== undefined
        || !snapshot || (snapshot.runId ?? snapshot.run_id) !== runId
        || !Number.isSafeInteger(boundary) || boundary! < 0 || boundary !== page.watermark
        || !snapshot.events?.length || page.events?.length || page.hasMore
        || snapshot.events.some(event => {
          const idx = event.idx;
          const valid = Number.isSafeInteger(idx) && idx! > previous && idx! <= boundary!
            && (event.runId ?? event.run_id ?? runId) === runId;
          previous = idx ?? -1;
          return !valid;
        })) throw new Error("replay_projection_invalid");
      bootstrap = snapshot;
      cursor = boundary!;
      // Pin a NEW watermark for the tail produced while the immutable snapshot
      // was being read. Never append folded text directly to cached text.
      continue;
    }
    if (bootstrap && page.projection) throw new Error("replay_snapshot_tail_replaced");
    if (watermark !== undefined && page.watermark !== watermark)
      throw new Error("replay_window_changed");
    if (Number.isSafeInteger(page.watermark)) watermark = page.watermark;
    events.push(...(page.events ?? []));
    if (Number.isSafeInteger(page.rawEvents) && (page.rawEvents as number) >= 0) {
      rawEvents += page.rawEvents as number;
    } else {
      rawEventsComplete = false;
    }
    if (page.projection?.events?.length) projection = page.projection;
    if (page.truncated) truncated = true;
    if (!page.hasMore) break;
    if (Number.isSafeInteger(page.watermark)) {
      if (watermark !== undefined && page.watermark !== watermark)
        throw new Error("replay_window_changed");
      watermark = page.watermark;
      const nextCursor = page.nextSinceIdx;
      if (!Number.isSafeInteger(nextCursor) || nextCursor! <= cursor)
        throw new Error("replay_cursor_stalled");
      cursor = nextCursor!;
      offset = 0;
      continue;
    }
    const next = page.nextOffset;
    if (typeof next !== "number" || next <= offset) break;
    offset = next;
  }
  if (bootstrap) {
    const boundary = bootstrap.cursor!;
    if (truncated || !Number.isSafeInteger(watermark)
      || !tailCoversRange(events, boundary, watermark, rawEventsComplete ? rawEvents : undefined)
      || events.some(event => (event.runId ?? event.run_id ?? runId) !== runId)) {
      throw new Error("replay_prefix_invalid");
    }
    // Restore one coherent projection through the tail watermark. SyncEngine
    // commits its projector + cursor together, then deduplicates queued live
    // events. An empty tail still preserves the snapshot's exact boundary.
    projection = { ...bootstrap, cursor: watermark, events: [...bootstrap.events!, ...events] };
  }
  const merged: EventsData = { events: bootstrap ? [] : events };
  if (watermark !== undefined) merged.watermark = watermark;
  if (rawEventsComplete && rawEvents > 0) merged.rawEvents = rawEvents;
  if (projection) merged.projection = projection;
  if (truncated) merged.truncated = true;
  return merged;
}
