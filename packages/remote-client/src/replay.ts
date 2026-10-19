import type { RemoteClient } from "./client";
import type { ReplayPage, RunReplay } from "@future-os/thread-projection";
import { readRunReplay } from "@future-os/thread-projection";
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
  return readRunReplay({
    readPage: async (command) => (await requestReadPage<EventsPage>(client, command, sessionId, isCurrent)).data,
    sessionId,
    runId,
    sinceIdx,
    isCurrent,
  });
}
