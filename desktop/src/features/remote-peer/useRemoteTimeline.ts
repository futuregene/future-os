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
}

/**
 * The host's page size for a backward history read, in *user exchanges* (not
 * entries) — the same unit the phone asks in, because that is the unit the
 * host's cursor counts in. Asking in entries would misalign the cursor.
 */
const HISTORY_PAGE_USER_EXCHANGES = 100;

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
  }, [enabled, desktopId, sessionId]);

  return useMemo(
    () => ({ entries, loading, loadingOlder, hasMore, error, loadOlder, refresh, streaming }),
    [entries, loading, loadingOlder, hasMore, error, loadOlder, refresh, streaming],
  );
}

function readString(value: unknown, key: string): string | null {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "string" ? field : null;
}

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
