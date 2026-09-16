// Real historical event playback in an actual browser. No fake clock, no
// invented token content. This measures production TS, NOT native RN rendering,
// NATS transport, mobile RTT, or battery power. Raw content is never displayed.
import type { SyncEngine } from "../mobile/src/remote/syncEngine";
import type { StreamEvent } from "../mobile/src/remote/types";
import type { TimelineState } from "../mobile/src/remote/timeline";
import type { FutureMarkdownDocument } from "../packages/markdown/src/types";

type Runtime = {
  SyncEngine: typeof SyncEngine;
  emptyTimeline(): TimelineState;
  applyReplayEvents(state: TimelineState, events: StreamEvent[]): Promise<TimelineState>;
  createStreamingMarkdownParser(): (text: string, streaming: boolean) => FutureMarkdownDocument;
  parseFutureMarkdown(text: string): FutureMarkdownDocument;
};
declare const MobileBaseline: Runtime;
declare const MobileCurrent: Runtime;
const runtimes = { baseline: MobileBaseline, current: MobileCurrent };
const status = document.querySelector("#status")!;
const output = document.querySelector("#output")!;
const run = document.querySelector<HTMLButtonElement>("#run")!;
const results: unknown[] = [];
const wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
const roundMs = (n: number) => Math.round(n * 100) / 100;
const quantile = (values: number[], q: number) => {
  const sorted = [...values].sort((a, b) => a - b);
  return roundMs(sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))] ?? 0);
};
async function digest(value: unknown, timeline = false) {
  // Object key insertion order is not a semantic difference. Only the UI's
  // top-level wall-clock-derived timing fields are excluded from trace hashes.
  if (timeline && Array.isArray(value)) value = value.map(item => {
    const { startedAt: _start, durationMs: _duration, ...content } = item;
    return content;
  });
  const bytes = new TextEncoder().encode(JSON.stringify(value, (_key, value) =>
    value && typeof value === "object" && !Array.isArray(value)
      ? Object.fromEntries(Object.keys(value).sort().map(key => [key, value[key]])) : value));
  const hash = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(hash)].map(n => n.toString(16).padStart(2, "0")).join("");
}
async function save(value: unknown) {
  results.push(value);
  output.textContent = JSON.stringify(results, null, 2);
  const response = await fetch("/results", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({
    environment: { userAgent: navigator.userAgent, hardwareConcurrency: navigator.hardwareConcurrency },
    scope: "real historical trace, controlled playback; production TS only; not native UI or battery measurement",
    results,
  }) });
  if (!response.ok) throw new Error(`saving metrics failed: ${response.status}`);
}
document.querySelector("#back")!.addEventListener("click", () => {
  status.textContent = "Local return handler executed (this is not a native navigation test).";
});

async function playback(events: StreamEvent[], version: keyof typeof runtimes, sample: number, round: number, paced: boolean) {
  const api = runtimes[version];
  const end = events.length - 1;
  const start = Math.max(2, events.length - (paced ? 201 : 1001));
  let cutoff = start - 1;
  let measuring = false;
  let commits = 0;
  let finished!: () => void;
  let failed!: (error: unknown) => void;
  let opened!: () => void;
  const complete = new Promise<void>((resolve, reject) => { finished = resolve; failed = reject; });
  const ready = new Promise<void>(resolve => { opened = resolve; });
  let last: TimelineState | null = null;
  const engine = new api.SyncEngine({
    requestGetState: async () => ({ activeRun: cutoff < end ? { runId: "trace-run" } : undefined }),
    requestHistory: async () => api.emptyTimeline(),
    fetchReplay: async (_session, _run, since) => ({
      events: events.slice(since + 1, cutoff + 1), watermark: cutoff,
    }),
    onSyncStatus: (_session, value) => { if (!measuring && value === "idle") opened(); },
    onFailure: value => failed(value.error),
  });
  engine.subscribe(commit => {
    last = commit.timeline;
    if (!measuring) return;
    commits++;
    if (commit.cursor.get("trace-run")?.highWater === end) finished();
  });
  const timer = setTimeout(() => failed(new Error("playback exceeded 180s")), 180000);
  let heartbeat: ReturnType<typeof setInterval> | undefined;
  try {
    await engine.open("trace");
    await Promise.race([ready, complete]);
    const lags: number[] = [];
    let expected = performance.now() + 16;
    heartbeat = setInterval(() => {
      const now = performance.now();
      lags.push(Math.max(0, now - expected));
      expected = now + 16;
    }, 16);
    cutoff = end;
    measuring = true;
    const begin = performance.now();
    for (let idx = start; idx <= end; idx++) {
      engine.event("trace", events[idx]!);
      if (paced) await wait(10); // Controlled ~100 events/s, NOT original timestamps.
    }
    await complete;
    const elapsedMs = roundMs(performance.now() - begin);
    await wait(20); // Observe the heartbeat delayed by the last batch.
    return { sample, round, version, scenario: paced ? "paced-real-tail-10ms" : "burst-real-tail",
      prefixEvents: start, liveEvents: end - start + 1, elapsedMs, commits,
      heartbeatP95Ms: quantile(lags, .95), heartbeatMaxMs: quantile(lags, 1),
      highWater: engine.cursorFor("trace").get("trace-run")?.highWater,
      digest: await digest(last?.items, true) };
  } finally {
    clearTimeout(timer);
    clearInterval(heartbeat);
    engine.clear();
  }
}

async function markdown(text: string, version: keyof typeof runtimes, sample: number, round: number) {
  const api = runtimes[version];
  const parse = api.createStreamingMarkdownParser();
  const costs: number[] = [];
  let final: FutureMarkdownDocument | undefined;
  // Replay the actual reply as 100 growing frames, no duplicated paragraphs.
  const stride = Math.max(1, Math.ceil(text.length / 100));
  for (let end = stride; end < text.length + stride; end += stride) {
    const begin = performance.now();
    final = parse(text.slice(0, end), true);
    costs.push(performance.now() - begin);
    await wait(0);
  }
  const expected = api.parseFutureMarkdown(text);
  const hash = await digest(final);
  if (hash !== await digest(expected)) throw new Error("Markdown differs from canonical parser");
  return { sample, round, version, scenario: "real-reply-markdown", chars: text.length,
    hasBrackets: text.includes("["), frames: costs.length,
    totalParseMs: roundMs(costs.reduce((a, b) => a + b, 0)), p95ParseMs: quantile(costs, .95),
    maxParseMs: quantile(costs, 1), digest: hash };
}

document.querySelector<HTMLButtonElement>("#markdown")!.addEventListener("click", async event => {
  const button = event.currentTarget as HTMLButtonElement;
  button.disabled = true;
  run.disabled = true;
  try {
    const saved = await (await fetch("/results")).json();
    results.splice(0, results.length, ...saved.results);
    const documents: { index: number; text: string }[] = await (await fetch("/documents")).json();
    if (!documents.length) throw new Error("No real Markdown replies found");
    for (const doc of documents) {
      for (let round = 1; round <= 3; round++) {
        const versions: (keyof typeof runtimes)[] = round % 2 ? ["baseline", "current"] : ["current", "baseline"];
        for (const version of versions) {
          status.textContent = `Real Markdown ${doc.index}, ${doc.text.length} chars: round ${round}, ${version}`;
          await save({ ...await markdown(doc.text, version, doc.index, round), scenario: "real-document-markdown" });
        }
      }
    }
    status.textContent = "COMPLETE: real long Markdown replies match canonical parsing in both versions.";
  } catch (error) {
    status.textContent = `FAILED: ${String(error)}`;
    console.error("MOBILE_MARKDOWN_FAILURE", String(error));
  } finally { button.disabled = false; run.disabled = false; }
});

run.addEventListener("click", async () => {
  run.disabled = true;
  try {
    const samples: { index: number; events: number; payloadChars: number }[] = await (await fetch("/samples")).json();
    for (const sample of samples) {
      status.textContent = `Loading real trace ${sample.index}, ${sample.events} events`;
      const events: StreamEvent[] = await (await fetch(`/sample/${sample.index}`)).json();
      const canonical = await MobileBaseline.applyReplayEvents(MobileBaseline.emptyTimeline(), events);
      const expected = await digest(canonical.items, true);
      const replies = canonical.items.filter(item => item.kind === "message" && item.role === "assistant")
        .map(item => item.kind === "message" ? item.text : "");
      const text = replies.sort((a, b) => b.length - a.length)[0] ?? "";
      await save({ scenario: "corpus", ...sample, replyChars: text.length, contentDigest: expected,
        eventTypes: Object.fromEntries([...new Set(events.map(e => e.type))].map(type => [type, events.filter(e => e.type === type).length])) });
      for (let round = 1; round <= 3; round++) {
        // Alternate A/B order to reduce warmup/order bias.
        const versions: (keyof typeof runtimes)[] = round % 2 ? ["baseline", "current"] : ["current", "baseline"];
        for (const version of versions) {
          status.textContent = `Trace ${sample.index}: round ${round}/3, ${version}`;
          for (const paced of [false, true]) {
            const result = await playback(events, version, sample.index, round, paced);
            if (result.digest !== expected || result.highWater !== events.length - 1)
              throw new Error("Real trace content/cursor mismatch");
            await save(result);
          }
          if (text) await save(await markdown(text, version, sample.index, round));
          await wait(100);
        }
      }
    }
    status.textContent = "COMPLETE: all real-trace content/cursor and Markdown equivalence checks passed. Metrics are NOT phone power measurements.";
  } catch (error) {
    status.textContent = `FAILED: ${String(error)}`;
    console.error("MOBILE_PERF_FAILURE", String(error));
  } finally { run.disabled = false; }
});
