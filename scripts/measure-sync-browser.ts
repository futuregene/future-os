// Browser entry: import production Mobile synchronization/projection code.
// Only transport is adapted to the isolated DesktopHost's loopback HTTP probe.
import { SyncEngine } from "../mobile/src/remote/syncEngine";
import { fetchEventsSince } from "../mobile/src/remote/replay";
import { requestReadPage } from "../mobile/src/remote/readPages";
import { timelineFromEntries } from "../mobile/src/remote/timeline";
import type { RemoteClient } from "../mobile/src/remote/client";
import type { EntriesData, RemoteCommand, RemoteSessionState } from "../mobile/src/remote/types";

type Sample = { label: string; session: string; run: string; expected_events: number };
const output = document.querySelector("pre")!;
const status = document.querySelector("#status")!;
const button = document.querySelector("button")!;
const results: unknown[] = [];

async function measure(sample: Sample, round: number) {
  const trace: { type: string; phase: string; bytes: number; elapsedMs: number; backendMs: number; parseMs: number }[] = [];
  let phase = "state";
  const client = {
    async requestRetry<T>(command: RemoteCommand) {
      const started = performance.now();
      const response = await fetch("/rpc", { method: "POST", headers: { "Content-Type": "application/json", "X-Sync-Measurement": "1" }, body: JSON.stringify(command) });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const bytes = await response.arrayBuffer();
      const parseStarted = performance.now();
      const parsed = JSON.parse(new TextDecoder().decode(bytes));
      const parseMs = performance.now() - parseStarted;
      trace.push({ type: command.type, phase, bytes: bytes.byteLength, elapsedMs: performance.now() - started, backendMs: Number(response.headers.get("X-Backend-Ms")), parseMs });
      if (!parsed.success) throw new Error(parsed.error ?? "RPC failed");
      return parsed as { success: boolean; data: T };
    },
  } as unknown as RemoteClient;
  let finish!: () => void;
  let fail!: (error: unknown) => void;
  const done = new Promise<void>((resolve, reject) => { finish = resolve; fail = reject; });
  let timing: unknown;
  let firstCommitMs: number | null = null;
  let receivedEvents = 0;
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
      const started = performance.now();
      const result = await fetchEventsSince(client, session, run, since, current);
      fetchReplayMs += performance.now() - started;
      receivedEvents += result.events?.length ?? 0;
      return { ...result, events: result.events ?? [] };
    },
    onTiming: value => {
      timing = { reason: value.reason, attempt: value.attempt, elapsedMs: value.elapsedMs,
        stagesMs: value.stagesMs, outcome: value.outcome };
    },
    onSyncStatus: (_session, value) => { if (value === "idle") finish(); },
    onFailure: failure => { fail(failure.error); },
  });
  engine.subscribe(() => { firstCommitMs ??= performance.now() - start; });
  const timeout = setTimeout(() => fail(new Error("measurement exceeded 120 seconds")), 120000);
  try {
    // Historical runs are completed, not currently streaming. Explicitly
    // select their real replay target through the engine's public reconcile
    // API; do not fabricate activeRun/get_state responses or token events.
    engine.reconcile(sample.session, "open", sample.run);
    await done;
    const syncMs = performance.now() - start;
    const highWater = engine.cursorFor(sample.session).get(sample.run)?.highWater;
    if (receivedEvents !== sample.expected_events || highWater !== receivedEvents - 1)
      throw new Error(`replay integrity mismatch: events=${receivedEvents} expected=${sample.expected_events} cursor=${highWater}`);
    const summarize = (name: string) => {
      const entries = trace.filter(row => row.phase === name);
      return { requests: entries.length, pages: entries.filter(row => row.type !== "get_read_chunk").length,
        chunks: entries.filter(row => row.type === "get_read_chunk").length,
        replyJsonBytes: entries.reduce((sum, row) => sum + row.bytes, 0),
        maxReplyBytes: Math.max(0, ...entries.map(row => row.bytes)),
        backendMs: entries.reduce((sum, row) => sum + row.backendMs, 0),
        parseMs: entries.reduce((sum, row) => sum + row.parseMs, 0),
        firstRequestMs: entries[0]?.elapsedMs ?? 0 };
    };
    return { sample: sample.label, round, receivedEvents, highWater, syncMs, firstCommitMs, fetchReplayMs,
      timelineItems: engine.timelineFor(sample.session)?.items.length, timing,
      state: summarize("state"), history: summarize("history"), replay: summarize("replay") };
  } finally {
    clearTimeout(timeout);
    engine.clear();
  }
}
button.addEventListener("click", async () => {
  button.disabled = true;
  try {
    const samples: Sample[] = await (await fetch("/samples")).json();
    for (let round = 1; round <= 3; round++) {
      for (const sample of samples) {
        status.textContent = `Measuring ${sample.label}, round ${round}/3`;
        const result = await measure(sample, round);
        results.push(result);
        output.textContent = JSON.stringify(results, null, 2);
        console.info("SYNC_BROWSER_RESULT", JSON.stringify(result));
      }
    }
    status.textContent = "COMPLETE: actual browser/loopback measurements; NOT mobile network or native UI timings.";
  } catch (error) {
    status.textContent = `FAILED: ${String(error)}`;
    console.error("SYNC_BROWSER_FAILURE", String(error));
  } finally {
    button.disabled = false;
  }
});
