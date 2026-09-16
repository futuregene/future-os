// Same real snapshot, same binaries and browser: compare legacy raw replay
// against opt-in semantic bootstrap. No fabricated activeRun, clock or RTT.
import { SyncEngine, type SyncTiming } from "../mobile/src/remote/syncEngine";
import { fetchEventsSince } from "../mobile/src/remote/replay";
import { requestReadPage } from "../mobile/src/remote/readPages";
import { timelineFromEntries } from "../mobile/src/remote/timeline";
import type { RemoteClient } from "../mobile/src/remote/client";
import type { EntriesData, RemoteCommand, RemoteSessionState } from "../mobile/src/remote/types";

type Sample = { label: string; session: string; run: string; expected_events: number };
type Mode = "raw-baseline" | "snapshot";
const output = document.querySelector("pre")!;
const status = document.querySelector("#status")!;
const button = document.querySelector("button")!;
const results: unknown[] = [];
const digests = new Map<string, string>();
document.querySelector("aside")!.textContent = "A/B: real completed historical runs, production Mobile SyncEngine/replay/projector and real DesktopHost → isolated Agent. Only transport is loopback HTTP instead of NATS/E2EE. No fake activeRun, clock or RTT. Both modes use the same binaries; raw baseline disables only preferSnapshot. This is NOT native phone UI or mobile network timing.";
button.textContent = "Compare raw replay vs snapshots (3 rounds)";

async function measure(sample: Sample, round: number, mode: Mode) {
  const trace: { type: string; phase: string; bytes: number; backendMs: number; elapsedMs: number }[] = [];
  let phase = "state";
  const client = {
    async requestRetry<T>(command: RemoteCommand) {
      const start = performance.now();
      const actual = mode === "raw-baseline" ? { ...command, preferSnapshot: false } : command;
      const response = await fetch("/rpc", { method: "POST", headers: { "Content-Type": "application/json", "X-Sync-Measurement": "1" }, body: JSON.stringify(actual) });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const bytes = await response.arrayBuffer();
      const parsed = JSON.parse(new TextDecoder().decode(bytes));
      trace.push({ type: command.type, phase, bytes: bytes.byteLength,
        backendMs: Number(response.headers.get("X-Backend-Ms")), elapsedMs: performance.now() - start });
      if (!parsed.success) throw new Error(parsed.error ?? "RPC failed");
      return parsed as { success: boolean; data: T };
    },
  } as unknown as RemoteClient;
  let finish!: () => void;
  let fail!: (error: unknown) => void;
  const done = new Promise<void>((resolve, reject) => { finish = resolve; fail = reject; });
  let timing: Omit<SyncTiming, "sessionId" | "runId"> | undefined;
  let firstCommitMs: number | null = null;
  let projectedEvents = 0;
  let rawEvents = 0;
  let fetchReplayMs = 0;
  const start = performance.now();
  const engine = new SyncEngine({
    requestGetState: async sessionId => {
      phase = "state";
      return (await client.requestRetry<RemoteSessionState>({ type: "get_state", sessionId })).data;
    },
    requestHistory: async (sessionId, current) => {
      phase = "history";
      const result = await requestReadPage<EntriesData>(client, { type: "get_session_entries", sessionId, before: Number.MAX_SAFE_INTEGER, limit: 3 }, sessionId, current);
      return timelineFromEntries(result.data.entries ?? []);
    },
    fetchReplay: async (session, run, since, current) => {
      phase = "replay";
      const begin = performance.now();
      const result = await fetchEventsSince(client, session, run, since, current);
      fetchReplayMs += performance.now() - begin;
      projectedEvents += result.projection?.events?.length ?? 0;
      rawEvents += result.events?.length ?? 0;
      return { ...result, events: result.events ?? [] };
    },
    onTiming: value => { timing = { reason: value.reason, attempt: value.attempt, elapsedMs: value.elapsedMs,
      stagesMs: value.stagesMs, replayPlan: value.replayPlan, outcome: value.outcome }; },
    onSyncStatus: (_session, value) => { if (value === "idle") finish(); },
    onFailure: value => fail(value.error),
  });
  engine.subscribe(() => { firstCommitMs ??= performance.now() - start; });
  const timeout = setTimeout(() => fail(new Error("measurement exceeded 120 seconds")), 120000);
  try {
    // Completed runs are explicitly selected through a production API; their
    // actual get_state is not altered to pretend they are currently live.
    engine.reconcile(sample.session, "open", sample.run);
    await done;
    const syncMs = performance.now() - start;
    const cursor = engine.cursorFor(sample.session).get(sample.run);
    if (cursor?.highWater !== sample.expected_events - 1 || !cursor.prefixComplete)
      throw new Error("restored source cursor mismatch");
    if (mode === "raw-baseline" && rawEvents !== sample.expected_events) throw new Error("raw baseline event count mismatch");
    if (mode === "snapshot" && (projectedEvents === 0 || rawEvents !== 0)) throw new Error("snapshot optimization was not used");
    const items = engine.timelineFor(sample.session)!.items;
    const hash = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(JSON.stringify(items)))))
      .map(n => n.toString(16).padStart(2, "0")).join("");
    if (digests.has(sample.label) && digests.get(sample.label) !== hash) throw new Error(`raw/snapshot display mismatch: ${sample.label}`);
    digests.set(sample.label, hash);
    const summarize = (name: string) => {
      const rows = trace.filter(t => t.phase === name);
      return { requests: rows.length, pages: rows.filter(t => t.type !== "get_read_chunk").length,
        chunks: rows.filter(t => t.type === "get_read_chunk").length,
        replyBytes: rows.reduce((sum,t) => sum+t.bytes,0), backendMs: rows.reduce((sum,t) => sum+t.backendMs,0),
        maxReplyBytes: Math.max(0,...rows.map(t=>t.bytes)), firstRequestMs: rows[0]?.elapsedMs ?? 0 };
    };
    return { sample: sample.label, round, mode, sourceEvents: sample.expected_events, highWater: cursor.highWater,
      rawEvents, projectedEvents, syncMs, firstCommitMs, fetchReplayMs, timing,
      state: summarize("state"), history: summarize("history"), replay: summarize("replay"),
      sameDisplay: true, timelineItems: items.length };
  } finally { clearTimeout(timeout); engine.clear(); }
}
button.addEventListener("click", async () => {
  button.disabled = true;
  try {
    const samples: Sample[] = await (await fetch("/samples")).json();
    for (let round = 1; round <= 3; round++) {
      for (const sample of samples) {
        const modes: Mode[] = round === 2 ? ["snapshot", "raw-baseline"] : ["raw-baseline", "snapshot"];
        for (const mode of modes) {
          status.textContent = `Round ${round}/3: ${sample.label}, ${mode}`;
          const result = await measure(sample, round, mode);
          results.push(result);
          output.textContent = JSON.stringify(results, null, 2);
          console.info("SYNC_SNAPSHOT_AB", JSON.stringify(result));
        }
      }
    }
    status.textContent = "COMPLETE: 18 real A/B synchronizations; cursors and rendered data agree.";
  } catch (error) {
    status.textContent = `FAILED: ${String(error)}`;
    console.error("SYNC_SNAPSHOT_AB_FAILURE", String(error));
  } finally { button.disabled = false; }
});
