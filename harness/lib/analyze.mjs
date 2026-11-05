/**
 * Analysis: turn a run's artifacts into findings.
 *
 * Three layers, cheapest first:
 *  1. deterministic signals — failed checks, exceptions, failed requests,
 *     IPC errors, performance deltas. These are facts, not opinions.
 *  2. vision — every key screenshot is read by a vision model against the
 *     case's own expectation text. This is the part that notices "the dialog is
 *     empty" or "the text is cut off", which no assertion encoded.
 *  3. synthesis — a text model reads the case timeline, the deterministic
 *     signals and the vision notes, and proposes findings in a fixed JSON shape.
 *
 * Any layer can be skipped (`--mode heuristic`) or fail without taking the run
 * down: a missing CLI must not destroy the evidence the run already produced.
 */
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { execFile } from "node:child_process";
import { discoverAcrossCases } from "./discovery.mjs";
import { fixtureProvenance, markFixtureFindings } from "./provenance.mjs";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const REPO_ROOT = resolve(import.meta.dirname, "..", "..");

export function futureBin() {
  const local = join(REPO_ROOT, "target", "debug", "future");
  if (process.env.FUTURE_BIN)
    return process.env.FUTURE_BIN;
  return existsSync(local) ? local : "future";
}

/** The model the analysis uses when `--model` is not given. */
export function defaultAnalyzerModel() {
  const settingsPath = join(REPO_ROOT, "harness", ".data", "cli", "settings.json");
  try {
    return JSON.parse(readFileSync(settingsPath, "utf8")).defaultModel ?? null;
  }
  catch {
    return null;
  }
}

/**
 * Whether a model accepts image input, from the snapshot of the agent's model
 * registry. A model id may be provider-qualified (`provider/id`); the registry
 * is matched by the bare id when the provider differs, which is what the agent
 * itself does when resolving a bare id.
 */
export function supportsVision(modelId) {
  if (!modelId)
    return false;
  const modelsPath = join(REPO_ROOT, "harness", ".data", "models.json");
  try {
    const providers = JSON.parse(readFileSync(modelsPath, "utf8")).providers ?? {};
    const bare = String(modelId).split("/").pop();
    for (const [provider, config] of Object.entries(providers)) {
      for (const model of config.models ?? []) {
        if (`${provider}/${model.id}` !== modelId && model.id !== bare)
          continue;
        return Array.isArray(model.modalities) && model.modalities.includes("image");
      }
    }
  }
  catch {
    // No snapshot: fall back to the image tool.
  }
  return false;
}

function cleanOutput(text) {
  return String(text)
    .split("\n")
    .filter(line => !line.startsWith("[dns]"))
    .join("\n")
    .trim();
}

async function runCommand(command, args, { timeout = 240_000 } = {}) {
  const { stdout, stderr } = await execFileAsync(command, args, {
    timeout,
    maxBuffer: 20 * 1024 * 1024,
    env: process.env,
  });
  return { stdout: cleanOutput(stdout), stderr: cleanOutput(stderr) };
}

export async function readImage(input, question, { timeout = 300_000 } = {}) {
  const { stdout } = await runCommand(futureBin(), [
    "tools", "call", "read_image",
    "--input", input,
    "--question", question,
    "--timeout", String(Math.round(timeout / 1000)),
  ], { timeout: timeout + 30_000 });
  return stdout;
}

/** `future run` with the prompt in a file (no argv length limits, no quoting). */
export async function runPrompt(promptPath, { model = null, timeout = 300_000, images = [] } = {}) {
  const args = ["run", "--no-session", "--no-tools", "--no-title", "--thinking", "off", "--mode", "json"];
  if (model)
    args.push("--model", model);
  for (const image of images)
    args.push("--image", image);
  args.push("--message-file", promptPath);
  const { stdout } = await runCommand(futureBin(), args, { timeout });
  let payload = null;
  try {
    payload = JSON.parse(stdout);
  }
  catch {
    const start = stdout.indexOf("{");
    const end = stdout.lastIndexOf("}");
    if (start >= 0 && end > start) {
      try {
        payload = JSON.parse(stdout.slice(start, end + 1));
      }
      catch {}
    }
  }
  const text = payload?.text ?? payload?.content ?? payload?.message ?? stdout;
  return { text: String(text ?? ""), payload };
}

function severityRank(value) {
  return { critical: 0, high: 1, medium: 2, low: 3, info: 4 }[value] ?? 5;
}

function truncate(text, limit = 800) {
  const value = String(text ?? "");
  return value.length > limit ? `${value.slice(0, limit)}…` : value;
}

/** Deterministic signals. No model, no network — these would hold in a court. */
export function deterministicFindings(result, { runDir }) {
  const findings = [];
  const evidence = result.artifacts?.dir ?? null;

  for (const check of result.checks ?? []) {
    if (check.ok)
      continue;
    findings.push({
      id: `${result.id}:check:${check.step}`,
      severity: "high",
      category: "behaviour",
      title: check.message,
      detail: [
        `用例「${result.title}」第 ${check.step} 步的检查未通过。`,
        check.expected !== undefined ? `期望：${truncate(JSON.stringify(check.expected))}` : null,
        check.actual !== undefined ? `实际：${truncate(JSON.stringify(check.actual))}` : null,
      ].filter(Boolean).join("\n"),
      evidence: { caseId: result.id, step: check.step, dir: evidence },
      source: "check",
      confidence: "high",
    });
  }

  if (result.status === "error" && result.fatal)
    findings.push({
      id: `${result.id}:fatal`,
      severity: "high",
      category: "harness-or-crash",
      title: `用例执行中断：${truncate(result.fatal, 120)}`,
      detail: result.fatal,
      evidence: { caseId: result.id, dir: evidence },
      source: "run",
      confidence: "medium",
    });

  const seenExceptions = new Set();
  for (const exception of result.exceptions ?? []) {
    const text = String(exception.text ?? "");
    const key = text.split("\n")[0];
    if (seenExceptions.has(key))
      continue;
    seenExceptions.add(key);
    findings.push({
      id: `${result.id}:exception:${key.slice(0, 40)}`,
      severity: "high",
      category: "crash",
      title: `未捕获异常：${truncate(key, 120)}`,
      detail: truncate(text, 2000),
      evidence: { caseId: result.id, url: exception.url, line: exception.line, dir: evidence },
      source: "exception",
      confidence: "high",
    });
  }

  const noise = /favicon|Download the React DevTools|\[vite\]|WebSocket connection/i;
  const seenConsole = new Set();
  for (const message of result.console?.errors ?? []) {
    const text = String(message.text ?? "");
    if (noise.test(text) || seenConsole.has(text))
      continue;
    seenConsole.add(text);
    findings.push({
      id: `${result.id}:console:${text.slice(0, 40)}`,
      severity: "medium",
      category: "console",
      title: `控制台错误：${truncate(text, 120)}`,
      detail: text,
      evidence: { caseId: result.id, dir: evidence },
      source: "console",
      confidence: "medium",
    });
  }

  for (const failure of result.network?.failures ?? []) {
    if (/favicon/.test(failure.url ?? ""))
      continue;
    // Demo-asset URLs from the shot fixtures (figures served by the screenshot
    // harness's little asset server) are not product requests.
    if (/(effect-size|forest-plot|parser-throughput)\.png/.test(failure.url ?? ""))
      continue;
    // Requests after the last step are the harness tearing the page down, not
    // the product failing.
    if (result.stepsEndedAt && failure.start > Date.parse(result.stepsEndedAt))
      continue;
    findings.push({
      id: `${result.id}:network:${failure.url}`,
      severity: "high",
      category: "network",
      title: `请求失败：${truncate(failure.url, 100)}`,
      detail: `${failure.method ?? "GET"} ${failure.url} → ${failure.error ?? failure.status}`,
      evidence: { caseId: result.id, dir: evidence },
      source: "network",
      confidence: "high",
    });
  }

  for (const entry of result.server?.errors ?? []) {
    findings.push({
      id: `${result.id}:ipc:${entry.command}`,
      severity: "high",
      category: "backend",
      title: `IPC ${entry.command} 报错：${truncate(entry.error, 100)}`,
      detail: `参数：${truncate(JSON.stringify(entry.args), 500)}\n错误：${entry.error}`,
      evidence: { caseId: result.id, dir: evidence },
      source: "ipc",
      confidence: "high",
    });
  }

  // Performance is a channel screenshots cannot show. Thresholds are coarse on
  // purpose: this flags "something is wrong", the synthesis decides if it
  // matters.
  const baseline = (result.metrics ?? []).find(m => m.at === "baseline");
  const final = (result.metrics ?? []).find(m => m.at === "final");
  if (baseline && final) {
    const heapDelta = final.jsHeapUsed - baseline.jsHeapUsed;
    const taskDelta = final.taskDuration - baseline.taskDuration;
    if (heapDelta > 180 * 1024 * 1024)
      findings.push({
        id: `${result.id}:perf:heap`,
        severity: "medium",
        category: "performance",
        title: `JS 堆增长 ${(heapDelta / 1024 / 1024).toFixed(0)} MB`,
        detail: `用例运行时堆从 ${(baseline.jsHeapUsed / 1024 / 1024).toFixed(0)} MB 增到 ${(final.jsHeapUsed / 1024 / 1024).toFixed(0)} MB；DOM 节点 ${baseline.nodes} → ${final.nodes}。`,
        evidence: { caseId: result.id, dir: evidence },
        source: "perf",
        confidence: "medium",
      });
    if (taskDelta > 6)
      findings.push({
        id: `${result.id}:perf:tasks`,
        severity: "low",
        category: "performance",
        title: `主线程累计忙碌 ${taskDelta.toFixed(1)}s`,
        detail: `用例墙钟 ${(result.durationMs / 1000).toFixed(1)}s，其中主线程任务 ${taskDelta.toFixed(1)}s；脚本 ${(final.scriptDuration - baseline.scriptDuration).toFixed(1)}s。`,
        evidence: { caseId: result.id, dir: evidence },
        source: "perf",
        confidence: "low",
      });
  }

  const slowSteps = (result.steps ?? []).filter(step => step.ok && step.ms > 4000 && !["wait", "waitFor", "sleep"].includes(step.action));
  for (const step of slowSteps) {
    findings.push({
      id: `${result.id}:slow:${step.index}`,
      severity: "low",
      category: "performance",
      title: `第 ${step.index} 步「${step.action}」耗时 ${(step.ms / 1000).toFixed(1)}s`,
      detail: `目标：${truncate(JSON.stringify(step.target), 300)}`,
      evidence: { caseId: result.id, step: step.index, dir: evidence },
      source: "perf",
      confidence: "low",
    });
  }

  return findings;
}

function visionTargets(result) {
  const targets = [];
  // Failure frames first: they are the only picture of the state the assertion
  // rejected, and a 3-image budget otherwise drops them.
  for (const step of result.steps ?? []) {
    if (step.failureShot)
      targets.push({ path: step.failureShot, step: step.index, note: `失败现场：${step.error ?? ""}` });
  }
  for (const step of result.steps ?? []) {
    if (!step.shot)
      continue;
    if (step.action === "shot" || step.action === "assert" || step.action === "db")
      targets.push({ path: step.shot, step: step.index, note: step.note ?? step.action });
  }
  const seen = new Set();
  return targets.filter((target) => {
    if (seen.has(target.path))
      return false;
    seen.add(target.path);
    return true;
  }).slice(0, 4);
}

async function filmstrip(videoPath, outPath, { frames = 12 } = {}) {
  if (!existsSync(videoPath))
    return null;
  if (existsSync(outPath))
    return outPath;
  try {
    const { stdout } = await execFileAsync("ffprobe", [
      "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", videoPath,
    ], { timeout: 30_000 });
    const duration = Number(String(stdout).trim());
    // Skip the first and last 1.5 s: the page's first paint and the harness
    // teardown are not product states, and a black tail frame reads as "the
    // screen went blank".
    const seek = duration > 4 ? 0.8 : 0;
    const usable = duration > 4 ? Math.max(2, duration - seek - 1.2) : duration;
    const columns = frames >= 12 ? 4 : 3;
    const rows = Math.ceil(frames / columns);
    const fps = Math.max(0.2, frames / Math.max(1, usable));
    await execFileAsync("ffmpeg", [
      "-y", "-loglevel", "error",
      "-ss", String(seek),
      "-t", String(usable),
      "-i", videoPath,
      "-vf", `fps=${fps.toFixed(3)},scale=320:-1,tile=${columns}x${rows}:padding=4:margin=4`,
      "-frames:v", "1",
      outPath,
    ], { timeout: 120_000 });
    return existsSync(outPath) ? outPath : null;
  }
  catch {
    return null;
  }
}

function visionQuestion(result, target) {
  const intro = [
    `你在对 FutureOS 桌面端做端到端验收测试。当前用例：${result.title}。`,
    `用例说明：${result.description ?? ""}`,
    `期望结果：${(result.expectations ?? []).map((item, index) => `${index + 1}. ${item}`).join(" ")}`,
  ];
  if (target.filmstrip) {
    return [
      ...intro,
      "这是一段操作录像里按时间均匀抽取的 6 帧拼图（从左到右、从上到下）。首尾可能包含页面加载或收尾帧；单帧空白不构成缺陷，只有多帧持续异常或与操作不符才值得报告。",
      "只依据这些帧回答：",
      "1. 从第一帧到最后一帧，界面状态发生了哪些变化？",
      "2. 有没有某一帧出现空白、错位、弹层残留、内容闪烁或状态与操作不匹配？",
      "3. 与期望不一致的地方是什么？没有就写「无」。",
      "用中文、分条作答，控制在 400 字内。",
    ].join("\n");
  }
  return [
    ...intro,
    `这一步是：${target.note}（第 ${target.step} 步）。`,
    "只依据这张截图回答，不要猜测看不见的内容：",
    "1. 画面上实际能看到什么？点出主区域、侧边栏、底部输入框的关键文案与状态。",
    "2. 与期望不一致的地方是什么？没有就写「无」。",
    "3. 有没有空白页、遮挡、错位、溢出、文案错误、状态残留？",
    "用中文、分条作答，控制在 400 字内。",
  ].join("\n");
}

async function shrinkImage(input, output) {
  try {
    await execFileAsync("sips", ["-Z", "1400", input, "--out", output], { timeout: 30_000 });
    return existsSync(output) ? output : input;
  }
  catch {
    try {
      await execFileAsync("ffmpeg", ["-y", "-loglevel", "error", "-i", input, "-vf", "scale='min(1400,iw)':-2", output], { timeout: 60_000 });
      return existsSync(output) ? output : input;
    }
    catch {
      return input;
    }
  }
}

async function mapPool(items, size, worker) {
  const results = new Array(items.length);
  let next = 0;
  async function run() {
    while (next < items.length) {
      const index = next++;
      results[index] = await worker(items[index], index);
    }
  }
  await Promise.all(Array.from({ length: Math.min(size, items.length) }, run));
  return results;
}

async function visionFindings(result, runDir, { limit = 3, native = false, model = null } = {}) {
  const notes = [];
  const dir = join(runDir, "vision", result.id);
  mkdirSync(dir, { recursive: true });
  const targets = visionTargets(result).slice(0, limit);
  const videoPath = result.artifacts?.dir ? join(result.artifacts.dir, "video.mp4") : null;
  const strip = videoPath ? await filmstrip(videoPath, join(dir, "filmstrip.png")) : null;
  if (strip)
    targets.push({ path: strip, step: 0, note: "录像关键帧拼图", filmstrip: true });
  await mapPool(targets, 3, async (target) => {
    if (!target.path || !existsSync(target.path))
      return;
    try {
      const thumb = await shrinkImage(target.path, join(dir, `input-${String(target.step).padStart(2, "0")}.png`));
      const question = visionQuestion(result, target);
      let answer;
      let via;
      if (native) {
        const promptPath = join(dir, `prompt-${String(target.step).padStart(2, "0")}.md`);
        writeFileSync(promptPath, `${question}\n\n图片已作为附件随本消息提供。`);
        const { text } = await runPrompt(promptPath, { model, images: [thumb] });
        answer = text || "（模型没有返回文字）";
        via = "native";
      }
      else {
        answer = await readImage(thumb, question);
        via = "read_image";
      }
      const file = join(dir, `${String(target.step).padStart(2, "0")}-${target.path.split("/").pop().replace(/\.png$/, "")}.md`);
      writeFileSync(file, `# 第 ${target.step} 步 · ${target.note}\n\n![shot](${target.path})\n\n${answer}\n`);
      notes.push({ step: target.step, note: target.note, file, answer, via });
    }
    catch (error) {
      notes.push({ step: target.step, note: target.note, error: String(error?.message ?? error) });
    }
  });
  notes.sort((a, b) => a.step - b.step);
  return notes;
}

/**
 * Where a step sits in the finished clip, in seconds.
 *
 * The clip is not the wall clock — the muxer caps an idle frame at 5 s and its
 * frames are not always 25 fps — so this interpolates over the clock the muxer
 * recorded (`videoClock()` in video.mjs). Without it, "this frame is the state
 * after step 7" cannot be checked against the filmstrip, which is sampled in
 * video time.
 */
function videoSeconds(step, window) {
  const samples = window?.samples;
  if (!samples?.length || !Number.isFinite(step.ts))
    return null;
  const end = (step.ts ?? 0) + (step.ms ?? 0);
  let low = 0;
  let high = samples.length - 1;
  let pick = -1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (samples[mid][0] <= end) {
      pick = mid;
      low = mid + 1;
    }
    else {
      high = mid - 1;
    }
  }
  if (pick < 0)
    return null;
  const [ts, at] = samples[pick];
  const next = samples[pick + 1];
  if (!next || next[0] <= ts)
    return at;
  return at + (next[1] - at) * Math.min(1, Math.max(0, (end - ts) / (next[0] - ts)));
}

/** Ordered evidence for one case: actions first, then the whole recording. */
async function judgeEvidence(result, runDir, { limit = 8 } = {}) {
  const all = [];
  const window = result.artifacts?.videoWindow ?? null;
  for (const step of result.steps ?? []) {
    if (!step.shot && !step.failureShot)
      continue;
    const path = step.failureShot ?? step.shot;
    if (!path || all.some(item => item.path === path))
      continue;
    const at = videoSeconds(step, window);
    all.push({ path, label: `第 ${step.index} 步 · ${step.action}${at !== null ? ` · 录像 ${at.toFixed(1)}s` : ""}${step.note ? ` · ${step.note}` : ""}${step.ok === false ? "（失败现场）" : ""}` });
  }
  // Keep the final state, always.
  //
  // A case can take 20+ screenshots; slicing to the first N dropped the one that
  // shows how things ended, and the filmstrip alone was too coarse to read a
  // closing overlay from — the model reported "the picker is still covering the
  // list" on a case whose last screenshot showed it closed. Sampling now keeps
  // the first frame, evenly spaced middle frames, and the last one, labelled.
  let images = all;
  if (all.length > limit) {
    const picked = [all[0]];
    const middle = Math.max(1, limit - 2);
    for (let i = 1; i <= middle; i++)
      picked.push(all[Math.round((i * (all.length - 1)) / (middle + 1))]);
    picked.push(all.at(-1));
    images = picked.filter((item, index, list) => list.findIndex(other => other.path === item.path) === index);
    images[images.length - 1] = { ...images.at(-1), label: `最终状态 · ${images.at(-1).label}` };
  }
  else if (images.length) {
    images[images.length - 1] = { ...images.at(-1), label: `最终状态 · ${images.at(-1).label}` };
  }
  const dir = join(runDir, "vision", result.id);
  mkdirSync(dir, { recursive: true });
  const videoPath = result.artifacts?.dir ? join(result.artifacts.dir, "video.mp4") : null;
  const strip = videoPath ? await filmstrip(videoPath, join(dir, "judge-filmstrip.png"), { frames: 12 }) : null;
  // The filmstrip goes just before the final-state screenshot, so the image the
  // model reads last is unambiguously "how the run ended".
  if (strip) {
    const final = images.at(-1);
    const head = images.slice(0, -1);
    const span = window?.durationMs
      ? `约 0–${(window.durationMs / 1000).toFixed(1)}s，每帧约 ${((window.durationMs / 1000) / 12).toFixed(1)}s`
      : "按真实时间";
    const finalAt = final ? (() => {
      const match = String(final.label).match(/录像 ([\d.]+)s/);
      return match ? `，标「最终状态」的那张在 ${match[1]}s` : "";
    })() : "";
    images = [...head, { path: strip, label: `整段操作录像的 12 帧拼图（从左到右、从上到下，${span}${finalAt}；早于某个步骤的帧拍的是那一步之前的状态，读数以标「最终状态」的那张为准）` }, ...(final ? [final] : [])];
  }
  return images;
}

/**
 * The performance numbers, written the way a person reviewing a profile would
 * want them: the summary, then the series with its trend, then how close each
 * assertion came to its limit.
 *
 * The series is the part that makes a complexity or leak judgement possible at
 * all. A mean of 320ms says nothing; twenty values that climb 260, 280, 300, …
 * 900 say "something accumulates per iteration".
 */
function perfDigest(result) {
  const perf = result.perf;
  if (!perf)
    return "";
  const lines = [];
  const summary = [
    ["readyMs", perf.bootMs],
    ["invokeRequests", perf.invokeRequests],
    ["invokeBytes", perf.invokeBytes],
    ["networkRequests", perf.networkRequests],
    ["ipcP95", perf.ipcP95],
    ["ipcMax", perf.ipcMax],
    ["heapGrowthMb", perf.heapGrowthMb],
    ["rssGrowthMb", perf.rssGrowthMb],
    ["startupMs", perf.startupMs],
    ["maxMs", perf.maxMs],
    ["durationMs", perf.durationMs],
  ].filter(([, value]) => value !== null && value !== undefined);
  if (summary.length)
    lines.push(summary.map(([key, value]) => `${key}=${value}`).join("  "));
  for (const series of perf.series ?? []) {
    const shown = series.values.length > 12
      ? [...series.values.slice(0, 6), "…", ...series.values.slice(-6)]
      : series.values;
    lines.push(`${series.label}: n=${series.n} 每次(ms)=${shown.join(",")} 均值=${series.mean} 漂移=${series.drift} 斜率=${series.slope}ms/次`);
  }
  // Which assertion was closest to its limit is the most useful single fact for
  // a reviewer: everything else has slack, that one is where a regression lands.
  const perfChecks = (result.checks ?? []).filter(check => check.kind === "perf");
  if (perfChecks.length) {
    lines.push("断言与余量：");
    for (const check of perfChecks) {
      const ratio = check.expected ? (check.actual / check.expected).toFixed(2) : "?";
      lines.push(`  ${check.message}: 实测 ${check.actual} / 限 ${Math.round(check.expected ?? 0)} (${ratio}× 限制)${check.baseline !== null && check.baseline !== undefined ? `，基线 ${check.baseline}` : ""} ${check.ok ? "ok" : "FAILED"}`);
    }
  }
  return lines.join("\n");
}

/** The one place the mobile/web boundary is stated, for both judge passes. */
function surfaceCaveat(result) {
  if (result.surface !== "mobile" && result.surface !== "remote")
    return null;
  return "重要边界：手机/远程界面是通过 react-native-web 渲染的，不是真机。Modal / Alert 遮罩层的透明度、系统状态栏与安全区、原生滚动物理都与真机不同，因此「对话框背景半透、底层列表透出、弹层与列表文字重叠」这类现象是 Web 渲染的差，不要把它们判成产品缺陷（可以写成 low 级观察并注明来源）。真正与平台无关的问题（文案错、数据不加和、按钮无响应、内容缺失、层级遮挡、布局错位）照常报。";
}

/** What the deterministic layer already proved, phrased for a model reader. */
function provenText(result) {
  const proven = (result.checks ?? []).filter(check => check.ok && check.kind !== "perf");
  if (!proven.length)
    return "";
  return proven.slice(0, 20)
    .map(check => `- ${check.message}${check.actual !== undefined ? `（实测 ${truncate(JSON.stringify(check.actual), 80)}）` : ""}`)
    .join("\n");
}

function timelineText(result) {
  return (result.steps ?? [])
    .filter(step => step.action !== "note")
    .map(step => `#${step.index} ${step.action}${step.target ? ` ${truncate(JSON.stringify(step.target), 120)}` : ""} ${step.ok === false ? `FAILED: ${step.error}` : "ok"}${step.note ? ` — ${step.note}` : ""}`)
    .join("\n");
}

/**
 * Model acceptance: the "is this actually a plausible product state?" gate.
 *
 * Deterministic assertions answer contract questions ("did the row disappear").
 * They cannot answer "is this what a person would expect to see" — spacing,
 * wrong content, a spinner stuck, a dynamic sequence that only looks wrong when
 * played. This sends the ordered screenshots + the whole recording's frames +
 * the runtime logs to a vision model and asks for a verdict in prose terms.
 */
async function judgeCase(result, runDir, { model = null } = {}) {
  const criteria = result.judge?.criteria ?? result.expectations ?? [];
  if (!criteria.length && !result.perf)
    return null;
  const images = await judgeEvidence(result, runDir, { limit: result.judge?.maxImages ?? 8 });
  const logs = [];
  if (result.console?.errors?.length)
    logs.push(`console errors: ${result.console.errors.map(item => item.text).join(" | ").slice(0, 1500)}`);
  if (result.exceptions?.length)
    logs.push(`exceptions: ${result.exceptions.map(item => item.text).join(" | ").slice(0, 1500)}`);
  if (result.network?.failures?.length)
    logs.push(`network failures: ${result.network.failures.map(item => `${item.url} ${item.error}`).join(" | ").slice(0, 1000)}`);
  if (result.cli?.runs?.length)
    logs.push(`CLI runs:\n${result.cli.runs.map(run => `$ future ${(run.args ?? []).join(" ")} (exit ${run.exit})\n${run.stderr ?? ""}`).join("\n").slice(0, 3000)}`);

  const perfText = perfDigest(result);
  // What the deterministic layer already proved. Handing this to the judge stops
  // the most common false positive: "the screen does not show what was clicked"
  // asserted about a frame that proves nothing, when a check on the live DOM at
  // that step already confirmed the opposite. A model may still disagree — but
  // it has to say so explicitly, not restate the claim as a fresh finding.
  const provenChecks = (result.checks ?? []).filter(check => check.ok && check.kind !== "perf");
  const provenText = provenChecks.length
    ? provenChecks.slice(0, 20).map(check => `- ${check.message}${check.actual !== undefined ? `（实测 ${truncate(JSON.stringify(check.actual), 80)}）` : ""}`).join("\n")
    : "";
  const perfCriteria = result.judge?.criteria?.length
    ? []
    : [
        "性能与资源占用符合常识：同一操作重复执行时单次耗时不随次数上升，内存不单调增长",
        "数字本身要与量级相符（本地首屏、本地命令应是毫秒到一两秒量级，不该以十秒计）",
      ];

  const surfaceNote = surfaceCaveat(result);
  const provenance = fixtureProvenance(result);

  const prompt = [
    "你是 FutureOS 的产品验收员，不是测试脚本执行器。",
    "下面有一次真实操作的证据：按时间排列的截图、整段录像的帧拼图，以及运行时日志。",
    "请判断实际结果是否像一个正常产品，是否符合这些用户可见的预期；不要只因为断言通过就判好，也不要报告证据里看不到的东西。",
    "",
    "用户可见预期：",
    ...criteria.map(item => `- ${item}`),
    ...perfCriteria.map(item => `- ${item}`),
    "",
    `用例：${result.title}`,
    `说明：${result.description ?? ""}`,
    `操作时间线：\n${timelineText(result)}`,
    // The labels used to live only in the report: the model got the images in
    // order and nothing saying which step each one was, so a frame from before
    // an action read as a frame after it ("the fork still shows the original
    // reply"). The legend is the same list the attachments arrive in.
    images.length ? `图片（按附件顺序）：\n${images.map((item, index) => `  ${index + 1}. ${item.label}`).join("\n")}` : "",
    surfaceNote ? `\n${surfaceNote}` : "",
    provenance.text ? `\n${provenance.text}` : "",
    "",
    logs.length ? `运行时信号：\n${logs.join("\n")}` : "运行时信号：无错误",
    provenText ? `\n已由确定性断言在真实 DOM 上验证通过的事实（不要把这些当缺陷重复怀疑；如果截图看起来相反，优先认为是截图时机/滚动位置造成的证据不足）：\n${provenText}` : "",
    perfText ? `\n性能与资源数据：\n${perfText}` : "",
    result.bodyText ? `\n页面文本快照（末尾时刻，截断；证明元素存在，不代表都在可视区）：\n${truncate(result.bodyText, 4000)}` : "",
    "",
    "请先按上面的预期逐条判断，然后做一遍【自主检查】：把自己当成第一次用这个产品的人，从整套证据里找出**上面预期没提到**的问题。例如：",
    "  · 动态过程：某个状态停住、卡片闪烁、切换后先出现上一个会话的内容再跳、动画结束后留下残影；",
    "  · 状态不自洽：列表说有、主区说无，计数与行数不符，按钮可用但点了没反应，禁用态没有原因；",
    "  · 文案与数据：数字不加和、时间逆序、单位或币种不一致、同一概念两种叫法、中英文混排；",
    "  · 布局：遮挡、被浮动元素压住、空态与列表同时出现、长文本溢出、滚动到底部才有的空白；",
    "  · 资源与性能：内存或请求量随重复操作单调上升、某项操作耗时比其他同类高一截。",
    "自主发现的问题，在 title 前加「【发现】」（例如「【发现】切换后先闪一下上一个会话」），并写清是哪一帧/哪一步看到的。如果确实没发现，就不要凑数。",
    "",
    "请输出严格 JSON（不要 markdown 围栏）：",
    `{"verdict":"pass|issue","confidence":"high|medium|low","summary":"一句话判断","findings":[{"severity":"high|medium|low|info","title":"简短标题","detail":"看到什么、为什么不符合预期","evidence":"哪一步/哪一帧"}]}`,
    "示例：",
    '{"verdict":"issue","confidence":"medium","summary":"配对可用，但切换后闪现旧会话内容","findings":[{"severity":"medium","title":"【发现】切换会话时先渲染上一个会话的正文","detail":"第 6→7 帧之间主区标题已变但正文仍是上一个会话，持续约 200ms","evidence":"录像第 6、7 帧"}]}',
    "规则：",
    "- 只判断用户能看到/会遇到的问题；内部实现细节不报。",
    "- 动态过程中的瞬时状态（生成中、加载中）不要当缺陷，除非它卡住或与操作明显不符。",
    "- 如果画面正常但证据不足，verdict=pass、confidence=low，不要编造。",
    "- 时间关系：标「第 N 步」的截图是第 N 步**执行完成之后**的状态，录像抽帧从更早的时刻开始，早于某一步的帧拍的是那一步之前的样子。凡是「状态残留 / 内容没变 / 没有消失 / 数量不对」这类判断，必须能在标「最终状态」的那张截图里看到；只在中间帧里看到、最终帧里看不到的，按证据不足处理（info，并写出是哪一帧）。",
    "- 对「上面已通过断言的事实」不要再提反面结论；你只能补充断言没盖住的、画面里真实可见的问题。",
    "- 预期项出现在文本快照但截图里看不到时，说明它可能在滚动区之外，不要判成缺失；标记证据不足即可。",
    "- 代码块/表格/长文本的排版、文案、空态、错位、遮挡都算。",
    "- 性能数据要按常识审：看的是趋势与量级，不是单个绝对值。重复 N 次同一操作时，单次耗时应基本恒定；若随次数上升（累计型），说明每次留下未回收的东西，报出来并指出可能的机制（监听器未解绑、缓存无上限、节点累积）。",
    "- 内存随交互线性增长、或单次本地操作达到秒级、或首屏 IPC 请求数异常多，都属于系统性问题；用一句话说明你的推断依据（哪几个数字、什么趋势）。",
    "- 但如果数字在噪声范围内（小值波动、单次尖尖），不要当缺陷；也不要因为数字大就报缺陷，先判断它相对量级是否合理。",
  ].join("\n");

  const promptPath = join(runDir, "analysis", `${result.id}.judge.md`);
  mkdirSync(join(runDir, "analysis"), { recursive: true });
  writeFileSync(promptPath, prompt);
  try {
    const { text } = await runPrompt(promptPath, { model, images: images.map(item => item.path) });
    writeFileSync(join(runDir, "analysis", `${result.id}.judge-answer.md`), text);
    const parsed = parseFindings(text) ?? {};
    const verdict = parsed.verdict === "issue" ? "issue" : "pass";
    const findings = Array.isArray(parsed.findings) ? parsed.findings : [];
    // Second pass, no checklist. A judge that is only asked "did these
    // expectations hold" can only find what someone already thought of; this one
    // is asked to look at the same evidence as a stranger and say what looks
    // wrong. Kept as a separate call so the two answers cannot blend, and skipped
    // on surfaces that have no screen to look at — a log-only run (a relay, a CLI)
    // gives the discovery pass nothing to inspect, and it fills the gap by
    // inventing visual expectations the product never promised.
    const screenshotCount = images.filter(item => !item.path.endsWith("judge-filmstrip.png")).length;
    const discovered = screenshotCount > 0 ? await discoverCase(result, runDir, { model, images }) : [];
    return {
      verdict,
      confidence: parsed.confidence ?? "low",
      summary: parsed.summary ?? truncate(text, 300),
      findings: [...findings, ...discovered],
      discovered: discovered.length,
      images: images.map(item => item.label),
    };
  }
  catch (error) {
    return { verdict: "unknown", confidence: "low", summary: "", findings: [], images: images.map(item => item.label), error: String(error?.message ?? error) };
  }
}

/**
 * The open-ended pass: same evidence, no expectations, "what looks wrong?".
 *
 * Deliberately not given the case's criteria — the point is to find the things
 * nobody wrote down. Findings are prefixed so a report can separate them from
 * checklist verdicts, and an empty answer is a valid answer.
 */
async function discoverCase(result, runDir, { model, images }) {
  const promptPath = join(runDir, "analysis", `${result.id}.discover.md`);
  const provenance = fixtureProvenance(result);
  const prompt = [
    "你是一位资深产品验收员，第一次打开这个产品。下面是一次真实操作留下的证据：按时间排列的截图、整段录像的帧拼图，以及运行时日志与数值。",
    "**没有预期清单。**请自己看，然后说出任何看起来不像一个成熟产品的地方。",
    "值得注意的类别（不限于此）：",
    "- 时序：某一步之后画面停在旧状态、闪一下再变、动画结束时残留、计数与实际条目不符；",
    "- 状态：空态与列表并存、加载态永不结束、按钮可点但无反应、禁用却不说原因、同一信息两处不一致；",
    "- 文案：中英文混排、术语不统一、数字/单位/币种不自洽、截断到无法辨认；",
    "- 布局：遮挡、重叠、贴边、溢出、留白突兀、层级颠倒；",
    "- 数据：合计算错、时间逆序、同一实体两个名字；",
    "- 资源：耗时或内存随重复操作单调上升、某项操作明显比同类慢。",
    "",
    `用例（仅供参考它做了什么）：${result.title}`,
    `操作时间线：\n${timelineText(result)}`,
    images?.length ? `图片（按附件顺序）：\n${images.map((item, index) => `  ${index + 1}. ${item.label}`).join("\n")}` : "",
    result.bodyText ? `\n页面文本/进程日志快照（截断）：\n${truncate(result.bodyText, 4000)}` : "",
    // Same two guards the checklist pass gets, for the same reason: without them
    // this pass reads a web-rendered modal as a product defect, and restates
    // things a live-DOM assertion already proved as discoveries.
    surfaceCaveat(result) ? `\n${surfaceCaveat(result)}` : "",
    provenance.text ? `\n${provenance.text}` : "",
    provenText(result) ? `\n已由确定性断言在真实 DOM / 进程上验证通过的事实（如果截图看起来相反，先当作截图时机或滚动位置造成的证据不足，不要判成缺陷）：\n${provenText(result)}` : "",
    "",
    "输出严格 JSON（不要 markdown 围栏）。如果确实没有发现问题，findings 为空数组：",
    `{"findings":[{"severity":"high|medium|low|info","title":"【发现】简短标题","detail":"看到什么 + 为什么不像正常产品","evidence":"哪一步/哪一帧"}]}`,
    "规则：",
    "- 只写证据里真的看得到的；看不出来就写 info 并说明证据不足。",
    "- 不报实现细节、不报需要读代码才能知道的事、不报风格偏好（例如“我更喜欢圆角”）。",
    "- 生成中/加载中的瞬时状态不算；除非它卡住或与操作明显不符。",
    "- 视频抽帧与最终截图可能不同步：标「第 N 步」的截图是该步完成后的状态，早于它的帧是操作之前的样子；「残留 / 没消失 / 没变化」必须能在最终状态帧里看到，否则写 info 并注明证据不足。",
    "- 宁少勿多：一条看得出、说得清的问题，胜过五条猜测。",
    "- 与上面「已验证通过」的事实冲突的观察，写成 low 并注明证据不足，不要写 high/medium。",
  ].join("\n");
  writeFileSync(promptPath, prompt);
  try {
    const { text } = await runPrompt(promptPath, { model, images: images.map(item => item.path) });
    writeFileSync(join(runDir, "analysis", `${result.id}.discover-answer.md`), text);
    const parsed = parseFindings(text) ?? {};
    const findings = Array.isArray(parsed.findings) ? parsed.findings : [];
    // Mark them here rather than trusting the model to remember the prefix.
    return findings.map(item => ({ ...item, title: /^\s*【发现】/.test(item.title ?? "") ? item.title : `【发现】${item.title ?? "未命名发现"}` }));
  }
  catch {
    return [];
  }
}

function buildSynthesisPrompt(result, findings, notes) {
  const stepLines = (result.steps ?? [])
    .filter(step => step.action !== "note")
    .map(step => `- #${step.index} ${step.action} ${step.target ? JSON.stringify(step.target).slice(0, 120) : ""} ${step.ok === false ? `FAILED: ${step.error}` : "ok"}${step.ms ? ` (${step.ms}ms)` : ""}${step.note ? ` — ${step.note}` : ""}`)
    .join("\n");
  return [
    "你是资深产品测试工程师。下面是 FutureOS 桌面端一个端到端用例的执行证据，请判断产品是否存在真实缺陷。",
    "",
    `用例：${result.title}`,
    `说明：${result.description ?? ""}`,
    `前置：${result.preconditions ?? ""}`,
    `期望：\n${(result.expectations ?? []).map(item => `- ${item}`).join("\n")}`,
    `状态：${result.status}（用例总耗时 ${result.durationMs}ms——这是整条用例跑了多久，不是任何单项性能指标）`,
    perfDigest(result) ? `\n性能断言（实测 / 允许上限 / 基线）：\n${perfDigest(result)}\n` : "",
    "",
    "步骤时间线：",
    stepLines,
    "",
    "确定性信号（失败检查 / 异常 / 请求失败 / IPC 错误）：",
    findings.length ? findings.map(item => `- [${item.severity}] ${item.title} :: ${truncate(item.detail, 500)}`).join("\n") : "- 无",
    "",
    "视觉模型对关键截图的读图结论：",
    notes.length ? notes.map(item => `- 第 ${item.step} 步 ${item.note}: ${truncate(item.answer ?? item.error, 800)}`).join("\n") : "- 无",
    "",
    "请输出严格 JSON（不要 markdown 代码围栏）：",
    `{"summary":"本用例一句话结论","findings":[{"severity":"high|medium|low|info","category":"behaviour|visual|copy|performance|accessibility|data","title":"简短标题","detail":"现象 + 影响 + 判断依据","evidence":"第几步/哪张图","confidence":"high|medium|low"}]}`,
    "规则：",
    "- 只报告有证据的问题；不确定的标 info 或不要报。",
    "- 不要复述用例本身，不要编造截图里没有的内容。",
    "- 如果证据显示产品行为符合预期，findings 为空数组。",
    "- 「删除当前会话后列表仍显示」这类状态残留算 behaviour。",
    "- 确定性信号（失败检查、异常、请求失败、DB 不一致）优先于单帧读图；两者冲突时，先怀疑截图时机或读图误差，最多报 info。",
    "- 不要仅凭一帧判断某个分组/入口「不存在」：它可能被滚动、折叠或浮层挡住。",
    "- 若截图/录像处于生成中（状态栏有“生成”），代码块或 markdown 的瞬时错位不要报为缺陷，除非稳定后的帧仍然如此。",
    "- 用例本身通过时，不要把「界面看起来没变」直接判成缺陷；说明它为什么没有被断言捕捉到。",
    "- 引用任何数字（耗时、MB、请求数、金额）必须是上面证据里出现过的原文；拼不准就写「约」或不要引数字。编造一个数字会让整份报告失去可信度。",
    "- 如果确定性检查带着基线与限值（如 baseline 2065 × 1.25 = 2581），先看“实测/限”再下结论；不要自己重算或估一个值。",
  ].join("\n");
}

function parseFindings(text) {
  const candidates = [];
  const fenced = text.match(/```(?:json)?\s*([\s\S]*?)```/g) ?? [];
  candidates.push(...fenced.map(block => block.replace(/```(?:json)?/g, "").replace(/```/g, "")));
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start >= 0 && end > start)
    candidates.push(text.slice(start, end + 1));
  for (const candidate of candidates) {
    try {
      const parsed = JSON.parse(candidate.trim());
      if (parsed && Array.isArray(parsed.findings))
        return parsed;
    }
    catch {}
  }
  return null;
}

export async function analyzeRun(runDir, { mode = "heuristic", model = null, only = [] } = {}) {
  runDir = resolve(runDir);
  const manifestPath = join(runDir, "run.json");
  if (!existsSync(manifestPath))
    throw new Error(`not a run directory (no run.json): ${runDir}`);
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const previousPath = join(runDir, "analysis.json");
  const previous = existsSync(previousPath) ? JSON.parse(readFileSync(previousPath, "utf8")) : null;
  const previousCases = new Map((previous?.cases ?? []).map(item => [item.id, item]));
  const caseDirs = readdirSync(runDir).filter((name) => {
    try {
      return existsSync(join(runDir, name, "result.json")) && (!only.length || only.includes(name));
    }
    catch {
      return false;
    }
  });

  const results = caseDirs.map(name => JSON.parse(readFileSync(join(runDir, name, "result.json"), "utf8")));
  const visionModel = model || defaultAnalyzerModel();
  const nativeVision = supportsVision(visionModel);
  const cases = [];
  for (const result of results) {
    process.stdout.write(`  analyze ${result.id}… `);
    const deterministic = deterministicFindings(result, { runDir });
    const notes = mode === "full" ? await visionFindings(result, runDir, { native: nativeVision, model: visionModel }) : [];
    const judged = mode === "full" ? await judgeCase(result, runDir, { model: visionModel }) : null;
    let synthesized = null;
    if (mode === "full") {
      const prompt = buildSynthesisPrompt(result, deterministic, notes);
      const promptPath = join(runDir, "analysis", `${result.id}.prompt.md`);
      mkdirSync(join(runDir, "analysis"), { recursive: true });
      writeFileSync(promptPath, prompt);
      try {
        const { text } = await runPrompt(promptPath, { model });
        writeFileSync(join(runDir, "analysis", `${result.id}.answer.md`), text);
        synthesized = parseFindings(text);
        if (!synthesized)
          synthesized = { summary: truncate(text, 400), findings: [], parseError: true };
      }
      catch (error) {
        synthesized = { summary: "", findings: [], error: String(error?.message ?? error) };
      }
    }
    // 「【发现】」 marks something the model found on its own, outside the case's
    // expectations. Keeping the flag separate is what lets the report say how
    // much of a run's value came from judgement rather than from a checklist.
    const judgeFindings = (judged?.findings ?? []).map((item, index) => {
      const raw = item.title ?? "（模型验收未命名）";
      const discovered = /^\s*【发现】\s*/.test(raw);
      return {
        id: `${result.id}:judge:${index}`,
        severity: item.severity ?? (judged?.verdict === "issue" ? "medium" : "info"),
        category: item.category ?? "behaviour",
        title: raw.replace(/^\s*【发现】\s*/, ""),
        detail: item.detail ?? judged?.summary ?? "",
        evidence: { caseId: result.id, quote: item.evidence ?? null, dir: result.artifacts?.dir },
        source: "judge",
        discovered,
        confidence: judged?.confidence ?? "medium",
      };
    });
    if (judged?.verdict === "issue" && !judgeFindings.length) {
      judgeFindings.push({
        id: `${result.id}:judge:summary`,
        severity: "medium",
        category: "behaviour",
        title: judged.summary || "模型验收未通过",
        detail: judged.summary || "模型认为实际结果与预期不符。",
        evidence: { caseId: result.id, dir: result.artifacts?.dir },
        source: "judge",
        confidence: judged.confidence ?? "medium",
      });
    }
    const found = [
      ...deterministic,
      ...judgeFindings,
      ...(synthesized?.findings ?? []).map((item, index) => ({
        id: `${result.id}:model:${index}`,
        severity: item.severity ?? "info",
        category: item.category ?? "behaviour",
        title: item.title ?? "（无标题）",
        detail: item.detail ?? "",
        evidence: { caseId: result.id, quote: item.evidence ?? null, dir: result.artifacts?.dir },
        source: "model",
        confidence: item.confidence ?? "medium",
      })),
    ];
    // A passing case means every explicit check held. A model may still report a
    // visual or copy problem (checks rarely cover those), but a *behaviour*
    // claim that contradicts the checks gets demoted to an observation — the
    // deterministic evidence outranks a reading of a single frame, and the
    // report says so rather than asking a human to arbitrate every one.
    const failedChecks = (result.checks ?? []).filter(check => !check.ok);
    // A model reviewing its own evidence produces useful harness notes ("the
    // screenshot did not cover the moment") that are not product findings. Keep
    // those out of the defect list and out of issue drafts; they are recorded
    // beside the case as harness observations.
    const evidenceNoise = /^(截图|录像|读图|视觉证据|证据).*(未|没有|不足|不一致|无法|覆盖|时机)|未被用例真正验证|断言没有覆盖/;
    const findings = [];
    const harnessNotes = [];
    for (const finding of found) {
      if (finding.source === "model" && ["low", "info"].includes(finding.severity) && evidenceNoise.test(finding.title)) {
        harnessNotes.push(finding);
        continue;
      }
      if (finding.source === "model" && result.status === "pass" && !failedChecks.length && finding.category === "behaviour") {
        finding.severity = "low";
        finding.confidence = "low";
        finding.demoted = "用例断言全部通过；该结论仅来自读图，已降级为观察项，需人工复核";
      }
      findings.push(finding);
    }
    findings.sort((a, b) => severityRank(a.severity) - severityRank(b.severity));

    cases.push({
      id: result.id,
      title: result.title,
      status: result.status,
      durationMs: result.durationMs,
      checks: result.checks ?? [],
      expectations: result.expectations ?? [],
      summary: synthesized?.summary ?? "",
      analysisError: synthesized?.error ?? null,
      vision: notes,
      judge: judged,
      harnessNotes,
      findings,
      artifacts: result.artifacts,
    });
    process.stdout.write(`${findings.length} findings\n`);
  }

  if (only.length) {
    for (const [id, item] of previousCases) {
      if (!only.includes(id) && !cases.some(current => current.id === id))
        cases.push(item);
    }
  }

  // Second-level discovery: the problems that only exist between cases. Runs on
  // the completed picture (including cases carried over from an earlier pass), and
  // only in full mode — it needs the per-case findings to compare.
  let crossCase = { findings: [], error: null };
  if (mode === "full" && cases.length >= 2) {
    process.stdout.write("  cross-case discovery… ");
    try {
      crossCase = await discoverAcrossCases({ analysis: { cases }, runDir, model, runPrompt });
    }
    catch (error) {
      // A failed second-level pass must not lose the per-case work.
      crossCase = { findings: [], error: String(error?.message ?? error) };
    }
    process.stdout.write(crossCase.error ? `skipped (${crossCase.error.slice(0, 80)})\n` : `${crossCase.findings.length} findings\n`);
  }
  const allFindings = cases.flatMap(item => item.findings);
  // Cross-case findings were produced after the per-case loop, so they are merged
  // here and every total comes from the merged list — a count that disagrees with
  // the list it summarises is the kind of thing this harness exists to catch.
  const mergedFindings = [...allFindings, ...(crossCase?.findings ?? [])];
  // A value only the harness can produce is not a product defect, whoever saw
  // it: demote once, over the merged list, so the report and the issue flow can
  // never disagree about the severity.
  markFixtureFindings(mergedFindings);
  const analysis = {
    runId: manifest.id,
    generatedAt: new Date().toISOString(),
    mode,
    model,
    cases: cases.map((item) => {
      const checks = Array.isArray(item.checks) ? item.checks : [];
      return {
        id: item.id,
        title: item.title,
        surface: item.surface ?? "desktop",
        status: item.status,
        durationMs: item.durationMs,
        checks: checks.length,
        failedChecks: checks.filter(check => !check.ok).length,
        expectations: item.expectations ?? [],
        summary: item.summary,
        analysisError: item.analysisError,
        vision: (Array.isArray(item.vision) ? item.vision : []).map(note => ({ step: note.step, note: note.note, file: note.file, via: note.via ?? null, error: note.error ?? null })),
        judge: item.judge ?? null,
        harnessNotes: item.harnessNotes ?? [],
        findings: item.findings,
        artifacts: item.artifacts,
      };
    }),
    findings: mergedFindings,
    crossCase: { findings: crossCase.findings, error: crossCase.error },
    totals: {
      cases: cases.length,
      passed: cases.filter(item => item.status === "pass").length,
      failed: cases.filter(item => item.status !== "pass").length,
      findings: mergedFindings.length,
      high: mergedFindings.filter(item => item.severity === "high").length,
      medium: mergedFindings.filter(item => item.severity === "medium").length,
      low: mergedFindings.filter(item => item.severity === "low").length,
      info: mergedFindings.filter(item => item.severity === "info").length,
      discovered: mergedFindings.filter(item => item.discovered).length,
      crossCase: mergedFindings.filter(item => item.source === "cross-case").length,
      fixture: mergedFindings.filter(item => item.fixture).length,
    },
  };
  writeFileSync(join(runDir, "analysis.json"), JSON.stringify(analysis, null, 2));
  writeFileSync(join(runDir, "findings.md"), renderFindings(analysis, runDir));
  return analysis;
}

function renderFindings(analysis, runDir) {
  const lines = [
    `# Findings — ${analysis.runId}`,
    "",
    `模式：${analysis.mode}；用例 ${analysis.totals.cases}，通过 ${analysis.totals.passed}，问题 ${analysis.totals.findings}（high ${analysis.totals.high} / medium ${analysis.totals.medium} / low ${analysis.totals.low}）。`,
    "",
  ];
  for (const item of analysis.cases) {
    lines.push(`## ${item.id} — ${item.title}（${item.status}）`);
    if (item.summary)
      lines.push("", item.summary);
    if (!item.findings.length)
      lines.push("", "没有发现问题。");
    for (const finding of item.findings) {
      lines.push("", `### [${finding.severity}] ${finding.title}${finding.fixture ? "（夹具数据，不计入缺陷）" : ""}`);
      lines.push("", finding.detail);
      if (finding.evidence?.dir)
        lines.push("", `证据：${relative(runDir, finding.evidence.dir)}`);
    }
    lines.push("");
  }
  return lines.join("\n");
}
