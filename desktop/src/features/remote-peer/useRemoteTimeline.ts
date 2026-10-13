import type { AgentMessage, SessionEntry } from "@future-os/thread-projection";
import type { RemotePeerEvent } from "./remotePeerClient";
import type { RemoteLiveReply } from "./remoteRunStreams";
import { entriesToMessages, HISTORY_PAGE_USER_EXCHANGES, HISTORY_TAIL_CURSOR } from "@future-os/thread-projection";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { requestRemotePeer } from "./remotePeerClient";
import { fetchRunReplay, replayTailCovers, replayUnsupported } from "./remoteRunReplay";
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
   * What this client is drawing from its runs' own logs, oldest first.
   *
   * Two sources feed a remote reply: the host's event pushes (how it arrives as
   * it is written) and the run's log, read back through `get_events_since`. A run
   * this client has read the log of is *rebuilt* from it — the projection covers
   * the run from its beginning, so the transcript's own partial copy of that run
   * is redundant and is hidden (`stripRunCopies`), and the reply the reader sees
   * is this one. A run it has only ever seen pushes for is drawn against the
   * transcript's copy instead (`foldLiveReply`): together those two are the reply
   * — the page has the beginning, the pushes have the end.
   *
   * Empty when nothing is in flight, and normally one entry: a host answers one
   * prompt at a time. It is briefly two when a settled reply is still waiting for
   * its record to be readable while the next run has already started.
   */
  liveReplies: RemoteLiveReply[];
  /**
   * A host that does not serve `get_events_since` (an older paired desktop).
   *
   * Advisory, and only for what it costs: every reply then falls back to the
   * page-plus-pushes path, which the transcript draws by folding the two copies.
   * The reader sees the same reply; what is lost is the run's own clock and the
   * certainty that a missed frame was noticed.
   */
  replayUnavailable: boolean;
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

/** The host's answer to `get_state`, as far as this hook reads it. */
interface RemoteSessionState {
  /** The run in flight, if any: the one a mid-join has to read the log of. */
  activeRun?: { runId?: string } | null;
  isCompacting?: boolean;
}

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
 * A remote session's timeline: history on open, a run's log while it streams.
 *
 * History is read once (and paged backwards on demand); live events arrive as
 * pushes. NATS Core is at-most-once, so a push can be lost — which is why
 * reopening a session re-reads history rather than trusting the stream to have
 * been complete.
 *
 * The reply in flight is the part history cannot answer. A run's *log* is the
 * truth about it (`get_events_since`), and reading it is what lets a client that
 * joined mid-reply draw the whole reply — including the part it never watched
 * arrive, and the run's own start time, which the pushes cannot carry (they begin
 * where the client subscribed). A run's log is read at three moments, and each is
 * a moment where the pushes alone would leave a hole:
 *
 * - **an open**, where the conversation's newest run may already be well along;
 * - **a link that came back**, where pushes were lost with the socket;
 * - **a gap the pushes themselves declare** (a host that does not trim sends one
 *   event per index, so a jump is loss), where only the missing slice is read.
 *
 * A host that does not serve the command says so once; the reply is then drawn
 * the way it was before the log existed — the page's beginning, then the pushed
 * tail (`foldLiveReply`).
 */
export function useRemoteTimeline(
  desktopId: string | null,
  sessionId: string | null,
  enabled: boolean,
): RemoteTimeline {
  const [entries, setEntries] = useState<RemoteEntry[]>([]);
  const [liveReplies, setLiveReplies] = useState<RemoteLiveReply[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [streaming, setStreaming] = useState(false);
  const [compacting, setCompacting] = useState(false);
  const [replayUnavailable, setReplayUnavailable] = useState(false);
  const [persistedEntryIds, setPersistedEntryIds] = useState<ReadonlySet<string>>(() => new Set());
  /** The cursor for the next older page (`nextOffset`), or null at the start. */
  const cursorRef = useRef<number | null>(null);
  /**
   * The live runs of this conversation: their projections, and what this client
   * knows about how much of each run's log it holds.
   *
   * A ref rather than state: the projectors are an accumulator (each delta builds
   * on the last), and the replies they produce are what is rendered. Creating it
   * lazily keeps it per-window rather than per-hook-instance, which is the same
   * lifetime the snapshots above have.
   */
  const streamsRef = useRef<RemoteRunStreams | null>(null);
  if (!streamsRef.current)
    streamsRef.current = new RemoteRunStreams();

  /**
   * Which conversation this hook is currently reading.
   *
   * Every async path (a page, a run's log) compares against it before writing
   * anything: a slow read for a *previous* conversation landing after the user
   * has opened another one is the classic way a timeline shows one conversation's
   * lines under another's title.
   */
  const epochRef = useRef(0);
  /** The reads in flight per run, so a burst of gaps asks once. */
  const replayingRef = useRef(new Set<string>());
  /** Whether the host has refused `get_events_since` (an older paired desktop). */
  const unsupportedRef = useRef(false);
  /**
   * Whether the host's pushes omit event indices by design.
   *
   * This client declares the trimmed lane (`lean_events_v1`), whose feed drops
   * whole event families — so a jump between indices is expected there, and
   * reading it as loss would read the run's log on every frame. It is the host
   * that decides, and its replay pages say so: a page that states how many source
   * events it covered *before* trimming (`rawEvents`) is a page from a host that
   * trims, and one reply settles the question for the connection.
   *
   * Until something proves it, a jump is treated as loss: an unproven gap costs
   * one read, and a lost frame nobody looked for costs the reply.
   */
  const hostTrimsRef = useRef(false);
  /**
   * The runs this conversation has already read a whole log for.
   *
   * A run whose first frame arrives at an index above zero is one this window
   * joined in flight, and its beginning is only in the log. Reading is bounded to
   * one attempt per run per visit: a host that refuses or fails does not get
   * asked again for every frame, and the next open (or a link that comes back)
   * asks once more.
   */
  const prefixAskedRef = useRef(new Set<string>());

  const publishReplies = useCallback(() => {
    setLiveReplies(streamsRef.current?.replies() ?? []);
  }, []);

  /**
   * Read a run's log and rebuild its reply from it.
   *
   * `full` asks for the run from its beginning (the only read that can cover a
   * reply this client joined part-way through, and the only one that carries the
   * run's own start time and a snapshot bootstrap for a long log). Otherwise the
   * read fills a gap at a position this client already knows, which is what a
   * lost push costs.
   */
  const readRunLog = useCallback(async (
    desktop: string,
    session: string,
    runId: string,
    full: boolean,
  ): Promise<void> => {
    const streams = streamsRef.current;
    if (!streams || unsupportedRef.current || replayingRef.current.has(runId))
      return;
    const epoch = epochRef.current;
    const isCurrent = () => epochRef.current === epoch && streamsRef.current === streams;
    const sinceIdx = full ? -1 : (streams.cursorFor(runId)?.highWater ?? -1);
    // A gap whose position this client does not know is a whole-run read: there
    // is nothing to fill from.
    const from = full || sinceIdx < 0 ? -1 : sinceIdx;
    replayingRef.current.add(runId);
    try {
      const replay = await fetchRunReplay({ desktopId: desktop, sessionId: session, runId, sinceIdx: from, isCurrent });
      if (!isCurrent())
        return;
      // A tail is only applied when it provably reaches the watermark it was
      // pinned to: the cursor moves to that point, and a short feed applied there
      // is a hole nothing would ever revisit. Refusing costs one more read.
      if (from >= 0 && !replayTailCovers(replay, from))
        return;
      // A read that carries no events is not evidence about the run: it is what a
      // host answers when there is nothing new, and what a reply that is not a
      // log at all looks like from here. Restoring it would replace the reply
      // this client has already folded with an empty one.
      if (replay.projection?.events?.length === 0 || (replay.events.length === 0 && !replay.projection))
        return;
      // A host that states a pre-trim count on its pages is a host whose pushes
      // are trimmed too (`remote_host::lean` is one switch for both lanes).
      if (replay.rawEvents !== undefined)
        hostTrimsRef.current = true;
      streams.restore(runId, replay);
      publishReplies();
    }
    catch (err) {
      if (!isCurrent())
        return;
      // An older host refuses the command outright. Say so once, and draw the
      // reply from the page plus the pushes from here on.
      if (replayUnsupported(err)) {
        unsupportedRef.current = true;
        setReplayUnavailable(true);
      }
    }
    finally {
      replayingRef.current.delete(runId);
    }
  }, [publishReplies]);

  /** Read a history page, and answer with what it carried. */
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
      if (side === "history")
        settleRunsInPage(streamsRef.current, parsed, publishReplies);
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
  }, [publishReplies]);

  /**
   * Ask the host which run is in flight, and read its log.
   *
   * The state read is what makes a mid-join complete: without it the client only
   * learns of a run when a push arrives, and a long tool call pushes nothing for
   * minutes — so a reader who opened the conversation while the host was working
   * would see the page's partial copy of the reply and no clock on it.
   */
  const reconcileActiveRun = useCallback(async (desktop: string, session: string): Promise<void> => {
    if (unsupportedRef.current)
      return;
    const epoch = epochRef.current;
    let active: string | null = null;
    try {
      const state = await requestRemotePeer<RemoteSessionState>(desktop, {
        type: "get_state",
        sessionId: session,
      }, session);
      if (epoch !== epochRef.current)
        return;
      active = state?.activeRun?.runId ?? null;
      // The host's own answer to "is a run in flight", which is what a window that
      // just opened has no other way of knowing: the pushes only start arriving at
      // whatever the run does next, and a long tool call emits nothing for minutes.
      // Only ever *set* here — a run that ends clears it on its terminal frame,
      // which is the frame that says so.
      if (active)
        setStreaming(true);
      if (typeof state?.isCompacting === "boolean")
        setCompacting(current => (current === state.isCompacting ? current : state.isCompacting === true));
    }
    catch {
      // A host that cannot answer is not evidence about its runs. The pushes
      // still carry the reply; the log read is simply skipped this time.
      return;
    }
    if (!active)
      return;
    // From the run's *beginning*, always, on this path: the moments that reach it
    // are the moments the pushes' provenance is in doubt — a window that just
    // opened, a link that was down, a record that was re-read. This client
    // declares the trimmed lane, whose feed omits event families by design, so
    // "how much of the run do I hold" cannot be answered by counting what arrived;
    // and the read is bounded by the host's own snapshot bootstrap, which folds a
    // long run into one reply.
    await readRunLog(desktop, session, active, true);
  }, [readRunLog]);

  /**
   * Re-read the conversation from the host: its record, then the run in flight.
   *
   * Used by the transcript's own Retry, and after an action that changes what the
   * host's record holds (a compaction that committed, a run that was continued).
   */
  const refresh = useCallback(async () => {
    if (!desktopId || !sessionId)
      return;
    cursorRef.current = null;
    await readPage("history", sessionId, desktopId);
    await reconcileActiveRun(desktopId, sessionId);
  }, [desktopId, sessionId, readPage, reconcileActiveRun]);

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
    unsupportedRef.current = false;
    hostTrimsRef.current = false;
    prefixAskedRef.current.clear();
    const cached = desktopId && sessionId ? snapshots.get(snapshotKey(desktopId, sessionId)) : undefined;
    setEntries(cached?.entries ?? []);
    setPersistedEntryIds(cached?.persistedIds ?? new Set());
    cursorRef.current = cached?.cursor ?? null;
    setHasMore(cached?.hasMore ?? false);
    setError(null);
    setStreaming(false);
    setCompacting(false);
    setReplayUnavailable(false);
    // Another conversation's replies must not be drawn under this one's title,
    // and the projections are accumulators that only apply to the run that
    // produced them — so the lane goes with them, cursors included: they describe
    // how much of *those* runs this client held.
    streamsRef.current?.clear();
    setLiveReplies([]);
    // The catch-up read is not a first read: with something already on screen
    // there is no empty state to cover, so the spinner would only say "this is
    // provisional", which the seeded transcript already is not.
    if (cached)
      setLoading(false);
    if (!enabled || !desktopId || !sessionId)
      return;
    void (async () => {
      await readPage("history", sessionId, desktopId, Boolean(cached));
      // Read what the host has in flight *after* the page: the transcript's own
      // copy of that reply is what the log's read replaces, and a read that
      // landed first would be replaced by the page.
      await reconcileActiveRun(desktopId, sessionId);
    })();
    // Leaving this conversation drops its fetched tool arguments. They are keyed
    // by run and call id, which never repeat, so nothing here would ever be read
    // again — and the cache would otherwise grow for the life of the window.
    return () => forgetRemoteToolTargets(desktopId, sessionId);
  }, [enabled, desktopId, sessionId, readPage, reconcileActiveRun]);

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
        if (source === desktopId) {
          // Both lanes are re-read, because they answer different questions: the
          // page is the host's *record* (a run that started and finished while
          // the link was down is only there), and the log is the run in flight
          // (whose intermediate frames were what the socket dropped).
          streamsRef.current?.restart();
          prefixAskedRef.current.clear();
          publishReplies();
          void readPage("history", sessionId, desktopId, true).then(() =>
            reconcileActiveRun(desktopId, sessionId));
        }
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
        const verdict = streams.apply({
          data: rawData,
          eventId: readString(payload, "eventId") ?? `${runId}:${idx}`,
          idx,
          runId,
          timestamp: readString(payload, "timestamp") ?? "",
          type,
          ...(readNumber(payload, "coalescedCount") === null
            ? {}
            : { coalescedCount: readNumber(payload, "coalescedCount")! }),
        }, { omitIndices: hostTrimsRef.current });
        publishReplies();
        // A frame that lands above index zero of a run this client has no
        // beginning for is a run it joined in flight: the reply's first half is
        // only in the log. (A gap *inside* a run it has been following is the
        // other case, and only a host that does not trim can declare one — see
        // `hostTrimsRef`.)
        const firstHalfMissing = streams.cursorFor(runId)?.prefixComplete === false;
        const gapInsideRun = verdict === "gap" || verdict === "overlap";
        if ((firstHalfMissing || gapInsideRun) && !prefixAskedRef.current.has(runId)) {
          prefixAskedRef.current.add(runId);
          void readRunLog(desktopId, sessionId, runId, firstHalfMissing);
        }
      }
      // The run is over: the host's journal has the reply now, so the transcript
      // re-reads it and this client's own copy steps aside — once that record
      // carries it (`settleRunsInPage`). Without this the reply stays a
      // projection while the settled record is read *past* it, and a reply whose
      // last words were written as the run ended could blink out of the
      // transcript until the next open. The read is a catch-up (`background`), so
      // no spinner replaces a reply the reader is already looking at.
      if (RUN_SETTLED.has(type)) {
        if (runId)
          streams?.settle(runId);
        publishReplies();
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
  }, [enabled, desktopId, sessionId, readPage, reconcileActiveRun, readRunLog, publishReplies]);

  const messages = useMemo(() => entriesToMessages(entries), [entries]);

  return useMemo(
    () => ({
      entries,
      messages,
      liveReplies,
      replayUnavailable,
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
    [entries, messages, liveReplies, replayUnavailable, sessionId, streaming, compacting, loading, loadingOlder, hasMore, error, loadOlder, refresh, persistedEntryIds],
  );
}

/**
 * Drop this client's copy of every run the page just made readable.
 *
 * A settled run's projection is the reply until the host's own record of it can
 * be read — the agent writes the last model call as the run ends, so the record
 * can trail the terminal frame by a moment. Once the page carries that record,
 * the projection is a second copy of the same words, and the record is the one
 * that carries what this lane cannot: the run's failure, its error text, and the
 * entry id a fork branches at.
 *
 * The trigger is the run's own `run.status`, which the agent writes on a settled
 * entry. A *running* status is what the mid-run snapshot carries, so it must not
 * count — that is exactly the copy the log was read to replace.
 */
function settleRunsInPage(
  streams: RemoteRunStreams | null,
  entries: readonly RemoteEntry[],
  publish: () => void,
): void {
  if (!streams)
    return;
  const runs = streams.knownRuns();
  let dropped = false;
  for (const runId of runs) {
    if (entries.some(entry => entry.runId === runId && isSettledStatus(entry.run?.status))) {
      streams.markRecorded(runId);
      dropped = true;
    }
  }
  if (dropped)
    publish();
}

/** The `run.status` values the agent writes on a *settled* entry. */
function isSettledStatus(status: string | null | undefined): boolean {
  return status === "completed" || status === "failed" || status === "cancelled"
    || status === "interrupted" || status === "incomplete";
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
