import type { SessionEntry } from "@future-os/thread-projection";
import type { RemotePeerEvent } from "./remotePeerClient";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { requestRemotePeer } from "./remotePeerClient";

/**
 * One entry in a remote session's history.
 *
 * The host serves the same `get_session_entries` page the phone reads, whose
 * entries are `SessionEntry` records — the same shape this app already renders
 * for its *local* sessions. They are deliberately not converted into local
 * session records: that would be a second copy of another machine's transcript,
 * and every path that treats a local record as authoritative would then be
 * wrong about it.
 */
export type RemoteEntry = SessionEntry;

export interface RemoteTimeline {
  entries: RemoteEntry[];
  /** True while the first page is being read. */
  loading: boolean;
  /** True while an older page is being appended. */
  loadingOlder: boolean;
  hasMore: boolean;
  error: string | null;
  loadOlder: () => Promise<void>;
  refresh: () => Promise<void>;
  /** The host has an in-flight run for this session. */
  streaming: boolean;
  /**
   * The host is compacting this conversation's context.
   *
   * Advisory, and derived from the host's own events rather than from the fact
   * that we asked: the command only *accepts* the request, and the host decides
   * whether it commits, fails, or finds nothing to do. It gates the composer
   * because a host answering one prompt at a time refuses a prompt sent while
   * it is compacting.
   */
  compacting: boolean;
  /**
   * The entry ids that came from the host's own history read — the only ones it
   * will accept as a persisted entry.
   *
   * A live push carries a run-scoped event id (`s:r:1:idx`), which names a frame
   * on the wire and nothing in the host's store. An action that addresses a
   * stored record (a fork at a settled turn) must use an id from here, and must
   * refuse rather than send an event id that happens to look like one.
   */
  persistedEntryIds: ReadonlySet<string>;
}

/**
 * The host's page size for a backward history read, in *user exchanges* (not
 * entries) — the same unit the phone asks in, because that is the unit the
 * host's cursor counts in. Asking in entries would misalign the cursor.
 */
const HISTORY_PAGE_USER_EXCHANGES = 100;

/**
 * The events that mean a run is in flight on the host, and the ones that settle
 * it.
 *
 * Enumerated rather than "everything that is not terminal": a type this client
 * has never seen is not evidence that the host is running or idle, and guessing
 * either way puts a wrong affordance in the composer.
 */
const RUN_ACTIVITY = new Set([
  "agent_start",
  "run_started",
  "text_chunk",
  "thinking_start",
  "thinking_delta",
  "thinking_end",
  "tool_start",
  "tool_delta",
  "toolcall_delta",
  "tool_end",
  "tool_result",
]);

const RUN_SETTLED = new Set(["run_finished", "run_failed", "agent_end"]);

/**
 * The compaction lifecycle.
 *
 * `compaction_unchanged` is terminal too: the host decided there was nothing
 * worth compacting, which ends the operation without a checkpoint. An operation
 * that ends without one of these would leave the composer waiting forever, so a
 * settled *run* clears it as well — nothing can still be compacting once a run
 * has finished.
 */
const COMPACTION_ACTIVITY = new Set(["compaction_started"]);

const COMPACTION_SETTLED = new Set([
  "compaction_committed",
  "compaction_failed",
  "compaction_unchanged",
  "compaction_end",
]);

interface EntriesPage {
  entries?: unknown[];
  hasMore?: boolean;
  nextOffset?: number;
}

/**
 * A remote session's timeline: history on open, then live events appended.
 *
 * History is read once (and paged backwards on demand); live events arrive as
 * pushes. NATS Core is at-most-once, so a push can be lost — which is why
 * reopening a session re-reads history rather than trusting the stream to have
 * been complete. A gap that silently never fills is the failure worth designing
 * against here, not a duplicated line.
 */
export function useRemoteTimeline(
  desktopId: string | null,
  sessionId: string | null,
  enabled: boolean,
): RemoteTimeline {
  const [entries, setEntries] = useState<RemoteEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [streaming, setStreaming] = useState(false);
  const [compacting, setCompacting] = useState(false);
  const [persistedEntryIds, setPersistedEntryIds] = useState<ReadonlySet<string>>(() => new Set());
  /** The cursor for the next older page (`nextOffset`), or null at the start. */
  const cursorRef = useRef<number | null>(null);
  /**
   * Guards against a slow page for a *previous* session landing after the user
   * has already opened another one — the classic way a timeline shows one
   * conversation's lines under another's title.
   */
  const epochRef = useRef(0);

  const readPage = useCallback(async (
    side: "history" | "older",
    session: string,
    desktop: string,
  ): Promise<RemoteEntry[]> => {
    const epoch = ++epochRef.current;
    if (side === "history")
      setLoading(true);
    else setLoadingOlder(true);
    try {
      const before = side === "older" ? cursorRef.current : null;
      const page = await requestRemotePeer<EntriesPage>(desktop, {
        type: "get_session_entries",
        sessionId: session,
        limit: HISTORY_PAGE_USER_EXCHANGES,
        // `before` is the *previous response's* `nextOffset`; the host counts
        // user exchanges, so a cursor it produced is the only valid one.
        ...(before !== null && before !== undefined ? { before } : {}),
      }, session);
      if (epoch !== epochRef.current)
        return [];
      const parsed = normaliseEntries(page?.entries);
      setEntries(current => (side === "history" ? parsed : [...parsed, ...current]));
      // History is where persisted ids come from; a page replaces the window
      // (a `history` read resets to the newest page), so its ids reset with it
      // and an `older` page only adds.
      setPersistedEntryIds(side === "history"
        ? new Set(parsed.map(item => item.id))
        : current => new Set([...current, ...parsed.map(item => item.id)]));
      const next = typeof page?.nextOffset === "number" ? page.nextOffset : null;
      cursorRef.current = next;
      setHasMore(page?.hasMore === true && next !== null && next > 0);
      setError(null);
      return parsed;
    }
    catch (err) {
      if (epoch === epochRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
      return [];
    }
    finally {
      if (epoch === epochRef.current) {
        setLoading(false);
        setLoadingOlder(false);
      }
    }
  }, []);

  const refresh = useCallback(async () => {
    if (!desktopId || !sessionId)
      return;
    cursorRef.current = null;
    await readPage("history", sessionId, desktopId);
  }, [desktopId, sessionId, readPage]);

  const loadOlder = useCallback(async () => {
    if (!desktopId || !sessionId || !hasMore || cursorRef.current === null)
      return;
    await readPage("older", sessionId, desktopId);
  }, [desktopId, sessionId, hasMore, readPage]);

  // Open: read history. The previous session's lines are cleared first, so they
  // can never appear under the new session's title even for one frame.
  useEffect(() => {
    epochRef.current += 1;
    setEntries([]);
    setHasMore(false);
    setError(null);
    setStreaming(false);
    setCompacting(false);
    setPersistedEntryIds(new Set());
    cursorRef.current = null;
    if (!enabled || !desktopId || !sessionId)
      return;
    void readPage("history", sessionId, desktopId);
  }, [enabled, desktopId, sessionId, readPage]);

  // Live: append this session's events; ignore every other session's.
  useEffect(() => {
    if (!enabled || !desktopId || !sessionId)
      return;
    let dispose: (() => void) | undefined;
    let cancelled = false;
    void listen<RemotePeerEvent>("remote-peer-event", (event) => {
      const { desktopId: source, kind, payload } = event.payload;
      // A link that came back: whatever the host pushed while it was gone is
      // gone for good (NATS Core is at-most-once), so the transcript is re-read
      // from the host rather than trusted to have stayed complete. This runs
      // before the `kind` check below, which only handles session events.
      if (kind === "resumed") {
        if (source === desktopId)
          void readPage("history", sessionId, desktopId);
        return;
      }
      if (kind !== "event" || source !== desktopId)
        return;
      if (readString(payload, "sessionId") !== sessionId)
        return;
      const type = readString(payload, "type");
      if (!type)
        return;
      // A run's activity and its settlement are the only things `streaming`
      // tracks. Any other family of event — a compaction, a settings change —
      // must leave it alone: the previous `!isTerminal(type)` reading treated
      // every unrecognized type as "a run just started", so a compaction left
      // the conversation looking busy (and the composer offering Stop) with no
      // run to stop.
      if (RUN_ACTIVITY.has(type))
        setStreaming(true);
      else if (RUN_SETTLED.has(type))
        setStreaming(false);
      // A settled run also ends any compaction: neither can outlive the other,
      // and a terminal compaction frame lost over at-most-once NATS must not
      // leave the composer permanently waiting.
      if (COMPACTION_ACTIVITY.has(type))
        setCompacting(true);
      else if (COMPACTION_SETTLED.has(type) || RUN_SETTLED.has(type))
        setCompacting(false);
      const entry = entryFromEvent(payload, type);
      if (!entry)
        return;
      setEntries(current => (current.some(existing => existing.id === entry.id)
        ? current
        : [...current, entry]));
    }).then((unlisten) => {
      if (cancelled)
        unlisten();
      else dispose = unlisten;
    }).catch(() => {
      // Not under Tauri (tests, a browser harness): history still renders.
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [enabled, desktopId, sessionId, readPage]);

  return useMemo(
    () => ({
      entries,
      loading,
      loadingOlder,
      hasMore,
      error,
      loadOlder,
      refresh,
      streaming,
      compacting,
      persistedEntryIds,
    }),
    [entries, loading, loadingOlder, hasMore, error, loadOlder, refresh, streaming, compacting, persistedEntryIds],
  );
}

function readString(value: unknown, key: string): string | null {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "string" ? field : null;
}

/**
 * The persisted user entry a fork of `assistantEntryId` would branch at.
 *
 * The fork point is the preceding *user* entry — the turn that produced the
 * reply — never the reply itself, mirroring the desktop's own Fork button. That
 * entry's `id` is what the projection exposes as the turn's `sourceEntryId`.
 */
export function precedingUserEntryId(
  entries: RemoteEntry[],
  assistantEntryId: string,
): string | null {
  const index = entries.findIndex(item => item.id === assistantEntryId);
  if (index < 0)
    return null;
  for (let i = index - 1; i >= 0; i -= 1) {
    const previous = entries[i]!;
    if (previous.role === "user")
      return previous.id;
  }
  return null;
}

/**
 * A history page's entries.
 *
 * Entries are kept verbatim once they have the two fields every renderer needs
 * (an identity and a role); anything else — a lean-trimmed record, a shape from
 * a newer host — is dropped rather than rendered as an empty bubble. A page
 * that drops entries is still correct to page from, because the cursor comes
 * from the response, not from what rendered.
 */
function normaliseEntries(raw: unknown): RemoteEntry[] {
  if (!Array.isArray(raw))
    return [];
  return raw.filter((item): item is RemoteEntry => {
    if (typeof item !== "object" || item === null)
      return false;
    const record = item as Record<string, unknown>;
    return typeof record.id === "string"
      && record.id.length > 0
      && (record.role === "user"
        || record.role === "assistant"
        || record.role === "tool"
        || record.role === "system");
  });
}

/**
 * A live event, as a timeline entry.
 *
 * Only the kinds that carry a settled message become entries. Streaming deltas
 * (`text_chunk` and friends) are what the host's own history read already
 * summarized, so appending them here would double every message; the live
 * state* they carry is surfaced by `streaming` and by the catalogue's own
 * refresh instead.
 */
function entryFromEvent(payload: unknown, type: string): RemoteEntry | null {
  const id = readString(payload, "eventId");
  const at = readString(payload, "timestamp");
  if (!id || !at)
    return null;
  const data = readJson(payload, "data");
  const createdAtMs = Date.parse(at);
  if (Number.isNaN(createdAtMs))
    return null;
  const base = {
    id,
    createdAtMs,
    runId: readString(payload, "runId"),
    blocks: [] as RemoteEntry["blocks"],
  };
  if (type === "agent_text" || type === "assistant_message") {
    const text = readString(data, "text") ?? readString(data, "content");
    return text ? { ...base, kind: type, role: "assistant", blocks: [{ kind: "text", text }] } : null;
  }
  if (type === "user_message") {
    const text = readString(data, "text") ?? readString(data, "content");
    return text ? { ...base, kind: type, role: "user", blocks: [{ kind: "text", text }] } : null;
  }
  return null;
}

function readJson(value: unknown, key: string): unknown {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  if (typeof field !== "string")
    return field ?? null;
  try {
    return JSON.parse(field);
  }
  catch {
    return null;
  }
}
