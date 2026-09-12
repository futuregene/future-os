import { Profiler } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import "../src/i18n";
import { MarkdownContent, StreamingMarkdownContent } from "../src/features/markdown/MarkdownContent";

// Run with the desktop Vite dev server, then open
// /scripts/streaming-markdown-benchmark.html. No Agent or Tauri IPC is used.
const host = document.getElementById("render-root")!;
const result = document.getElementById("result")!;
const root = createRoot(host);
const rounds = 5;
let busy = false;
const measuredBaselines = new Set<number>();

async function nextFrame() {
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
}

function waitForRows(count: number) {
  return new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => { observer.disconnect(); reject(new Error("Worker/render timeout")); }, 30000);
    const check = () => {
      if (host.querySelectorAll("tbody tr").length !== count)
        return;
      clearTimeout(timer);
      observer.disconnect();
      resolve();
    };
    const observer = new MutationObserver(check);
    observer.observe(host, { childList: true, subtree: true });
    check();
  });
}

async function run(mode: "baseline" | "worker") {
  if (busy)
    return;
  busy = true;
  const rows = Number((document.getElementById("rows") as HTMLInputElement).value);
  if (!Number.isInteger(rows) || rows < 1 || rows > 3000) {
    result.textContent = "Choose 1–3000 rows";
    busy = false;
    return;
  }
  if (document.visibilityState !== "visible" || (mode === "baseline" && measuredBaselines.has(rows))) {
    result.textContent = "Bring the page to the foreground; reload before repeating a baseline to avoid parse-cache hits.";
    busy = false;
    return;
  }
  if (mode === "baseline")
    measuredBaselines.add(rows);
  const Renderer = mode === "baseline" ? MarkdownContent : StreamingMarkdownContent;
  flushSync(() => root.render(null));
  const base = "| Field | Value | Note |\n| --- | --- | --- |\n"
    + Array.from({ length: rows }, (_, i) => `| item ${i} | **result** | Some detailed explanation |`).join("\n");
  let source = base;
  const commits: number[] = [];
  const samples: Array<{ pushMs: number; renderMs: number; formatReadyMs: number; maxTimerDelayMs: number }> = [];
  const render = () => root.render(
    <Profiler id="markdown" onRender={(_id, _phase, duration) => commits.push(duration)}>
      <Renderer content={source} live />
    </Profiler>,
  );
  try {
    result.textContent = `Running ${mode}, ${rows} rows`;
    flushSync(render);
    await waitForRows(rows);
    await nextFrame();
    for (let step = 0; step < rounds; step++) {
      commits.length = 0;
      let maxTimerDelayMs = 0;
      let last = performance.now();
      const heartbeat = setInterval(() => {
        const now = performance.now();
        maxTimerDelayMs = Math.max(maxTimerDelayMs, now - last - 10);
        last = now;
      }, 10);
      source += `\n| extra ${step} | 1 | more |`;
      const start = performance.now();
      flushSync(render);
      const pushMs = performance.now() - start;
      await waitForRows(rows + step + 1);
      await nextFrame();
      // Let the heartbeat observe the commit/layout long task as well.
      await new Promise(resolve => setTimeout(resolve, 20));
      clearInterval(heartbeat);
      samples.push({
        pushMs,
        renderMs: commits.reduce((a, b) => a + b, 0),
        formatReadyMs: performance.now() - start - 20,
        maxTimerDelayMs,
      });
    }
    const median = (key: keyof typeof samples[number]) => {
      const sorted = samples.map(sample => sample[key]).sort((a, b) => a - b);
      return Math.round(sorted[Math.floor(sorted.length / 2)]! * 10) / 10;
    };
    result.textContent = JSON.stringify({
      mode, rows, chars: base.length, rounds, userAgent: navigator.userAgent,
      median: {
        pushMs: median("pushMs"), renderMs: median("renderMs"),
        formatReadyMs: median("formatReadyMs"), maxTimerDelayMs: median("maxTimerDelayMs"),
      },
      samples,
    }, null, 2);
    console.log("streaming-markdown-benchmark", result.textContent);
  }
  catch (error) {
    result.textContent = String(error);
  }
  finally {
    busy = false;
  }
}

document.getElementById("baseline")!.addEventListener("click", () => { void run("baseline"); });
document.getElementById("worker")!.addEventListener("click", () => { void run("worker"); });
