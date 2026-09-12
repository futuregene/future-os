import type { RemoteClient } from "./client";
import type { ReplayEventWire } from "./timeline";

export interface EventsData {
  /** Raw replay events — the RPC serializes them with snake_case `run_id`. */
  events?: ReplayEventWire[];
  truncated?: boolean;
  /** Coalesced replica of a run whose event ring overflowed — replaces the
   *  session's timeline wholesale (see `timelineFromProjection`). */
  projection?: { run_id?: string; cursor?: number; events?: ReplayEventWire[] } | null;
}

export interface EventsPage extends EventsData {
  watermark?: number;
  nextSinceIdx?: number;
  hasMore?: boolean;
  nextOffset?: number;
}

/**
 * Fetch a run's replay tail via `get_events_since`, looping paginated replies
 * until `hasMore=false`. The desktop pages replay events under the NATS payload
 * cap (a multi-MB journal tail must never ship as one oversized reply), so a
 * single-shot request would silently truncate. The merged envelope carries the
 * first page's `projection` (the whole-run replacement) through unchanged.
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
  let truncated = false;
  let offset = 0;
  let cursor = sinceIdx;
  let watermark: number | undefined;
  for (;;) {
    // An in-flight request may finish, but a hidden/replaced lane must not
    // keep issuing pages or accumulating a replay nobody is displaying.
    if (!isCurrent()) throw new Error("stale_sync_lane");
    const page = (
      await client.requestRetry<EventsPage>(
        {
          type: "get_events_since",
          sessionId,
          runId,
          sinceIdx: cursor,
          offset,
          ...(watermark === undefined ? {} : { replayUntilIdx: watermark }),
        },
        sessionId,
      )
    ).data;
    if (!isCurrent()) throw new Error("stale_sync_lane");
    if (watermark !== undefined && page.watermark !== watermark)
      throw new Error("replay_window_changed");
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
  const merged: EventsData = { events };
  if (projection) merged.projection = projection;
  if (truncated) merged.truncated = true;
  return merged;
}
