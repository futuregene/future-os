import type { RemoteClient } from "../client";
import { requestReadPage } from "../readPages";
import { SyncEngine } from "../syncEngine";
import { type TimelineState } from "../timeline";
import type { EntriesData } from "../types";
import { HISTORY_PAGE_USER_EXCHANGES, HISTORY_TAIL_CURSOR } from "@future-os/thread-projection";

// One deadline covers transport retries AND waiting behind a replay. Expiry
// cancels the transaction, so late responses/queued mutations cannot skip pages.
export const HISTORY_PAGE_TIMEOUT_MS = 30_000;

// The page size and the "newest page" cursor are facts about the host's cursor,
// shared with the desktop client: see `packages/thread-projection/src/historyPage.ts`.
// Re-exported here so this module stays the phone's one history-paging entry.
export { HISTORY_PAGE_USER_EXCHANGES, HISTORY_TAIL_CURSOR };

export async function readHistoryPage(
  client: RemoteClient,
  sessionId: string,
  before: number,
  isCurrent: () => boolean,
  untrimmed = false,
): Promise<EntriesData> {
  const response = await requestReadPage<EntriesData>(client, {
    type: "get_session_entries", sessionId, before, limit: HISTORY_PAGE_USER_EXCHANGES,
    // The bridge's byte-budget trim sheds whole oldest exchanges from a
    // backward page; the gap-fill integrity check below needs the page to
    // end flush with the requested cursor, so it opts out explicitly.
    ...(untrimmed ? { untrimmed: true } : {}),
  }, sessionId, isCurrent);
  return response.data;
}

export type HistoryPagingState = NonNullable<TimelineState["historyWindow"]> & { loading: boolean; };

export function latestTimelineWindow(
  timeline: TimelineState,
  userExchangeCount: number,
): TimelineState {
  let remaining = userExchangeCount;
  let start = 0;
  for (let index = timeline.items.length - 1; index >= 0; index -= 1) {
    const item = timeline.items[index];
    if (item?.kind !== "message" || item.role !== "user") continue;
    remaining -= 1;
    if (remaining === 0) {
      start = index;
      break;
    }
  }
  if (remaining > 0 || start === 0) return timeline;
  const items = timeline.items.slice(start);
  const visibleIds = new Set(items.map((item) => item.id));
  return {
    ...timeline,
    items,
    durableItemIds: new Set(
      [...(timeline.durableItemIds ?? [])].filter((id) => visibleIds.has(id)),
    ),
  };
}

export function prependHistoryPage(
  live: TimelineState,
  older: TimelineState,
): TimelineState {
  const liveIds = new Set(live.items.map((item) => item.id));
  const olderItems = older.items.filter((item) => !liveIds.has(item.id));
  return {
    ...live,
    items: [...olderItems, ...live.items],
    durableItemIds: new Set([
      ...(live.durableItemIds ?? []),
      ...(older.durableItemIds ?? []),
    ]),
  };
}

/** A page is complete when the authoritative sync lane publishes its merge.
 * React rendering and native layout are deliberately outside this contract.
 */
export function commitHistoryPage(
  engine: SyncEngine,
  sessionId: string,
  older: TimelineState,
  isCurrent: () => boolean,
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    let applied = false;
    const cleanup = () => {
      unsubscribe();
      signal.removeEventListener("abort", abort);
    };
    const abort = () => {
      cleanup();
      reject(new Error("history_page_cancelled"));
    };
    const unsubscribe = engine.subscribe((commit) => {
      if (commit.sessionId !== sessionId || !applied) return;
      cleanup();
      resolve();
    });
    signal.addEventListener("abort", abort);
    if (!isCurrent()) {
      abort();
      return;
    }
    engine.mutate(sessionId, (live) => {
      if (!isCurrent()) {
        abort();
        return live;
      }
      applied = true;
      return prependHistoryPage(live, older);
    });
  });
}

/** Replace the loaded tail without evicting a contiguous older prefix. A long
 * exchange can make the bridge return only the next exchange: in that case
 * the durable entry cursor, not a shared UI row, proves the pages are adjacent. */
export function retainOlderHistoryPrefix(
  existing: TimelineState | null,
  latest: TimelineState,
  adjacent: boolean,
): TimelineState {
  if (!existing || latest.items.length === 0) return latest;
  const latestIds = new Set(latest.items.map((item) => item.id));
  const latestRuns = new Set(latest.items.flatMap(item =>
    item.kind === "message" && item.runId ? [`${item.role}:${item.runId}`] : [],
  ));
  // A just-acknowledged prompt has a local id until history arrives. Bind it
  // by role + run, never by text (repeated prompts are distinct exchanges).
  const overlap = existing.items.findIndex(item => latestIds.has(item.id) ||
    (adjacent && item.kind === "message" && !!item.runId && latestRuns.has(`${item.role}:${item.runId}`)),
  );
  if (overlap === 0 || (overlap < 0 && !adjacent)) return latest;
  const durableRuns = new Set(existing.items.flatMap(item =>
    existing.durableItemIds?.has(item.id) && item.runId ? [item.runId] : [],
  ));
  const prefix = overlap < 0
    ? existing.items.filter(item => existing.durableItemIds?.has(item.id) ||
      (!!item.runId && durableRuns.has(item.runId)))
    : existing.items.slice(0, overlap);
  if (prefix.length === 0) return latest;
  return {
    ...latest,
    items: [...prefix, ...latest.items],
    durableItemIds: new Set([
      ...(latest.durableItemIds ?? []),
      ...prefix
        .filter((item) => existing.durableItemIds?.has(item.id))
        .map((item) => item.id),
    ]),
  };
}

export function diagnosticError(error: unknown): {
  name?: string;
  message: string;
  code?: unknown;
} {
  if (error instanceof Error) {
    return {
      name: error.name,
      message: error.message,
      ...(error.cause &&
        typeof error.cause === "object" &&
        "code" in error.cause
        ? { code: (error.cause as { code?: unknown; }).code }
        : {}),
    };
  }
  return { message: String(error) };
}
