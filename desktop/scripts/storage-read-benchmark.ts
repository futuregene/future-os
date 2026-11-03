import { decodeJsonBytes } from "@future-os/thread-projection";

const output = document.getElementById("result")!;
let busy = false;
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
const percentile = (values: number[], percent: number) => [...values].sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(values.length * percent / 100))] ?? 0;

async function run(mode: "native" | "cooperative") {
  if (busy || document.visibilityState !== "visible") return;
  busy = true;
  const size = Number((document.getElementById("size") as HTMLSelectElement).value);
  const shape = (document.getElementById("shape") as HTMLSelectElement).value;
  const unit = "思考🙂 abc\n".repeat(8);
  const count = Math.ceil(size * 1024 * 1024 / new TextEncoder().encode(unit).length);
  const source = shape === "string" ? { body: unit.repeat(count) }
    : { entries: Array.from({ length: count }, (_, index) => ({ id: index, text: unit, opaque: { signature: "synthetic", extension: [null, true, index] } })) };
  const bytes = new TextEncoder().encode(JSON.stringify(source));
  const samples: unknown[] = [];
  const inputDelays: number[] = [];
  const typing = document.getElementById("typing")!;
  const onInput = (event: Event) => inputDelays.push(Math.max(0, performance.now() - event.timeStamp));
  typing.addEventListener("input", onInput);
  try {
    for (let round = 0; round < 5; round += 1) {
      output.textContent = `Running ${mode}, ${size} MiB, ${shape}, round ${round + 1}`;
      const frames: number[] = [];
      const tasks: number[] = [];
      let observing = true;
      let last = performance.now();
      const probe = (now: number) => {
        frames.push(now - last);
        last = now;
        if (observing) requestAnimationFrame(probe);
      };
      requestAnimationFrame(probe);
      const observer = typeof PerformanceObserver === "undefined" ? null : new PerformanceObserver(entries => {
        for (const entry of entries.getEntries()) tasks.push(entry.duration);
      });
      if (typeof PerformanceObserver !== "undefined" && PerformanceObserver.supportedEntryTypes?.includes("longtask")) observer?.observe({ type: "longtask" });
      await frame();
      await frame();
      frames.length = 0;
      const start = performance.now();
      let timerDelayMs = 0;
      const inputTurn = new Promise<void>(resolve => setTimeout(() => {
        timerDelayMs = performance.now() - start;
        resolve();
      }, 0));
      const value = mode === "native"
        ? JSON.parse(new TextDecoder().decode(bytes)) as typeof source
        : await decodeJsonBytes<typeof source>(bytes);
      const decodeMs = performance.now() - start;
      if (shape === "string" ? value.body !== source.body
        : value.entries?.length !== count || value.entries[0]?.text !== unit || value.entries[count - 1]?.id !== count - 1)
        throw new Error("Decoded data mismatch");
      await inputTurn;
      await frame();
      await frame();
      observing = false;
      observer?.disconnect();
      samples.push({ decodeMs, timerDelayMs, frameP95Ms: percentile(frames, 95), frameMaxMs: Math.max(0, ...frames), longTasks: tasks });
    }
    output.textContent = JSON.stringify({ mode, sizeMiB: size, shape, encodedBytes: bytes.length, rounds: samples.length,
      userAgent: navigator.userAgent, hardwareConcurrency: navigator.hardwareConcurrency, samples,
      manualInputSamples: inputDelays.length, manualInputDelayP95Ms: percentile(inputDelays, 95),
      scope: "Browser JSON decoding and event-loop probes; excludes dataset construction, native IPC, DB durability and Markdown rendering. No result is a mobile-device measurement.",
    }, null, 2);
  }
  catch (error) { output.textContent = String(error); }
  finally { typing.removeEventListener("input", onInput); busy = false; }
}
document.getElementById("native")!.addEventListener("click", () => { void run("native"); });
document.getElementById("cooperative")!.addEventListener("click", () => { void run("cooperative"); });
