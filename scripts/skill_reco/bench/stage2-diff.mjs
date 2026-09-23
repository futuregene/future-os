#!/usr/bin/env node
// Stage 1 vs stage 2, question by question, and an anatomy of every final error.
//
// Reads the cached runs only — no API calls. Three questions this answers:
//
//  1. How often do the two stages disagree about the winner, and when they do, which one was
//     right? That is the only honest way to say what stage 2 is buying.
//  2. Is there a signal in stage 1's OWN output (how much probability it put on its top-1, and how
//     far ahead of top-2 it was) that predicts whether stage 2 will overturn it? If a confident
//     stage 1 is never wrong, stage 2 could be skipped on those and the cost would drop.
//  3. For each final error: what did the two stages see, how close were the scores, and was the
//     mistake a ranking failure, a gate failure, or a genuinely ambiguous request?
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, readJson } from "./common.mjs";
import { loadRoster } from "../roster.mjs";
import { NONE_GATE_THRESHOLD } from "../suggest.mjs";
import { FITS_THRESHOLD } from "./second-call.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;

const readAll = (dir) =>
  fs
    .readdirSync(path.join(RUNS_DIR, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, dir, f), "utf8")));

const byId = (rows) => new Map(rows.map((row) => [row.id, row]));
const pipeline = byId(readAll("jev"));
const gold = new Map(readAll("gold-v1").map((row) => [row.id, row.gold]));
const textOf = new Map(questions.map((q) => [q.id, q.text]));

const ids = questions.map((q) => q.id);
const goldOf = (id) => gold.get(id) ?? [];
const goldFirst = (id) => goldOf(id)[0] ?? null;
const answerable = ids.filter((id) => goldOf(id).length > 0);
const refusable = ids.filter((id) => goldOf(id).length === 0);

const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;
const f2 = (x) => (x === null || x === undefined ? "—" : Number(x).toFixed(2));

const route = (id) => pipeline.get(id)?.stage1_top ?? [];
const routeWinner = (id) => route(id)[0]?.name ?? null;
const fits = (id) => pipeline.get(id)?.stage2 ?? [];
const fitWinner = (id) => {
  const ranked = [...fits(id)].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0));
  return ranked[0]?.name ?? null;
};
const fitOf = (id, name) => fits(id).find((c) => c.name === name)?.fit ?? null;
const gatePasses = (id) => (pipeline.get(id)?.gate?.length ?? 0) > 0;
const finalOf = (id) => pipeline.get(id)?.answer?.[0] ?? null;
const top1p = (id) => route(id)[0]?.p ?? null;
const margin = (id) => {
  const r = route(id);
  if (r.length < 2) return null;
  return (r[0]?.p ?? 0) - (r[1]?.p ?? 0);
};

// ------------------------------------------------------- 1. do the two stages agree?

const passed = ids.filter(gatePasses);
let same = 0;
const flips = [];
for (const id of passed) {
  const a = routeWinner(id);
  const b = fitWinner(id);
  if (a === b) same += 1;
  else flips.push(id);
}

console.log("=== 两次调用的一致程度（只看门控放行的题）===");
console.log(`  放行 ${passed.length} 道：两次调用选出同一个第一名 ${same} 道 = ${pct(same, passed.length)}`);
console.log(`  不一致 ${flips.length} 道\n`);

const flipRows = flips.map((id) => {
  const g = goldFirst(id);
  const a = routeWinner(id);
  const b = fitWinner(id);
  const outcome = !g ? "该拒答（两边都错）" : b === g ? "复核改对" : a === g ? "复核改错" : "两边都错";
  return { id, gold: g, stage1: a, stage2: b, s1p: top1p(id), s1f: fitOf(id, a), s2f: fitOf(id, b), outcome };
});

console.log("  题号   金标题              路由第一名            复核第一名            路由p  路由fit 复核fit  结果");
for (const r of flipRows) {
  console.log(
    `  ${r.id}  ${String(r.gold ?? "（拒答）").padEnd(20)}${String(r.stage1).padEnd(22)}${String(r.stage2).padEnd(22)}` +
      `${f2(r.s1p).padStart(5)}  ${f2(r.s1f).padStart(6)}  ${f2(r.s2f).padStart(6)}  ${r.outcome}`,
  );
}

const helped = flipRows.filter((r) => r.outcome === "复核改对").length;
const hurt = flipRows.filter((r) => r.outcome === "复核改错").length;
console.log(`\n  → 复核改对 ${helped} 道、改错 ${hurt} 道、两边都错 ${flipRows.length - helped - hurt} 道`);
console.log(`  → 净收益 +${helped - hurt} 道（占 65 道有答案题的 ${pct(helped - hurt, 65)}）`);

// ------------------------------------------------------- 2. can stage 2 be skipped?

console.log("\n=== 能不能靠路由自己的把握度省掉第二次调用 ===");
console.log("  路由 top-1 概率区间      题数   其中复核会改判   改判后复核是对的");
const bands = [
  [0, 0.5],
  [0.5, 0.7],
  [0.7, 0.85],
  [0.85, 0.95],
  [0.95, 1.01],
];
for (const [lo, hi] of bands) {
  const inBand = passed.filter((id) => top1p(id) !== null && top1p(id) >= lo && top1p(id) < hi);
  const flipped = inBand.filter((id) => routeWinner(id) !== fitWinner(id));
  const flippedToRight = flipped.filter((id) => goldFirst(id) && fitWinner(id) === goldFirst(id));
  console.log(
    `  ${`[${lo.toFixed(2)}, ${hi.toFixed(2)})`.padEnd(22)}${String(inBand.length).padStart(5)}   ${String(flipped.length).padStart(12)}   ${String(flippedToRight.length).padStart(14)}`,
  );
}

// Same question, asked the other way: if we trusted stage 1 whenever it is confident, how many
// answers would we get right, and how much of stage 2 would we skip? A gate refusal stays wrong
// either way, so it counts as wrong here.
console.log("\n  假设「路由 top-1 ≥ T 就直接采用，不再复核」：");
console.log("       T    跳过的题   最终首答正确");
for (const T of [0.7, 0.8, 0.9, 0.95, 0.98, 1.01]) {
  let correct = 0;
  let skipped = 0;
  for (const id of answerable) {
    const g = goldFirst(id);
    if (!gatePasses(id)) continue; // refused: wrong unless the gold answer was itself a refusal
    if (top1p(id) !== null && top1p(id) >= T) {
      skipped += 1;
      if (routeWinner(id) === g) correct += 1;
    } else if (finalOf(id) === g) {
      correct += 1;
    }
  }
  const skippedPct = pct(skipped, passed.length);
  console.log(`    ${T.toFixed(2)}    ${String(skipped).padStart(7)} (${skippedPct.padStart(5)})    ${String(correct).padStart(2)}/65 = ${pct(correct, 65)}`);
}

// ------------------------------------------------------- 3. the errors

const wrong = ids.filter((id) => {
  const g = goldOf(id);
  const p = pipeline.get(id)?.answer ?? [];
  if (!g.length) return p.length > 0; // false suggestion
  return p[0] !== g[0];
});

console.log(`\n\n=== 最终答错 ${wrong.length} 道，逐道解剖 ===`);
for (const id of wrong) {
  const g = goldOf(id);
  const passedGate = gatePasses(id);
  const a = routeWinner(id);
  const b = fitWinner(id);
  const ranked = [...fits(id)].sort((x, y) => (y.fit ?? 0) - (x.fit ?? 0));
  console.log(`\n──── ${id} ────`);
  console.log(`  题面: ${textOf.get(id).slice(0, 150)}`);
  console.log(`  金标题: ${g.length ? g.join(", ") : "（拒答）"}`);
  console.log(`  路由:   ${route(id).slice(0, 3).map((e) => `${e.name} p=${f2(e.p)}`).join(" · ")}`);
  if (passedGate) {
    console.log(`  复核:   ${ranked.map((c) => `${c.name} fit=${f2(c.fit)}`).join(" · ")}  阈值 ${FITS_THRESHOLD}`);
  } else {
    console.log(`  复核:   （未执行，门控 none=${f2(pipeline.get(id)?.gate?.noneProbability)} ≥ ${NONE_GATE_THRESHOLD}）`);
  }
  // Where the gold answer actually sat, so the failure can be named.
  const goldAt = route(id).findIndex((e) => e.name === g[0]) + 1;
  const emptyRoster = roster.byName.get(g[0]);
  console.log(
    `  金标题在路由里的位置: ${g.length ? (goldAt ? `第 ${goldAt} 名` : "未进前 3（前 3 名分别是 " + route(id).map((e) => e.name).join(", ") + "）") : "—"}；` +
      `金标题 fit=${f2(fitOf(id, g[0]))}`,
  );
  if (emptyRoster) console.log(`  金标题技能的 description: ${emptyRoster.description.slice(0, 170)}`);
}

// ------------------------------------------------------- 4. what the runner-up missed by

console.log("\n\n=== 错误离正确有多远（只看首答错的题）===");
for (const id of wrong.filter((i) => goldOf(i).length > 0)) {
  const g = goldFirst(id);
  const b = fitWinner(id);
  const bf = fitOf(id, b);
  const gf = fitOf(id, g);
  console.log(`  ${id}: 先选了 ${b}(fit ${f2(bf)})，金标题 ${g}(fit ${f2(gf)})  差 ${gf !== null && bf !== null ? (bf - gf).toFixed(2) : "—"}`);
}
console.log("\n（反例：所有答对且有复核的题里，赢家与第二名的最小 fit 间距）");
const correctMargins = answerable
  .filter((id) => gatePasses(id) && finalOf(id) === goldFirst(id))
  .map((id) => {
    const ranked = [...fits(id)].sort((x, y) => (y.fit ?? 0) - (x.fit ?? 0));
    return (ranked[0]?.fit ?? 0) - (ranked[1]?.fit ?? 0);
  })
  .sort((a, b) => a - b);
console.log(`  最小的 5 个间距: ${correctMargins.slice(0, 5).map((x) => x.toFixed(2)).join(", ")}`);
console.log(`  中位间距: ${(correctMargins[Math.floor(correctMargins.length / 2)] ?? 0).toFixed(2)}`);
