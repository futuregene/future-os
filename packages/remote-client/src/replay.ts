import type { RemoteClient } from "./client";
import type { ReplayPage, RunReplay } from "@future-os/thread-projection";
import { hasEventPayload, hydrateRequiredContent, needsContentHydration, parseJsonText, readRunReplay, withEventPayload, type ContentPage } from "@future-os/thread-projection";
import { requestReadPage } from "./readPages";

/**
 * The phone's read of a run's event log.
 *
 * The wire contract — how a run's pages are walked, what a snapshot bootstrap
 * means, when a page must be refused — lives in the shared package
 * (`@future-os/thread-projection`: `readRunReplay`), because both remote clients
 * read the same host and a rule that is only true on one side is a bug on the
 * other. What stays here is the part that is the phone's: reading a page
 * through its own request layer, which reassembles the host's chunked replies
 * client-side and pins them to the bridge that started the read.
 *
 * Types keep their names for this module's callers; the shapes come from the
 * package so there is one definition of each.
 */
export type EventsData = RunReplay;
export type EventsPage = ReplayPage;
export type { ReplayEventWire } from "@future-os/thread-projection";
export { normalizeReplayEvents, REPLAY_PAGE_EVENTS, tailCoversRange } from "@future-os/thread-projection";

/**
 * Prefer semantic bootstrap for a cold run, then fetch its new fixed-window
 * tail. Older peers fall back to raw `get_events_since`, looping paginated
 * replies until `hasMore=false`. The desktop pages replay events under the
 * NATS payload cap (a multi-MB journal tail must never ship as one oversized
 * reply), so a single-shot request would silently truncate. Legacy projection
 * replies pass through unchanged; an opt-in snapshot is extended through its
 * verified tail watermark and restored as one coherent whole-run replacement.
 */
export async function fetchEventsSince(
  client: RemoteClient,
  sessionId: string,
  runId: string,
  sinceIdx: number,
  isCurrent: () => boolean = () => true,
): Promise<EventsData> {
  const replay = await readRunReplay({
    readPage: async (command) => (await requestReadPage<EventsPage>(client, command, sessionId, isCurrent)).data,
    sessionId,
    runId,
    sinceIdx,
    isCurrent,
  });
  replay.events = await hydrateReplayEvents(replay.events);
  if (replay.projection) replay.projection.events = await hydrateReplayEvents(replay.projection.events);
  return replay;
  async function hydrateReplayEvents<T extends { type: string; data: string; runId?: string }>(events: T[]): Promise<T[]> {
    const result: T[] = [];
    for (const event of events) result.push(await hydrateRemoteEvent(client, sessionId, event, isCurrent, runId));
    return result;
  }
}

/** Required fields are loaded within the session's serial lane; optional tool
 * facts remain lazy. Generation checks fence every page before application. */
export async function hydrateRemoteEvent<T extends { type: string; data: string; runId?: string }>(
  client: RemoteClient, sessionId: string, event: T, isCurrent: () => boolean = () => true,
  fallbackRunId = "",
): Promise<T> {
  if (hasEventPayload(event) || (!event.data.includes('"content_refs"') && event.data.length <= 32 * 1024)) return event;
  const data = await parseJsonText<Record<string, unknown>>(event.data, { isCurrent });
  if (!needsContentHydration(event.type, data)) return withEventPayload(event, data);
  const hydrated = await hydrateRequiredContent(event.type, data, async contentRequest => {
    const reply = await requestReadPage<ContentPage>(client, {
      type: "get_event_content_range", sessionId, runId: event.runId || fallbackRunId, contentRequest,
    }, sessionId, isCurrent);
    if (!reply.success) throw new Error(reply.error || "event_content_read_failed");
    return reply.data;
  }, isCurrent);
  return withEventPayload(event, hydrated);
}
