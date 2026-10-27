import { contentRangeStart } from "./contentRange";
import { copyEventPayload } from "./eventPayload";

/**
 * Reading a run's event log back from a host — the wire contract, not a client.
 *
 * A remote run arrives over two lanes that a host serves differently: a *push*
 * lane (core NATS, at-most-once — any suffix can be lost, including the terminal
 * frame) and a *read* lane (`get_events_since`: durable, ordered, pageable). The
 * read is the truth; the pushes are what makes the reply arrive as it is written.
 * This module is the read lane: how a client walks a run's pages, what it may
 * conclude from them, and when a page must be refused rather than applied.
 *
 * Both remote clients read through it — the phone (`mobile/src/remote/replay.ts`
 * wraps it in its request layer) and the desktop's remote client
 * (`desktop/src/features/remote-peer/remoteRunReplay.ts`). It lives in this
 * package because every rule below is a fact about the *host's* replies: a copy
 * per client would drift from the host's contract the moment one side was fixed.
 *
 * The page loop has three jobs:
 *
 * - **Bootstrap a cold run.** A run that has been going for a while has a long
 *   log; asking for it from index 0 can be tens of thousands of events. The host
 *   folds it into one immutable snapshot (`preferSnapshot`) whose *text* is what
 *   a client renders, the client pulls that back in chunks, and then follows it
 *   with a fixed-window tail — so the result is one coherent whole-run
 *   projection rather than "prefix now, rest maybe".
 * - **Pin the window.** The first page names a `watermark`, the newest index that
 *   existed when the read began, and every later page must repeat it: a run that
 *   grew while it was being read would otherwise be assembled from two different
 *   ends, which is how a client loses a slice in the middle or draws one twice.
 * - **Prove the tail.** A client that applies a suffix must be able to say it
 *   holds everything through the watermark — under at-most-once, a missing middle
 *   looks exactly like a short run. `tailCoversRange` is that proof, and a feed
 *   that trims events by design (`lean_events_v1`) states its own pre-trim count
 *   so the proof stays exact instead of being relaxed to "whatever arrived will
 *   do".
 */

/** One run event, normalized: the shape both clients' projectors consume. */
export interface ReplayEvent {
  /** The host's event type (`agent_start`, `text_chunk`, `tool_end`, …). */
  type: string;
  /**
   * The event's payload as a JSON string — what the projectors parse. Empty when
   * the feed carried none, which a projector reads as "nothing to fold".
   */
  data: string;
  /**
   * The run this event belongs to, or `""` when the feed did not name one (a
   * folded projection may carry it only on its envelope). A caller that asked
   * for a specific run stamps that id in, exactly as the phone does.
   */
  runId: string;
  /** Run-scoped index. Absent on a feed that does not number its events. */
  idx?: number;
  /**
   * How many source events this one stands for, when the host merged them
   * (`event_coalescing_v1`). Its `idx` is the end of that range.
   */
  coalescedCount?: number;
  /** The host's own id for the event, when the envelope carried one. */
  eventId?: string;
  /** The host's ISO timestamp, when the envelope carried one. */
  timestamp?: string;
}

/**
 * A replay event as it comes off the wire: snake_case for the fields the agent
 * serializes, camelCase for the ones the host's own publisher writes. Both ends
 * are accepted because both exist — a legacy bridge serializes `run_id`.
 */
export interface ReplayEventWire {
  type?: unknown;
  data?: unknown;
  run_id?: unknown;
  runId?: unknown;
  idx?: unknown;
  event_id?: unknown;
  eventId?: unknown;
  timestamp?: unknown;
  coalescedCount?: unknown;
  coalesced_count?: unknown;
  [key: string]: unknown;
}

/** One wire event in the canonical shape. Non-objects become `null`. */
export function normalizeReplayEvent(raw: unknown): ReplayEvent | null {
  if (typeof raw !== "object" || raw === null)
    return null;
  const record = raw as ReplayEventWire;
  const text = (value: unknown): string => (typeof value === "string" ? value : "");
  const coalescedCount = typeof record.coalescedCount === "number"
    ? record.coalescedCount
    : typeof record.coalesced_count === "number"
      ? record.coalesced_count
      : undefined;
  return copyEventPayload(record, {
    type: text(record.type),
    data: text(record.data),
    runId: text(record.runId) || text(record.run_id),
    ...(typeof record.idx === "number" ? { idx: record.idx } : {}),
    ...(coalescedCount === undefined ? {} : { coalescedCount }),
    ...(text(record.eventId) || text(record.event_id)
      ? { eventId: text(record.eventId) || text(record.event_id) }
      : {}),
    ...(text(record.timestamp) ? { timestamp: text(record.timestamp) } : {}),
  });
}

export function normalizeReplayEvents(raw: unknown): ReplayEvent[] {
  if (!Array.isArray(raw))
    return [];
  return raw.flatMap((item) => {
    const event = normalizeReplayEvent(item);
    return event === null ? [] : [event];
  });
}

/**
 * Whether a fetched tail covers the range `since+1 .. watermark`.
 *
 * `rawEvents` is the peer's statement of how many source events that range held
 * *before* its trim removed any. A peer that does not trim produces exactly one
 * event per index, so its events must match index-for-index. A peer that does
 * trim (the client declared `lean_events_v1`) sends holes by design — the client
 * asked for a feed it does not need the omitted slices of — so counting what
 * arrived would fail every time. There the raw count is what still proves the
 * tail reached the watermark, and strict ordering within the range is what still
 * catches a garbled or reordered reply.
 */
export function tailCoversRange(
  events: readonly { idx?: number; data?: string; coalescedCount?: number }[],
  since: number,
  watermark: number | undefined,
  rawEvents: number | undefined,
): boolean {
  const trimmed = Number.isSafeInteger(rawEvents) && (rawEvents as number) >= 0;
  const raw = trimmed ? (rawEvents as number) : events.length;
  if (trimmed && watermark !== undefined && watermark !== since + raw) return false;
  if (!trimmed) {
    let cursor=since;
    for (const event of events) {
      const idx=event.idx;
      if (idx === undefined || !Number.isSafeInteger(idx) || idx <= cursor) return false;
      const first=contentRangeStart(event.data) ?? idx;
      if (first > cursor + 1 || first < 0 || first > idx) return false;
      // Legacy coalescing has no byte offsets, so it cannot authorize overlap.
      if (first < cursor + 1 && contentRangeStart(event.data) === undefined) return false;
      cursor=idx;
    }
    return watermark === undefined || cursor === watermark;
  }
  let previous = since;
  return events.every((event) => {
    const idx = event.idx;
    if (!Number.isSafeInteger(idx) || (idx as number) <= previous) return false;
    previous = idx as number;
    return watermark === undefined || (idx as number) <= watermark;
  });
}

/**
 * Events per replay page.
 *
 * Token events are small: a host's generic 100-item default wastes a network
 * round trip per ~100 tokens. A bounded event count still relies on the
 * independent wire byte budget (and chunked reads) for large tool payloads.
 */
export const REPLAY_PAGE_EVENTS = 1000;

/** A whole-run projection the host folded for a cold read. */
export interface ReplayProjection {
  /** The run this projection is for; `""` when the host's envelope omitted it. */
  runId: string;
  /** The newest index the projection covers — the boundary its tail continues from. */
  cursor: number;
  /** The folded events, in index order. */
  events: ReplayEvent[];
}

/** One page of a run's log, as it comes off the wire. */
export interface ReplayPage {
  events?: unknown;
  truncated?: boolean;
  /** Fixed replay boundary: the first page names it, every later page repeats it. */
  watermark?: number;
  /** How many source events the pages covered *before* a trim removed any. */
  rawEvents?: number;
  /** A whole-run projection, when the host answered with a snapshot. */
  projection?: { run_id?: unknown; runId?: unknown; cursor?: unknown; events?: unknown } | null;
  /** The host answered with a snapshot the client should bootstrap from. */
  runSnapshot?: boolean;
  nextSinceIdx?: number;
  hasMore?: boolean;
  nextOffset?: number;
}

/** A run's log, as one read returned it. */
export interface RunReplay {
  /**
   * The journal events this read fetched.
   *
   * Empty when the host answered with a snapshot: there, `projection` carries
   * the run's whole log instead, and appending these raw deltas to folded text
   * would draw the same words twice.
   */
  events: ReplayEvent[];
  /** The newest index the read covered; absent for a feed that does not number events. */
  watermark?: number;
  /** How many source events the range held before a trim; see `tailCoversRange`. */
  rawEvents?: number;
  /** The host reported a trimmed or dropped feed. */
  truncated?: boolean;
  /**
   * A whole-run projection to rebuild the run from — folded prefix plus the tail
   * that arrived while it was read. Present only for a snapshot bootstrap, which
   * is the one read that covers a run from its beginning.
   */
  projection?: ReplayProjection | null;
  /**
   * The read provably covers the run from index 0.
   *
   * True for a snapshot bootstrap, and for a whole-run read whose events are
   * contiguous from 0 through the watermark. A tail top-up never sets it: it
   * fills a gap whose position the caller already knows, and says nothing about
   * what came before it.
   */
  completePrefix: boolean;
}

/** Read one page of a run's log. The client owns how (chunked reads, lanes). */
export type ReplayPageReader = (command: {
  type: "get_events_since";
  sessionId: string;
  runId: string;
  sinceIdx: number;
  limit: number;
  offset: number;
  preferSnapshot?: boolean;
  replayUntilIdx?: number;
}) => Promise<ReplayPage | null | undefined>;

/**
 * Read a run's log from `sinceIdx` through a pinned watermark.
 *
 * `sinceIdx = -1` reads the run from its beginning, which is the only form that
 * can establish `completePrefix`. Anything else is a tail: it fills a gap at a
 * position the caller already knows.
 *
 * Throws — rather than returning something partial — when a reply cannot be
 * validated (`replay_window_changed`, `replay_prefix_invalid`,
 * `replay_projection_invalid`, `replay_cursor_stalled`, `stale_sync_lane`). A
 * page that cannot be verified must not be applied, and the caller's answer is
 * to reconcile again rather than to render what it cannot vouch for.
 *
 * The tail's own completeness is *not* checked here: `tailCoversRange` is
 * exported for the caller, which is where the consequence lives (the phone
 * invalidates its cached prefix, the desktop re-reads the record). Both call it
 * with the `watermark`/`rawEvents` this returns.
 */
export async function readRunReplay(options: {
  readPage: ReplayPageReader;
  sessionId: string;
  runId: string;
  sinceIdx: number;
  isCurrent?: () => boolean;
}): Promise<RunReplay> {
  const { readPage, sessionId, runId, sinceIdx } = options;
  const isCurrent = options.isCurrent ?? (() => true);
  const events: ReplayEvent[] = [];
  let bootstrap: ReplayProjection | undefined;
  /**
   * A projection a host sent *without* the snapshot flag.
   *
   * An older bridge answers a first replay page with a folded projection directly
   * (`projection` beside the raw events) rather than in the `runSnapshot` envelope.
   * It is the same thing to a reader — one coherent whole-run replacement — so it
   * is carried out the same way rather than being folded in as deltas.
   */
  let legacyProjection: ReplayProjection | undefined;
  let truncated = false;
  let offset = 0;
  let cursor = sinceIdx;
  let watermark: number | undefined;
  // Totals across the pages, so the completeness check at the end speaks for the
  // whole tail rather than one page. `rawEvents` only counts when every page
  // states it: a partial total would be a wrong total.
  let rawEvents = 0;
  let rawEventsComplete = true;

  for (;;) {
    // An in-flight request may finish, but a hidden or replaced lane must not
    // keep issuing pages, or accumulate a replay nobody is displaying.
    if (!isCurrent())
      throw new Error("stale_sync_lane");
    const page = await readPage({
      type: "get_events_since",
      sessionId,
      runId,
      sinceIdx: cursor,
      limit: REPLAY_PAGE_EVENTS,
      offset,
      // Only the first request of a whole-run read may bootstrap: a host answers
      // a snapshot for "the run from its beginning, first page", and one in the
      // middle of a tail would replace what the client already holds.
      ...(sinceIdx === -1 && cursor === -1 && offset === 0 && watermark === undefined && !bootstrap
        ? { preferSnapshot: true }
        : {}),
      ...(watermark === undefined ? {} : { replayUntilIdx: watermark }),
    });
    if (!isCurrent())
      throw new Error("stale_sync_lane");
    if (!page)
      throw new Error("replay_page_missing");

    if (page.runSnapshot) {
      const raw = page.projection;
      const boundary = raw?.cursor;
      const folded = raw ? normalizeReplayEvents(raw.events) : [];
      const projectionRun = typeof raw?.runId === "string"
        ? raw.runId
        : typeof raw?.run_id === "string"
          ? raw.run_id
          : "";
      let previous = -1;
      if (bootstrap || cursor !== -1 || offset !== 0 || watermark !== undefined
        || !raw || !Number.isSafeInteger(boundary) || (boundary as number) < 0
        || boundary !== page.watermark || folded.length === 0
        || (Array.isArray(page.events) && page.events.length > 0) || page.hasMore
        || (projectionRun !== "" && projectionRun !== runId)
        // Both ends reject an empty or reordered list: the folded events keep
        // their own `idx`, and only strict order proves the snapshot is the run
        // it claims to be rather than a slice of it.
        || folded.some((event) => {
          const idx = event.idx;
          const valid = Number.isSafeInteger(idx) && (idx as number) > previous
            && (idx as number) <= (boundary as number)
            && (event.runId === "" || event.runId === runId);
          previous = idx ?? -1;
          return !valid;
        })) {
        throw new Error("replay_projection_invalid");
      }
      bootstrap = { runId: projectionRun, cursor: boundary as number, events: folded };
      cursor = boundary as number;
      // Pin a NEW watermark for the tail produced while the immutable snapshot
      // was being read. Never append folded text directly to cached text.
      continue;
    }
    if (bootstrap && page.projection)
      throw new Error("replay_snapshot_tail_replaced");
    const rawProjection = page.projection;
    if (!bootstrap && rawProjection && Array.isArray(rawProjection.events) && rawProjection.events.length > 0) {
      legacyProjection = {
        runId: typeof rawProjection.runId === "string"
          ? rawProjection.runId
          : typeof rawProjection.run_id === "string"
            ? rawProjection.run_id
            : "",
        cursor: typeof rawProjection.cursor === "number" ? rawProjection.cursor : -1,
        events: normalizeReplayEvents(rawProjection.events),
      };
    }
    if (watermark !== undefined && page.watermark !== watermark)
      throw new Error("replay_window_changed");
    if (Number.isSafeInteger(page.watermark))
      watermark = page.watermark;
    const pageEvents = normalizeReplayEvents(page.events);
    // A page of another run's events must never advance this run's cursor. An
    // event that names no run is stamped by the caller, which is the only side
    // that knows which run it asked for.
    if (pageEvents.some((event) => event.runId !== "" && event.runId !== runId))
      throw new Error("replay_run_changed");
    events.push(...pageEvents);
    if (Number.isSafeInteger(page.rawEvents) && (page.rawEvents as number) >= 0)
      rawEvents += page.rawEvents as number;
    else
      rawEventsComplete = false;
    if (page.truncated)
      truncated = true;
    if (!page.hasMore)
      break;
    if (Number.isSafeInteger(page.watermark)) {
      const next = page.nextSinceIdx;
      if (!Number.isSafeInteger(next) || (next as number) <= cursor)
        throw new Error("replay_cursor_stalled");
      cursor = next as number;
      offset = 0;
      continue;
    }
    const nextOffset = page.nextOffset;
    if (typeof nextOffset !== "number" || nextOffset <= offset)
      break;
    offset = nextOffset;
  }

  if (bootstrap) {
    const boundary = watermark;
    if (truncated || boundary === undefined
      || !tailCoversRange(events, bootstrap.cursor, boundary, rawEventsComplete ? rawEvents : undefined)) {
      throw new Error("replay_prefix_invalid");
    }
    // One coherent projection through the tail watermark: the snapshot's folded
    // events, then everything the run wrote while it was being read. A snapshot
    // whose text the host blanked (the lean lane) is still usable — the events it
    // keeps are the ones a client renders.
    return {
      events: [],
      watermark: boundary,
      ...(truncated ? { truncated } : {}),
      projection: { ...bootstrap, cursor: boundary, events: [...bootstrap.events, ...events] },
      completePrefix: true,
    };
  }

  const last = events.length - 1;
  const contiguous = events.every((event, index) => event.idx === index);
  const completePrefix = sinceIdx === -1 && !truncated && contiguous
    && (watermark === undefined || watermark === last);
  return {
    events,
    ...(watermark === undefined ? {} : { watermark }),
    ...(rawEventsComplete && rawEvents > 0 ? { rawEvents } : {}),
    ...(truncated ? { truncated } : {}),
    ...(legacyProjection ? { projection: legacyProjection } : {}),
    // A projection *is* the whole run: whatever this read's raw events said, the
    // caller rebuilds from the projection rather than from the deltas here (which
    // a host that sent both would have duplicated).
    completePrefix: legacyProjection ? true : completePrefix,
  };
}
