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
): Promise<EventsData> {
  const events: ReplayEventWire[] = [];
  let projection: EventsData["projection"] = null;
  let truncated = false;
  let offset = 0;
  for (;;) {
    const page = (
      await client.requestRetry<EventsPage>(
        { type: "get_events_since", sessionId, runId, sinceIdx, offset },
        sessionId,
      )
    ).data;
    events.push(...(page.events ?? []));
    if (page.projection?.events?.length) projection = page.projection;
    if (page.truncated) truncated = true;
    if (!page.hasMore) break;
    const next = page.nextOffset;
    if (typeof next !== "number" || next <= offset) break;
    offset = next;
  }
  const merged: EventsData = { events };
  if (projection) merged.projection = projection;
  if (truncated) merged.truncated = true;
  return merged;
}
