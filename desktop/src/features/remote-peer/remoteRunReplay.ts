import type { ContentPage, ReplayEvent, ReplayPage, RunEvent, RunReplay } from "@future-os/thread-projection";
import { copyEventPayload, hydrateRequiredContent, needsContentHydration, parseJsonText, readRunReplay, tailCoversRange, withEventPayload } from "@future-os/thread-projection";
import { requestRemotePeer } from "./remotePeerClient";

/**
 * Reading a host's run log for one conversation.
 *
 * The page loop itself is the shared one (`@future-os/thread-projection`'s
 * `readRunReplay`): every rule in it — how a snapshot bootstrap is validated, how
 * the watermark is pinned, how a trimmed feed proves its own completeness — is a
 * fact about the host's replies, and the phone reads the same feed. What is this
 * client's is the *transport*: the desktop reads through its Rust side
 * (`remote_peer_request`), which declares `chunkedRead` on every command and puts
 * an oversized page back together before the reply reaches JavaScript. So the
 * request here is one call, and the pages arrive whole.
 */

/**
 * A host that does not serve `get_events_since` at all.
 *
 * The command has been on the wire since the phone needed it, so this is an
 * older paired desktop rather than a fault. It is worth distinguishing: the
 * caller stops asking (and answers with the page-plus-pushes path instead of a
 * log it cannot read), where a *failed* read is retried.
 */
export function replayUnsupported(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return /unsupported command/i.test(message);
}

/**
 * One run's log, read from `sinceIdx` through a pinned watermark.
 *
 * `sinceIdx = -1` asks for the run from its beginning and is the only form that
 * can carry a snapshot bootstrap — the host folds a long run into one immutable
 * projection and the client walks it back in chunks.
 */
export async function fetchRunReplay(options: {
  desktopId: string;
  sessionId: string;
  runId: string;
  sinceIdx: number;
  isCurrent?: () => boolean;
}): Promise<RunReplay> {
  const { desktopId, sessionId, runId, sinceIdx } = options;
  const isCurrent = options.isCurrent ?? (() => true);
  const replay = await readRunReplay({
    // One call per page: the Rust side declares `chunkedRead` and reassembles an
    // oversized reply before it reaches here (see `remote_peer::request`).
    readPage: command =>
      requestRemotePeer<ReplayPage>(desktopId, command as unknown as Record<string, unknown>, sessionId, isCurrent),
    sessionId,
    runId,
    sinceIdx,
    isCurrent,
  });
  for (const events of [replay.events, replay.projection?.events ?? []]) {
    for (let index = 0; index < events.length; index += 1) {
      const event = events[index]!;
      if (!event.data.includes("\"content_refs\"") && event.data.length <= 32 * 1024)
        continue;
      const data = await parseJsonText<Record<string, unknown>>(event.data, { isCurrent });
      const hydrated = needsContentHydration(event.type, data)
        ? await hydrateRequiredContent(event.type, data, contentRequest =>
            requestRemotePeer<ContentPage>(desktopId, { type: "get_event_content_range", runId, contentRequest }, sessionId, isCurrent), isCurrent)
        : data;
      events[index] = withEventPayload(event, hydrated);
    }
  }
  return replay;
}

/**
 * Whether a fetched tail provably reaches the watermark it was pinned to.
 *
 * A tail is applied by moving the cursor to the watermark — "I hold everything
 * through here" — so a short feed silently applied there is a hole nothing would
 * ever revisit. The check is the shared `tailCoversRange`; what is this caller's
 * is what to do about it, which is to refuse the apply and read again.
 */
export function replayTailCovers(replay: RunReplay, sinceIdx: number): boolean {
  return tailCoversRange(replay.events, sinceIdx, replay.watermark, replay.rawEvents);
}

/**
 * The events a read returned, as the projector's own event shape.
 *
 * A replay's events are journal deltas: they have an index, a type and a JSON
 * payload, which is exactly what `RunProjector` folds. The event id and time are
 * carried when the envelope has them, and synthesised from the run and its index
 * when it does not — the projector only orders by `sequence`, so an id is a
 * label rather than a key, but a *stable* one is what keeps a rebuilt row from
 * remounting when the same event arrives again.
 */
export function replayEventsToRunEvents(runId: string, events: readonly ReplayEvent[]): RunEvent[] {
  const now = Date.now();
  return events.map((event) => {
    const createdAt = event.timestamp ? Date.parse(event.timestamp) : Number.NaN;
    return copyEventPayload(event, {
      id: event.eventId ?? `${runId}:${event.idx ?? -1}`,
      runId: event.runId || runId,
      eventType: event.type,
      payload: event.data,
      sequence: event.idx ?? -1,
      createdAt: Number.isNaN(createdAt) ? now : createdAt,
    });
  });
}
