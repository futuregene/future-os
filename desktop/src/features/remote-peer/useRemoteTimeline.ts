import type { AgentMessage, SessionEntry } from "@future-os/thread-projection";
import type { RemotePeerEvent } from "./remotePeerClient";
import { entriesToMessages } from "@future-os/thread-projection";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { requestRemotePeer } from "./remotePeerClient";
import { RemoteRunStreams } from "./remoteRunStreams";
import { forgetRemoteToolTargets } from "./remoteToolTarget";

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
  /**
   * The transcript, as messages — the shape this app draws its own in.
   *
   * The host serves *entries*: one per event on its agent's journal, so a single
   * reply arrives as a thinking entry, several tool entries and a text entry.
   * The local view has never drawn entries directly; it folds them into messages
   * (`entriesToMessages`: one user turn and one reply each). Rendering entries
   * one-per-message put an author line above every fragment, which is what this
   * looked like on a real conversation — a column of "assistant", most of them
   * empty, each with a line or two under it.
   */
  messages: AgentMessage[];
  /**
   * The reply the host is writing right now, or `null` when nothing is running.
   *
   * A remote conversation had no live reply at all: the transcript only learned
   * what the host had *persisted*, so a prompt sent from this window was followed
   * by silence until the run ended — the answer appeared whole, seconds later,
   * with nothing in between. The host publishes its agent's events the moment they
   * happen, so the reply is drawn from those (`remoteRunStreams`), which is what
   * makes a remote run behave like one on this machine.
   *
   * Kept apart from `messages` rather than folded into it: this is a projection of
   * events in flight, not a record, and the two are replaced by each other — the
   * settled reply arrives in `messages` when the history is re-read at the end of
   * the run.
   */
  live: AgentMessage | null;
  /**
   * Whether the host would take a compact request for this conversation.
   *
   * The host's own preconditions, mirrored so the affordance appears only where
   * it would be accepted: there is a conversation, there is something to compact,
   * and no run is in flight (a host answering one prompt at a time refuses one
   * then).
   */
  canCompact: boolean;
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
 *
 * Three, which is what the phone asks for, and the difference is not cosmetic:
 * this client asked for **100** and the host answered with the whole
 * conversation. Every open paid for a hundred exchanges of history before the
 * first line could be drawn — on a long conversation, the reason a remote one
 * took seconds to appear where the phone took none. Older pages arrive as the
 * reader scrolls up, so the cost of the extra history is only paid for history
 * that is actually looked at.
 *
 * **This value and the phone's are one contract, in two files.** They diverged
 * once (100 against 3) precisely because neither side could see the other, so
 * each is named in the other's comment: changing one is changing both. The two
 * clients are separate implementations on purpose — this one must survive a
 * minimised window, which the phone's does not have to — but a page size is a
 * fact about the host's cursor, not a choice either client gets to make alone.
 *
 * The phone's copy: `mobile/src/remote/timeline/historyPaging.ts`.
 */
const HISTORY_PAGE_USER_EXCHANGES = 3;

/**
 * The cursor that means "the page at the end of this session".
 *
 * A history read is *backward*: the host serves the `limit` user exchanges that
 * end at `before`, and only that cursor can name the tail — there is no offset
 * a client could pass to mean "last". Leaving `before` off does not mean the
 * newest page; it is the host's other read (a forward page from offset 0), so
 * the client was handed the *oldest* entries of a conversation as its first
 * page: one question, its reply's first frames, and nothing else. The page
 * after it then repeated the same ordinals, which is why "load earlier" showed
 * the question and the reply a second time.
 *
 * `Number.MAX_SAFE_INTEGER`, the same value the phone opens with, and the same
 * value the host documents as its first paint (`NEWEST_PAGE_CURSOR` in
 * `desktop/src-tauri/src/remote_host/business/history.rs`): anything at or above
 * it means "the end of the session", anything below it is a scrolled-up page.
 */
const HISTORY_TAIL_CURSOR = Number.MAX_SAFE_INTEGER;

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
 * What was last read for one conversation, kept across opens.
 *
 * Without this, every click on a conversation threw away what was on screen and
 * asked the host for it again: the transcript blanked, then reappeared a page at
 * a time. The phone does not behave that way — it keeps what it has read and
 * refreshes behind it — and switching between two conversations is a thing
 * people do constantly, so the second visit cost the same as the first.
 *
 * In memory only, and never written anywhere: it is one machine's view of
 * another machine's transcript for as long as this window is open. What it must
 * not become is a second copy of the host's history on disk, which is the same
 * reason the entries are never turned into local thread records.
 */
interface TimelineSnapshot {
  cursor: number | null;
  entries: RemoteEntry[];
  hasMore: boolean;
  persistedIds: ReadonlySet<string>;
}

const snapshots = new Map<string, TimelineSnapshot>();

function snapshotKey(desktopId: string, sessionId: string): string {
  return `${desktopId}::${sessionId}`;
}

/** Drop a conversation's snapshot, so the next open reads it fresh. */
export function forgetRemoteTimeline(desktopId: string, sessionId: string): void {
  snapshots.delete(snapshotKey(desktopId, sessionId));
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
  const [live, setLive] = useState<AgentMessage | null>(null);
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
   * The live replies of this conversation's runs, folded from the host's pushes.
   *
   * A ref rather than state: the projectors are an accumulator (each delta builds
   * on the last), and the bubble they produce is what is rendered. Creating it
   * lazily keeps it per-window rather than per-hook-instance, which is the same
   * lifetime the snapshots above have.
   */
  const streamsRef = useRef<RemoteRunStreams | null>(null);
  if (!streamsRef.current)
    streamsRef.current = new RemoteRunStreams();

  /**
   * The entries folded into messages, by the same projector this app uses for
   * its own sessions — imported rather than reimplemented, because "one author
   * line per reply" is a rule with several parts (run identity, compaction
   * dividers, aborted turns) that only stays right in one place.
   */
  const messages = useMemo(() => entriesToMessages(entries), [entries]);
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
    /**
     * Whether something is already on screen for this conversation.
     *
     * A catch-up read over a transcript the reader can already see must not put
     * a spinner there: the spinner's job is to say "there is nothing yet", and a
     * conversation that is already drawn is not that.
     */
    background = false,
  ): Promise<RemoteEntry[]> => {
    const epoch = ++epochRef.current;
    if (side === "history") {
      if (!background)
        setLoading(true);
    }
    else {
      setLoadingOlder(true);
    }
    try {
      // The tail for a first/catch-up read, the previous response's
      // `nextOffset` for a scrolled-up one. The host counts user exchanges, so
      // a cursor it produced is the only valid one — and the tail cursor is the
      // only value that means "not a scrolled-up read" to it.
      const before = side === "older" ? cursorRef.current : HISTORY_TAIL_CURSOR;
      const page = await requestRemotePeer<EntriesPage>(desktop, {
        type: "get_session_entries",
        sessionId: session,
        limit: HISTORY_PAGE_USER_EXCHANGES,
        ...(before !== null && before !== undefined ? { before } : {}),
      }, session);
      if (epoch !== epochRef.current)
        return [];
      const parsed = normaliseEntries(page?.entries);
      const next = typeof page?.nextOffset === "number" ? page.nextOffset : null;
      // An older page is prepended, and never twice: an overlapping page (a
      // cursor that did not advance, a host that re-sent what it already
      // served) would otherwise draw the same question and reply a second time
      // under a "load earlier" button that already had them.
      setEntries((current) => {
        if (side === "history")
          return parsed;
        const seen = new Set(current.map(item => item.id));
        return [...parsed.filter(item => !seen.has(item.id)), ...current];
      });
      // History is where persisted ids come from; a page replaces the window
      // (a `history` read resets to the newest page), so its ids reset with it
      // and an `older` page only adds.
      setPersistedEntryIds(side === "history"
        ? new Set(parsed.map(item => item.id))
        : current => new Set([...current, ...parsed.map(item => item.id)]));
      cursorRef.current = next;
      setHasMore(page?.hasMore === true && next !== null && next > 0);
      // Kept for the next open, which is where the point of it is: a `history`
      // read holds the newest page (an `older` read adds to what is there), and
      // the cursor travels with it so paging up continues where it left off.
      snapshots.set(snapshotKey(desktop, session), {
        cursor: next,
        entries: side === "history" ? parsed : [],
        hasMore: page?.hasMore === true && next !== null && next > 0,
        persistedIds: new Set(parsed.map(item => item.id)),
      });
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

  // Open: show what was read last time, then read the newest page again.
  //
  // Both halves matter. The seeded state is what makes a revisit instant — the
  // reader sees their conversation rather than a blank column — and the re-read
  // is what keeps it honest: NATS Core is at-most-once, so anything the host
  // pushed while this window was elsewhere may never arrive, and a transcript
  // that trusted the stream would be quietly missing it.
  useEffect(() => {
    epochRef.current += 1;
    const cached = desktopId && sessionId ? snapshots.get(snapshotKey(desktopId, sessionId)) : undefined;
    setEntries(cached?.entries ?? []);
    setPersistedEntryIds(cached?.persistedIds ?? new Set());
    cursorRef.current = cached?.cursor ?? null;
    setHasMore(cached?.hasMore ?? false);
    setError(null);
    setStreaming(false);
    setCompacting(false);
    // Another conversation's reply must not be drawn under this one's title, and
    // its projectors are deltas that only apply to the run that produced them.
    streamsRef.current?.clear();
    setLive(null);
    // The catch-up read is not a first read: with something already on screen
    // there is no empty state to cover, so the spinner would only say "this is
    // provisional", which the seeded transcript already is not.
    if (cached)
      setLoading(false);
    if (!enabled || !desktopId || !sessionId)
      return;
    void readPage("history", sessionId, desktopId, Boolean(cached));
    // Leaving this conversation drops its fetched tool arguments. They are keyed
    // by run and call id, which never repeat, so nothing here would ever be read
    // again — and the cache would otherwise grow for the life of the window.
    return () => forgetRemoteToolTargets(desktopId, sessionId);
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
      // The reply as it is written. Every frame of a run carries its `runId` and
      // its run-scoped `idx`, which is what the projector orders by; a frame
      // without them (a settings change, a presence tick) is not part of a reply.
      const runId = readString(payload, "runId");
      const idx = readNumber(payload, "idx");
      const rawData = readRaw(payload, "data");
      const streams = streamsRef.current;
      if (runId && idx !== null && rawData !== null && streams) {
        setLive(streams.apply({
          data: rawData,
          eventId: readString(payload, "eventId") ?? `${runId}:${idx}`,
          idx,
          runId,
          timestamp: readString(payload, "timestamp") ?? "",
          type,
        }));
      }
      // The run is over: the host's journal has the reply now, so the transcript
      // re-reads it and the live bubble steps aside. Without this the bubble stays
      // and the settled reply lands *beside* it — the same words twice, one of them
      // frozen mid-stream. The read is a catch-up (`background`), so no spinner
      // replaces a reply the reader is already looking at.
      if (RUN_SETTLED.has(type)) {
        if (runId)
          streams?.settle(runId);
        setLive(null);
        void readPage("history", sessionId, desktopId, true);
      }
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
      messages,
      live,
      canCompact: sessionId !== null && sessionId !== ""
        && entries.length > 0
        && !streaming
        && !compacting,
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
    [entries, messages, live, sessionId, streaming, compacting, loading, loadingOlder, hasMore, error, loadOlder, refresh, persistedEntryIds],
  );
}

function readString(value: unknown, key: string): string | null {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "string" ? field : null;
}

/** A number field of a pushed event, or null when it is absent or not a number. */
function readNumber(value: unknown, key: string): number | null {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "number" && Number.isFinite(field) ? field : null;
}

/**
 * An event's payload as the string the projector parses.
 *
 * The host sends it as a JSON *string* (that is how the agent's own body spells
 * it), and the projector expects the string too — it parses it itself, and a
 * value re-serialized on the way would only be the same bytes in more work. A
 * body that arrives already decoded is re-encoded, so a host that changes shape
 * does not silently stop streaming.
 */
function readRaw(value: unknown, key: string): string | null {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  if (typeof field === "string")
    return field;
  if (field === undefined || field === null)
    return null;
  try {
    return JSON.stringify(field);
  }
  catch {
    return null;
  }
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
 *
 * The envelope's `eventId`/`timestamp` are preferred when they are there, and
 * the event's own record is the fallback: a lean host trims the envelope to
 * `type`/`data`/`runId`/`idx` (`remote_host/lean.rs`), and a `user_message`
 * always carries its entry id and the host's wall clock in `data`
 * (`agent/src/rpc/session_prompt.rs`). Without the fallback a lean feed's live
 * lane produces no entry at all, because this used to return `null` the moment
 * the envelope was missing either field.
 */
function entryFromEvent(payload: unknown, type: string): RemoteEntry | null {
  const data = readJson(payload, "data");
  const runId = readString(payload, "runId");
  const idx = readNumber(payload, "idx");
  const id = readString(payload, "eventId")
    ?? readString(data, "entry_id")
    // Last resort, and only for the frames that have one: a run-scoped frame
    // id is unique within its run, so dedup still holds — it just names a frame
    // on the wire rather than an entry in the host's store.
    ?? (runId !== null && idx !== null ? `${runId}:${idx}` : null);
  if (!id)
    return null;
  const at = readString(payload, "timestamp");
  const createdAtMs = at !== null ? Date.parse(at) : readNumber(data, "created_at_ms") ?? Date.now();
  if (Number.isNaN(createdAtMs))
    return null;
  const base = {
    id,
    createdAtMs,
    runId,
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
