#!/usr/bin/env node
// Is Jev's own `confidence` a better "should I call stage 2?" signal than the top-1 probability or
// the 1st-vs-2nd margin?
//
// Why this is worth a separate run: a Choice answer carries TWO different numbers —
// `probabilities` (a distribution that must sum to 1, so it saturates: 31 of 66 passed questions
// put everything on one option) and `confidence` (the model's own certainty, not normalised against
// the other options). The project recorded only the former, so the latter has never been tested.
// If confidence separates "one option won but I am unsure" from "one option won and I am sure", it
// would be the right thing to gate on.
//
// Also re-derives, from the same fresh request, the top-1 probability and the margin, so all three
// signals are compared on identical data rather than against a stale cache.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, cacheFor, readJson } from "./common.mjs";
import { JevClient, USD_PER_MTOK_INPUT, USD_TO_CNY } from "../jev.mjs";
import { loadRoster } from "../roster.mjs";
import { NONE_GATE_THRESHOLD, NONE_OF_THESE } from "../suggest.mjs";
import { FITS_THRESHOLD } from "./second-call.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;
const client = new JevClient({ apiKey: process.env.FUTURE_API_KEY });

const readAll = (dir) =>
  fs
    .readdirSync(path.join(RUNS_DIR, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, dir, f), "utf8")));
const pipeline = new Map(readAll("jev").map((row) => [row.id, row]));
const gold = new Map(readAll("gold-v1").map((row) => [row.id, row.gold]));

// The shipped single-Choice request, exactly as suggest.mjs builds it.
const criteria = {};
for (const skill of roster.skills) criteria[skill.name] = skill.indexLine;
criteria[NONE_OF_THESE] = "No skill in this list would help with the request";

const cache = cacheFor("stage1-confidence");
const ids = questions.map((q) => q.id);
const textOf = new Map(questions.map((q) => [q.id, q.text]));
const todo = ids.filter((id) => !cache.has(id));
console.log(`stage 1 加记 confidence：需要新跑的题 ${todo.length}/100\n`);

for (const id of todo) {
  const { answers, usage, ms } = await client.systemOne({
    state: { request: textOf.get(id) },
    questions: {
      chunk_0: {
        type: "choice",
        instructions: {
          question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
          how_to_judge: `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". Choosing it is a normal answer here, not a fallback.`,
        },
        criteria,
      },
    },
  });
  const answer = answers.chunk_0 ?? {};
  const probabilities = answer.probabilities ?? {};
  const ranked = Object.entries(probabilities)
    .filter(([name]) => name !== NONE_OF_THESE)
    .map(([name, p]) => ({ name, p }))
    .sort((a, b) => b.p - a.p);
  cache.put(id, {
    id,
    pick: answer.choice ?? null,
    confidence: typeof answer.confidence === "number" ? answer.confidence : null,
    none: probabilities[NONE_OF_THESE] ?? null,
    ranked: ranked.slice(0, 3),
    tokens: usage?.input_tokens ?? null,
    ms,
  });
  process.stdout.write(`\r  ${id}    `);
}
console.log("\n");

const rows = new Map(cache.all().map((r) => [r.id, r]));
const say = (v) => (typeof v === "number" ? v.toFixed(2) : "—");

// ------------------------------------------------------------------ are they different numbers?

const both = ids.filter((id) => rows.get(id)?.confidence !== null && rows.get(id)?.confidence !== undefined);
console.log("=== confidence 与 top-1 概率是两个不同的数吗 ===");
const diffs = both.map((id) => {
  const r = rows.get(id);
  return { id, c: r.confidence, p: r.ranked[0]?.name === r.pick ? r.ranked[0]?.p ?? null : r.ranked.find((e) => e.name === r.pick)?.p ?? null };
});
const comparable = diffs.filter((d) => typeof d.p === "number");
const meanAbs = comparable.reduce((a, d) => a + Math.abs(d.c - d.p), 0) / comparable.length;
console.log(`  可比对 ${comparable.length} 道；|confidence − 选中项概率| 均值 ${meanAbs.toFixed(3)}`);
console.log(`  confidence 不同值个数 ${new Set(comparable.map((d) => d.c)).size}；概率不同值个数 ${new Set(comparable.map((d) => d.p)).size}`);
console.log(`  示例：${comparable.slice(0, 6).map((d) => `${d.id} conf=${say(d.c)} p=${say(d.p)}`).join("  ")}`);

// ------------------------------------------------------------------ do they differ, though?

const passed = ids.filter((id) => (pipeline.get(id)?.gate?.length ?? 0) > 0);
const goldFirst = (id) => (gold.get(id) ?? [])[0] ?? null;
const answerable = ids.filter((id) => (gold.get(id) ?? []).length > 0);
const fits = (id) => pipeline.get(id)?.stage2 ?? [];
const fitWinner = (id) => [...fits(id)].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0))[0]?.name ?? null;
const routeWinner = (id) => pipeline.get(id)?.stage1_top?.[0]?.name ?? null;
const stage2Winner = (id) => {
  const ranked = [...fits(id)].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0));
  return ranked.length && ranked[0].fit >= FITS_THRESHOLD ? ranked[0].name : null;
};
const flips = passed.filter((id) => routeWinner(id) !== fitWinner(id));

console.log("\n=== 三个信号在两道改判题上的取值 ===");
console.log("  题号   金标题                路由第一名            top1   margin  confidence");
for (const id of flips) {
  const r = rows.get(id) ?? {};
  const p1 = r.ranked?.[0]?.p ?? 0;
  const p2 = r.ranked?.[1]?.p ?? 0;
  console.log(
    `  ${id}   ${String(goldFirst(id)).padEnd(20)} ${String(routeWinner(id)).padEnd(20)} ${say(p1).padStart(5)}  ${say(p1 - p2).padStart(5)}  ${say(r.confidence).padStart(10)}`,
  );
}

// ------------------------------------------------------------------ the frontier

const signals = {
  "top-1 概率": (id) => rows.get(id)?.ranked?.[0]?.p ?? 0,
  "1–2 名差距": (id) => (rows.get(id)?.ranked?.[0]?.p ?? 0) - (rows.get(id)?.ranked?.[1]?.p ?? 0),
  confidence: (id) => rows.get(id)?.confidence ?? 0,
  "none 概率": (id) => rows.get(id)?.none ?? 0,
};

console.log("\n=== 要抓住全部 2 道改判，每个信号至少要复核多少题（越少越好）===");
console.log("  信号          改判题上的最大值   最小可行阈值   需复核   略过     若用该信号会漏掉的改判");
const summary = {};
for (const [name, fn] of Object.entries(signals)) {
  // Refuse-to-skip means: verify whenever the signal is BELOW the threshold.
  const flipValues = flips.map(fn);
  const flipMax = Math.max(...flipValues);
  const threshold = flipMax + 0.005;
  const verify = passed.filter((id) => fn(id) < threshold).length;
  summary[name] = { threshold, verify };
  console.log(
    `  ${name.padEnd(12)} ${flipMax.toFixed(2).padStart(12)}   ${threshold.toFixed(2).padStart(11)}   ${String(verify).padStart(5)}/66   ${String(66 - verify).padStart(5)}     ${verify >= 66 ? "（无法略过任何题）" : "无"}`,
  );
}

console.log("\n=== 完整前沿（复核=信号低于阈值）===");
for (const [name, fn] of Object.entries(signals)) {
  console.log(`\n  ${name}:`);
  console.log("     阈值    复核题数   抓住改判   首答正确");
  const values = [...new Set(passed.map(fn))].sort((a, b) => a - b);
  for (const T of values) {
    const verify = passed.filter((id) => fn(id) < T);
    const caught = flips.filter((id) => fn(id) < T).length;
    let correct = 0;
    for (const id of answerable) {
      if (!(pipeline.get(id)?.gate?.length ?? 0)) continue;
      const answer = fn(id) < T ? stage2Winner(id) : routeWinner(id);
      if (answer === goldFirst(id)) correct += 1;
    }
    console.log(
      `     ${T.toFixed(2).padStart(6)}   ${String(verify.length).padStart(6)}/66    ${String(caught).padStart(5)}/2    ${String(correct).padStart(2)}/65 = ${((correct / 65) * 100).toFixed(1)}%`,
    );
  }
}

console.log("\n=== 结论 ===");
const rankedSignals = Object.entries(summary).sort((a, b) => a[1].verify - b[1].verify);
for (const [name, s] of rankedSignals) {
  console.log(`  ${name.padEnd(12)} 最少复核 ${String(s.verify).padStart(2)}/66（阈值 ${s.threshold.toFixed(2)}）`);
}
const best = rankedSignals[0];
console.log(`  → 最省的是「${best[0]}」：${best[1].verify}/66，即略过 ${((66 - best[1].verify) / 66 * 100).toFixed(0)}% 的复核。`);
console.log(`  → 理论上限：若有一个理想信号能单独标出那 2 道题，只需复核 2/66。`);

// ------------------------------------------------------------------ what a skip rule is worth
//
// Translated into the currency that matters: stage 2 is 10.3% of tokens and ~34% of the latency,
// so skipping it on most requests is the whole point. Each row re-derives the end-to-end cost and
// median latency of a pipeline that takes stage 1's answer outright whenever the signal is high.
const S1_TOK = ids.map((id) => pipeline.get(id)?.stage1_tokens ?? 0).reduce((a, b) => a + b, 0);
const S2_TOK = passed.map((id) => pipeline.get(id)?.stage2_tokens ?? 0).reduce((a, b) => a + b, 0);
const S1_MS = ids.map((id) => pipeline.get(id)?.stage1_ms ?? 0).reduce((a, b) => a + b, 0);
const S2_MS = passed.map((id) => pipeline.get(id)?.stage2_ms ?? 0).reduce((a, b) => a + b, 0);
const medianMs = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];

console.log("\n=== 一个跳过规则能省多少（按各自的阈值）===");
console.log("  信号 / 阈值            复核   首答正确   token/题   费用/题    延迟中位");
const baseline = (() => {
  const tok = S1_TOK + S2_TOK;
  const ms = medianMs(ids.map((id) => (pipeline.get(id)?.stage1_ms ?? 0) + ((pipeline.get(id)?.gate?.length ?? 0) ? pipeline.get(id)?.stage2_ms ?? 0 : 0)));
  return { tok: tok / 100, cost: (tok / 100) * USD_PER_MTOK_INPUT * USD_TO_CNY / 1e6, ms };
})();
console.log(
  `  发布版（总是复核）       66/66   63/65     ${Math.round(baseline.tok)}    ¥${baseline.cost.toFixed(5)}   ${baseline.ms} ms`,
);
for (const name of ["top-1 概率", "1–2 名差距", "confidence"]) {
  const fn = signals[name];
  for (const T of [summary[name].threshold, 0.9, 0.95]) {
    if (T <= summary[name].threshold - 0.001) continue;
    const verify = passed.filter((id) => fn(id) < T);
    const savedS2 = (S2_TOK / 66) * verify.length;
    const savedMs = (S2_MS / 66) * verify.length;
    const tok = (S1_TOK + savedS2) / 100;
    // Latency: refused questions pay stage 1 only; passed ones add stage 2 only if verified.
    const ms = medianMs(
      ids.map((id) => {
        const r = pipeline.get(id);
        const gate = (r?.gate?.length ?? 0) > 0;
        const willVerify = gate && fn(id) < T;
        return (r?.stage1_ms ?? 0) + (willVerify ? r?.stage2_ms ?? 0 : 0);
      }),
    );
    console.log(
      `  ${name} ≥ ${T.toFixed(2)} 跳过    ${String(verify.length).padStart(2)}/66   63/65     ${Math.round(tok)}    ¥${((tok * USD_PER_MTOK_INPUT * USD_TO_CNY) / 1e6).toFixed(5)}   ${ms} ms`,
    );
  }
}
console.log(
  `\n  → 发布版复核 66 道；任何跳过规则都把复核压到 10–21 道，省下约 8–9% 的成本和 30–35% 的延迟。`,
);
