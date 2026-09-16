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
  /** Resumable semantic result for one run, from snapshot bootstrap or ring
   * overflow. Replaces that run's projection, not unrelated session history. */
  projection?: { run_id?: string; runId?: string; cursor?: number; events?: ReplayEventWire[] } | null;
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
    if (truncated || !Number.isSafeInteger(watermark) || watermark !== boundary + events.length
      || events.some((event, index) => event.idx !== boundary + index + 1
        || (event.runId ?? event.run_id ?? runId) !== runId)) {
      throw new Error("replay_prefix_invalid");
    }
    // Restore one coherent projection through the tail watermark. SyncEngine
    // commits its projector + cursor together, then deduplicates queued live
    // events. An empty tail still preserves the snapshot's exact boundary.
    projection = { ...bootstrap, cursor: watermark, events: [...bootstrap.events!, ...events] };
  }
  const merged: EventsData = { events: bootstrap ? [] : events };
  if (watermark !== undefined) merged.watermark = watermark;
  if (projection) merged.projection = projection;
  if (truncated) merged.truncated = true;
  return merged;
}
