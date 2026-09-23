#!/usr/bin/env node
// Which stage-1 reading best predicts "stage 2 is going to overturn this"?
//
// The question comes from a cost angle: stage 2 costs 10.3% and, on 97% of the questions it sees, it
// only confirms stage 1. So can we decide *when to ask* it from stage 1's own output, and is the
// margin between rank 1 and rank 2 a better signal than rank 1's probability alone?
//
// A warning that shapes the whole analysis: the stage-1 Choice saturates. On 31 of the 66 passed
// questions the runner-up has probability exactly 0, so margin == top1 by construction and the two
// signals cannot differ there. Any comparison is decided by the other 35.
//
// The honest framing is not "which threshold gives the best accuracy" - all of them give the same
// accuracy, because the flips are only 2 questions and every rule here catches them. It is:
// **how many unnecessary stage-2 calls does each signal force, to be sure it catches the 2?**
// That is a coverage question, so the frontier below is reported as (verifications, flips caught).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, readJson } from "./common.mjs";
import { FITS_THRESHOLD } from "./second-call.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const questions = readJson(QUESTIONS_FILE).questions;

const readAll = (dir) =>
  fs
    .readdirSync(path.join(RUNS_DIR, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, dir, f), "utf8")));
const pipeline = new Map(readAll("jev").map((row) => [row.id, row]));
const gold = new Map(readAll("gold-v1").map((row) => [row.id, row.gold]));

const ids = questions.map((q) => q.id);
const goldOf = (id) => gold.get(id) ?? [];
const goldFirst = (id) => goldOf(id)[0] ?? null;
const answerable = ids.filter((id) => goldOf(id).length > 0);
const refusable = ids.filter((id) => goldOf(id).length === 0);

const rec = (id) => pipeline.get(id);
const gatePasses = (id) => (rec(id)?.gate?.length ?? 0) > 0;
const passed = ids.filter(gatePasses);

const top = (id) => rec(id)?.stage1_top ?? [];
const routeWinner = (id) => top(id)[0]?.name ?? null;
const fits = (id) => rec(id)?.stage2 ?? [];
const fitWinner = (id) => [...fits(id)].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0))[0]?.name ?? null;
const fitOf = (id, name) => fits(id).find((c) => c.name === name)?.fit ?? null;
const stage2Winner = (id) => {
  const ranked = [...fits(id)].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0));
  return ranked.length && ranked[0].fit >= FITS_THRESHOLD ? ranked[0].name : null;
};

// ---- the signals, all read from stage 1 only -----------------------------------------------
const signals = {
  top1: (id) => top(id)[0]?.p ?? 0,
  margin: (id) => (top(id)[0]?.p ?? 0) - (top(id)[1]?.p ?? 0),
  // Ratio is scale-free where the distribution does not saturate; included for completeness.
  ratio: (id) => {
    const p1 = top(id)[0]?.p ?? 0;
    const p2 = top(id)[1]?.p ?? 0;
    return p2 === 0 ? p1 : (p1 - p2) / p1;
  },
  nonzero: (id) => (rec(id)?.stage1_top ?? []).filter((e) => (e.p ?? 0) > 0).length,
};

// Where the two overturns sit, on each signal.
const flips = passed.filter((id) => routeWinner(id) !== fitWinner(id));
console.log(`=== 被复核改判的 ${flips.length} 道题，在三个信号上的取值 ===\n`);
console.log("  题号   金标题                路由第一名            top1   margin  ratio  非零");
for (const id of flips) {
  console.log(
    `  ${id}   ${String(goldFirst(id)).padEnd(20)} ${String(routeWinner(id)).padEnd(20)} ${signals.top1(id).toFixed(2).padStart(5)}  ${signals.margin(id).toFixed(2).padStart(5)}  ${signals.ratio(id).toFixed(2).padStart(5)}  ${String(signals.nonzero(id)).padStart(3)}`,
  );
}

// A rule that verifies when the signal is BELOW a threshold catches a flip iff the flip's value is
// below that threshold. So the cheapest rule that catches all flips sits just above the largest
// flip value, and its cost is the number of questions below that value.
console.log(`\n=== 每个信号：要抓住全部 ${flips.length} 道改判，至少要复核多少题 ===`);
console.log("  信号      改判题的最大值   最小可行阈值   需要复核的题数   略过的题");
const summary = {};
for (const [name, fn] of Object.entries(signals)) {
  const flipMax = Math.max(...flips.map(fn));
  // "Below threshold" rules: threshold must exceed every flip's value.
  const threshold = flipMax + (name === "nonzero" ? 0.5 : 0.005);
  const verify = passed.filter((id) => !(fn(id) >= threshold));
  summary[name] = { flipMax, threshold, verify: verify.length };
  console.log(
    `  ${name.padEnd(9)} ${flipMax.toFixed(2).padStart(8)}   ${threshold.toFixed(2).padStart(11)}   ${String(verify.length).padStart(12)}/66   ${String(66 - verify.length).padStart(8)}`,
  );
}

// The full frontier, so the trade is visible rather than just the single cheapest point.
console.log("\n=== 完整前沿：阈值 → 复核多少题 / 抓住几道改判 / 首答正确 ===");
for (const name of ["top1", "margin", "ratio"]) {
  const fn = signals[name];
  console.log(`\n  ${name} 阈值（低于则复核）   复核题数     抓住改判   首答正确`);
  const values = [...new Set(passed.map(fn))].sort((a, b) => a - b);
  for (const T of values) {
    const verify = passed.filter((id) => fn(id) < T);
    const caught = flips.filter((id) => fn(id) < T).length;
    // Score the resulting pipeline: verified questions take stage 2's answer, the rest take stage 1's.
    let correct = 0;
    for (const id of answerable) {
      if (!gatePasses(id)) continue;
      const answer = fn(id) < T ? stage2Winner(id) : routeWinner(id);
      if (answer === goldFirst(id)) correct += 1;
    }
    console.log(
      `          ${(T <= 0 ? "0" : T.toFixed(2)).padStart(6)}        ${String(verify.length).padStart(6)}/66     ${String(caught).padStart(7)}/2     ${String(correct).padStart(2)}/65 = ${((correct / 65) * 100).toFixed(1)}%`,
    );
  }
}

// An OR rule catches a flip if either signal is below its own threshold, so a pair of loose
// thresholds can catch more than either alone at the same cost. Worth checking directly.
console.log("\n=== 组合规则：复核条件 = top1 < T1 或 margin < T2 ===");
console.log("   T1     T2    复核题数   抓住改判   首答正确");
for (const T1 of [0.9, 0.95, 1.01]) {
  for (const T2 of [0.5, 0.65, 0.7, 0.8, 1.01]) {
    const verify = passed.filter((id) => signals.top1(id) < T1 || signals.margin(id) < T2);
    const caught = flips.filter((id) => signals.top1(id) < T1 || signals.margin(id) < T2).length;
    let correct = 0;
    for (const id of answerable) {
      if (!gatePasses(id)) continue;
      const answer = verify.includes(id) ? stage2Winner(id) : routeWinner(id);
      if (answer === goldFirst(id)) correct += 1;
    }
    console.log(
      `  ${T1.toFixed(2)}   ${T2.toFixed(2)}    ${String(verify.length).padStart(7)}/66     ${String(caught).padStart(5)}/2     ${String(correct).padStart(2)}/65 = ${((correct / 65) * 100).toFixed(1)}%`,
    );
  }
}

// ---- does the signal carry any information at all, beyond these two points? -------------------
// Rank the passed questions by each signal and look at where the flips fall: if they are spread
// among the most-confident questions, the signal cannot be trusted to find them.
console.log(`\n=== ${flips.length} 道改判在这两个信号里的排位（分位越低=模型越没把握）`);
for (const name of ["top1", "margin"]) {
  const fn = signals[name];
  const ranked = [...passed].sort((a, b) => fn(a) - fn(b));
  const ranks = flips.map((id) => {
    const at = ranked.findIndex((x) => x === id);
    return `${id}=第 ${at + 1}/${ranked.length}（${((at / ranked.length) * 100).toFixed(0)} 分位）`;
  });
  console.log(`  ${name.padEnd(8)} ${ranks.join("   ")}`);
}

console.log("\n=== 结论 ===");
const best = Object.entries(summary).sort((a, b) => a[1].verify - b[1].verify)[0];
for (const [name, s] of Object.entries(summary)) {
  console.log(`  用 ${name} 作信号：最少要复核 ${s.verify}/66 道（阈值 ${s.threshold.toFixed(2)}）`);
}
console.log(`  → ${best[0]} 最省（${best[1].verify}/66）。`);
