// Controlled historical trace playback. Production Mobile SyncEngine/replay,
// real HTTP/DesktopHost/Agent reads; activeRun and visible watermark are test
// controls, NOT observations of a currently streaming run. No invented tokens.
import { SyncEngine, type SyncTiming } from "../mobile/src/remote/syncEngine";
import { fetchEventsSince, type EventsPage } from "../mobile/src/remote/replay";
import { requestReadPage } from "../mobile/src/remote/readPages";
import { emptyTimeline, timelineFromEntries, type TimelineState } from "../mobile/src/remote/timeline";
import type { RemoteClient } from "../mobile/src/remote/client";
import type { EntriesData, RemoteCommand, RemoteSessionState } from "../mobile/src/remote/types";

type Sample = { label: string; session: string; run: string; expected_events: number };
type Trace = { type: string; bytes: number; elapsedMs: number; backendMs: number; phase: string; since?: number };
const results: unknown[] = [];
const rounds = new URLSearchParams(location.hash.slice(1)).get("rounds") === "1" ? 1 : 3;
const output = document.querySelector("pre")!;
const status = document.querySelector("#status")!;
const button = document.querySelector("button")!;
document.querySelector("aside")!.textContent = "Controlled active-run playback of a REAL historical trace. Production Mobile SyncEngine + loopback DesktopHost/Agent. Test controls: activeRun identity, visible watermark, terminal history mirror removal. NOT actual live streaming, NATS/E2EE, or phone rendering. Metrics only; no fake clock or invented token events.";
button.textContent = `Measure cache reuse and fallback (${rounds} rounds)`;

async function digest(timeline: TimelineState | null) {
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(JSON.stringify(timeline?.items))))).map(n => n.toString(16).padStart(2, "0")).join("");
}

async function runRound(sample: Sample, round: number) {
  // This selected trace ends with one agent_end; exclude it so the production
  // projector remains streaming. Assert that below, don't assume it silently.
  const finalCutoff = sample.expected_events - 2;
  let cutoff = finalCutoff - 200;
  let visible = true;
  let phase = "state";
  let trace: Trace[] = [];
  let received = 0;
  let sinceRequests: number[] = [];
  let timing: Omit<SyncTiming, "sessionId" | "runId"> | null = null;
  let fetchMs = 0;
  let finish: (() => void) | undefined;
  let fail: ((error: unknown) => void) | undefined;
  let historyPreview = emptyTimeline();
  const client = {
    async requestRetry<T>(command: RemoteCommand) {
      const start = performance.now();
      const response = await fetch("/rpc", { method: "POST", headers: { "Content-Type": "application/json", "X-Sync-Measurement": "1" }, body: JSON.stringify(command) });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const bytes = await response.arrayBuffer();
      const parsed = JSON.parse(new TextDecoder().decode(bytes));
      trace.push({ type: command.type, phase, since: command.sinceIdx, bytes: bytes.byteLength,
        elapsedMs: performance.now() - start, backendMs: Number(response.headers.get("X-Backend-Ms")) });
      if (!parsed.success) throw new Error(parsed.error ?? "RPC failed");
      if (command.type === "get_events_since") {
        if (parsed.data.readChunk) throw new Error("trace control requires an unchunked replay page");
        const page: EventsPage = parsed.data;
        if (page.projection) throw new Error("projection trace needs separate control");
        const raw = page.events ?? [];
        if (cutoff === finalCutoff && raw.some(e => e.idx === finalCutoff + 1 && e.type !== "agent_end"))
          throw new Error("trace final event is not agent_end");
        page.events = raw.filter(e => e.idx! <= cutoff);
        page.watermark = cutoff;
        page.nextSinceIdx = page.events.at(-1)?.idx ?? command.sinceIdx ?? -1;
        page.hasMore = page.nextSinceIdx < cutoff;
        if (page.hasMore && page.events.length === 0) throw new Error("trace cutoff stalled");
      }
      return parsed as { success: boolean; data: T };
    },
  } as unknown as RemoteClient;
  const create = () => new SyncEngine({
    isSessionVisible: () => visible,
    requestGetState: async sessionId => {
      phase = "state";
      const state = (await client.requestRetry<RemoteSessionState>({ type: "get_state", sessionId })).data;
      // Explicit scenario control: the source is a completed historical run.
      return { ...state, activeRun: { runId: sample.run } };
    },
    requestHistory: async (sessionId, current) => {
      phase = "history";
      const page = await requestReadPage<EntriesData>(client, { type: "get_session_entries", sessionId, before: Number.MAX_SAFE_INTEGER, limit: 3 }, sessionId, current);
      // Do not allow this historical run's terminal mirror to override the
      // controlled active-run state. Other real durable rows are unchanged.
      historyPreview = timelineFromEntries((page.data.entries ?? []).filter(e => e.runId !== sample.run || e.role === "user"));
      return historyPreview;
    },
    fetchReplay: async (session, run, since, current) => {
      phase = "replay";
      sinceRequests.push(since);
      const start = performance.now();
      const result = await fetchEventsSince(client, session, run, since, current);
      fetchMs += performance.now() - start;
      received += result.events?.length ?? 0;
      return { ...result, events: result.events ?? [] };
    },
    onTiming: value => { timing = { reason: value.reason, attempt: value.attempt, elapsedMs: value.elapsedMs, stagesMs: value.stagesMs, replayPlan: value.replayPlan, outcome: value.outcome }; },
    onSyncStatus: (_session, value) => { if (value === "idle") finish?.(); },
    onFailure: error => fail?.(error.error),
  });
  let engine = create();
  const summarizeCache = () => {
    const timeline = engine.timelineFor(sample.session);
    const cursor = engine.cursorFor(sample.session).get(sample.run);
    return { visibleItems: timeline?.items.length ?? 0, highWater: cursor?.highWater ?? null,
      prefixComplete: cursor?.prefixComplete ?? false, streaming: timeline?.streaming ?? false,
      hasProjector: timeline?.liveRuns?.has(sample.run) ?? false,
      // Exactly the production cache budget's estimate (not total heap use).
      cacheBudgetBytes: JSON.stringify(timeline).length * 2 + engine.cursorFor(sample.session).size * 64 };
  };
  async function open(scenario: string, expectedSince: number, expectedEvents: number) {
    trace = []; received = 0; sinceRequests = []; timing = null; fetchMs = 0;
    const before = summarizeCache();
    const start = performance.now();
    const done = new Promise<void>((resolve, reject) => { finish = resolve; fail = reject; });
    const timer = setTimeout(() => fail?.(new Error("sync exceeded 120 seconds")), 120000);
    try {
      void engine.open(sample.session).catch(error => fail?.(error));
      await done;
      const syncMs = performance.now() - start;
      const after = summarizeCache();
      if (received !== expectedEvents || sinceRequests.length !== 1 || sinceRequests[0] !== expectedSince || after.highWater !== cutoff || !after.prefixComplete || !after.streaming || !after.hasProjector)
        throw new Error(`integrity/cache mismatch in ${scenario}: ${JSON.stringify({ received, sinceRequests, after, expectedEvents, expectedSince, cutoff })}`);
      const replay = trace.filter(t => t.phase === "replay");
      const value = { sample: sample.label, round, scenario, syncMs, before, after, expectedEvents, received,
        since: sinceRequests[0], replayRequests: replay.length, replayChunkRequests: replay.filter(t => t.type === "get_read_chunk").length,
        replayReplyBytes: replay.reduce((sum,t)=>sum+t.bytes,0), replayBackendMs: replay.reduce((sum,t)=>sum+t.backendMs,0),
        historyRequests: trace.filter(t => t.phase === "history").length, fetchMs, timing };
      results.push(value);
      output.textContent = JSON.stringify(results, null, 2);
      console.info("SYNC_WARM_RESULT", JSON.stringify(value));
      return value;
    } finally { clearTimeout(timer); finish = undefined; fail = undefined; }
  }
  try {
    status.textContent = `Round ${round}/${rounds}: establish real prefix`;
    await open("cold-prefix", -1, cutoff + 1);
    visible = false;
    // Exercise the production navigation cache budget before re-entering.
    const evicted = engine.pruneCache("another-session");
    if (evicted.includes(sample.session)) throw new Error("default navigation budget evicted this sample; measure that path separately");
    cutoff = finalCutoff;
    visible = true;
    status.textContent = `Round ${round}/${rounds}: warm +200 real events`;
    await open("warm-plus-200", finalCutoff - 200, 200);
    const expectedDigest = await digest(engine.timelineFor(sample.session));
    status.textContent = `Round ${round}/${rounds}: warm with no new events`;
    await open("warm-empty", finalCutoff, 0);
    if (await digest(engine.timelineFor(sample.session)) !== expectedDigest) throw new Error("empty reopen changed rendered content");
    visible = false;
    const beforeInvalidateCalls = trace.length;
    engine.reconcile(sample.session, "resend", sample.run);
    if (trace.length !== beforeInvalidateCalls) throw new Error("hidden invalidation issued background reads");
    visible = true;
    status.textContent = `Round ${round}/${rounds}: explicitly invalidated cache`;
    await open("hidden-resend-invalidated", -1, finalCutoff + 1);
    if (await digest(engine.timelineFor(sample.session)) !== expectedDigest) throw new Error("warm tail and full recovery content differ");
    const visibleItems = engine.timelineFor(sample.session)!.items;
    engine.clear();
    engine = create();
    // Keep the same already-visible content, deliberately without a projector
    // or cursor. This models display-only cache, not a restorable snapshot.
    await new Promise<void>(resolve => {
      const unsubscribe = engine.subscribe(() => { unsubscribe(); resolve(); });
      engine.mutate(sample.session, () => ({ ...emptyTimeline(), items: visibleItems }));
    });
    status.textContent = `Round ${round}/${rounds}: visible content without resumable state`;
    await open("display-only-cache", -1, finalCutoff + 1);
    if (await digest(engine.timelineFor(sample.session)) !== expectedDigest) throw new Error("display-only recovery differs from warm result");
    console.info("SYNC_WARM_VERIFIED", JSON.stringify({ round, renderedContentEqual: true, defaultNavigationEvicted: false }));
  } finally { engine.clear(); }
}
button.addEventListener("click", async () => {
  button.disabled = true;
  try {
    const samples: Sample[] = await (await fetch("/samples")).json();
    const sample = samples.find(s => s.label === "largest")!;
    for (let round = 1; round <= rounds; round++) await runRound(sample, round);
    status.textContent = `COMPLETE: controlled cache playback verified; ${rounds * 5} measured synchronizations, no invented events.`;
  } catch (error) {
    status.textContent = `FAILED: ${String(error)}`;
    console.error("SYNC_WARM_FAILURE", String(error));
  } finally { button.disabled = false; }
});
