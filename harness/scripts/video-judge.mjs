#!/usr/bin/env node
/**
 * Judge a recorded UI flow: hand the video to a video-capable model and ask whether
 * the run matched the expected flow, plus what it saw go wrong.
 *
 * Two constraints learned the hard way (2026-11-06), both enforced here rather than
 * trusted to the prompt:
 *
 *  · **A transport failure is not a verdict.** A 300 s HTTP/2 stream timeout used to
 *    throw the whole judgement away; it is now retried once and the result records
 *    that it was. Two of thirteen recordings were lost to that.
 *  · **Every issue must quote what it saw.** The model once reported
 *    「暂无会话 yet 工作区/对话」, which is not text the app contains — it merged two
 *    locale strings — and once read a *red* button as an *enabled* one. So: an issue
 *    without a verbatim `quote` is dropped, a quote whose words do not exist in the
 *    app's own copy is dropped, and a claim about interactivity without a state
 *    indicator in the quote is dropped. If nothing survives, the verdict is `pass`.
 *
 * Measured on the desktop renderer: three injected anomalies were all reported with
 * the right step; clean recordings produced no surviving issue. A judgement takes
 * 60-260 s, so run it per case asynchronously, never per assertion.
 *
 * Usage:
 *   FUTURE_API_KEY=... node harness/scripts/video-judge.mjs \
 *     --video run.mp4 --expect "依次打开 …" [--corpus run/result.json] [--json out.json]
 *
 * Exit code: 0 = pass, 1 = fail, 2 = usage/transport error.
 */
import fs from "node:fs";
import path from "node:path";

const API = "https://future-os.cn/api/v1/chat/completions";
const MODEL = "qwen3.8-max";

export const PROMPT = (expect) =>
  `这是一段自动化测试的录屏。测试目标是：${expect}\n\n` +
  "你只能报告**在画面里确实看见的**异常。看不见不等于没发生——录屏是低帧率采样。\n\n" +
  "每条 issue 必须给出 `quote`：**从画面上原样抄下的文字**（不要改写、不要拼接、不要翻译）。" +
  "引用不出来的观察，不要写进来。\n\n" +
  "只在下列情况判 fail：屏幕上出现报错提示；同一屏两处内容互相矛盾；某个提示跨多个界面" +
  "持续存在并遮挡内容；点击之后画面完全没有变化；某个本该出现的东西没出现。\n" +
  "不算异常：没看到的动作、没看清的文字、录屏开始前就存在的浮层、与目标无关的既有 UI。\n\n" +
  "只输出一个 JSON，不要 markdown：\n" +
  '{"verdict":"pass|fail","summary":"一句话",' +
  '"issues":[{"at":"时间点","quote":"画面上原样抄下的文字","what":"现象","why":"为什么算异常"}]}';

/** Words that exist in the product: every string in its locale bundles, plus an
 *  optional corpus file (a case's captured body text). A quote built from words
 *  that are nowhere in here was not read off the screen. */
export function buildCorpus({ repoRoot, extraFiles = [] } = {}) {
  const parts = [];
  const localeDir = path.join(repoRoot, "desktop", "src", "i18n", "locales");
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.name.endsWith(".json")) {
        try {
          const collect = (o) => {
            if (typeof o === "string") parts.push(o);
            else if (Array.isArray(o)) o.forEach(collect);
            else if (o && typeof o === "object") Object.values(o).forEach(collect);
          };
          collect(JSON.parse(fs.readFileSync(p, "utf8")));
        } catch { /* a broken locale file should not break judging */ }
      }
    }
  };
  walk(localeDir);
  for (const f of extraFiles) {
    try {
      const raw = fs.readFileSync(f, "utf8");
      const parsed = f.endsWith(".json") ? JSON.parse(raw) : null;
      parts.push(parsed?.bodyText ?? raw);
    } catch { /* ignore */ }
  }
  return parts.join("\n");
}

const STATE_CLAIM = /(可用|可点击|已启用|enabled|clickable)/i;
const STATE_EVIDENCE = /(禁用|disabled|置灰|不可用|aria-)/i;

/** Drop issues that cannot be backed by a verbatim quote; demote fail to pass when
 *  nothing survives. Pure function so the rules can be tested without a model. */
export function applyQuoteGate(judged, corpus) {
  const issues = Array.isArray(judged.issues) ? judged.issues : [];
  const rejected = [];
  if (judged.verdict !== "fail") return { ...judged, issues, rejected, downgraded: false };

  const kept = [];
  for (const issue of issues) {
    const quote = String(issue.quote ?? "").trim();
    const claim = `${issue.why ?? ""} ${issue.what ?? ""}`;
    if (!quote) {
      rejected.push({ at: issue.at ?? null, why: "没有原样引用画面文字" });
      continue;
    }
    // Only CJK words are checked against the product copy: an English word in a
    // quote is usually a state attribute read off the DOM (`disabled`), not UI text.
    const tokens = quote.match(/[\u4e00-\u9fa5]{2,}/g) ?? [];
    const unknown = tokens.filter((t) => !corpus.includes(t));
    if (unknown.length) {
      rejected.push({ at: issue.at ?? null,
        why: `引用里的词在应用文案中不存在（疑似改写或拼接）：${unknown.slice(0, 3).join("、")}` });
      continue;
    }
    if (STATE_CLAIM.test(claim) && !STATE_EVIDENCE.test(quote)) {
      rejected.push({ at: issue.at ?? null,
        why: "从外观推断控件可用性，引用里没有状态证据（disabled/禁用/置灰）" });
      continue;
    }
    kept.push(issue);
  }
  const verdict = kept.length ? "fail" : "pass";
  return { ...judged, verdict, issues: kept, rejected, downgraded: verdict !== judged.verdict };
}

async function post(body, { fetchImpl }) {
  const res = await fetchImpl(API, {
    method: "POST",
    headers: { "Content-Type": "application/json",
      Authorization: `Bearer ${process.env.FUTURE_API_KEY}` },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).slice(0, 300)}`);
  return res.json();
}

/** One judgement, retried once: a stream timeout says nothing about the recording. */
export async function judge({ video, expect, fetchImpl = fetch, model = MODEL }) {
  if (!fs.existsSync(video)) throw new Error(`no such video: ${video}`);
  const b64 = fs.readFileSync(video).toString("base64");
  const body = {
    model,
    messages: [{ role: "user", content: [
      { type: "text", text: PROMPT(expect) },
      { type: "video_url", video_url: { url: `data:video/mp4;base64,${b64}` } }] }],
    max_tokens: 800, temperature: 0.2,
  };

  let attempts = 0, lastError = null, payload = null;
  const started = Date.now();
  while (attempts < 2) {
    attempts += 1;
    try { payload = await post(body, { fetchImpl }); lastError = null; break; }
    catch (error) { lastError = error; }
  }
  if (!payload) throw new Error(`judge failed after ${attempts} attempt(s): ${lastError?.message ?? lastError}`);

  const txt = payload.choices?.[0]?.message?.content ?? "";
  const parsed = JSON.parse((txt.match(/\{[\s\S]*\}/) ?? ["{}"])[0]);
  return {
    ...parsed,
    retried: attempts > 1,
    attempts,
    model,
    usage: payload.usage ?? null,
    seconds: Math.round((Date.now() - started) / 1000),
    raw: txt,
  };
}

async function main() {
  const argv = process.argv.slice(2);
  const arg = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && argv[i + 1] ? argv[i + 1] : d; };
  const video = arg("video"), expect = arg("expect"), out = arg("json");
  const corpusFile = arg("corpus");
  if (!video || !expect) { console.error("need --video and --expect"); process.exit(2); }
  if (!process.env.FUTURE_API_KEY) { console.error("missing FUTURE_API_KEY"); process.exit(2); }

  const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..", "..");
  let judged;
  try {
    judged = await judge({ video, expect, model: arg("model", MODEL) });
  } catch (error) {
    console.error(String(error.message ?? error));
    process.exit(2);
  }

  const corpus = buildCorpus({ repoRoot, extraFiles: corpusFile ? [corpusFile] : [] });
  const gated = applyQuoteGate(judged, corpus);
  const result = { video, expect, model: judged.model, seconds: judged.seconds,
    retried: judged.retried, attempts: judged.attempts, usage: judged.usage,
    verdict: gated.verdict, downgraded: gated.downgraded,
    summary: judged.summary ?? null, issues: gated.issues, rejected: gated.rejected };

  if (out) fs.writeFileSync(path.resolve(out), JSON.stringify({ ...result, raw: judged.raw }, null, 2));
  console.log(JSON.stringify(result, null, 1));
  process.exit(result.verdict === "pass" ? 0 : 1);
}

if (process.argv[1] && process.argv[1].endsWith("video-judge.mjs")) {
  main();
}
