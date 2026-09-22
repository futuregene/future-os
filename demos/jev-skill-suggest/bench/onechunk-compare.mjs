#!/usr/bin/env node
// Compare "all in one chunk" (a single 141-option Choice with none) against the shipped chunked
// design, on the same 100 questions, with the same held-out protocol.
//
// Reads runs/stage1-onechunk and runs/stage1-chunked-raw-18.
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";

const questions = readJson(QUESTIONS_FILE).questions;
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));
const ids = questions.map((q) => q.id);
const goldOf = (id) => gold.get(id).gold;

const one = new Map(cacheFor("stage1-onechunk").all().map((row) => [row.id, row]));
const chunked = new Map(cacheFor("stage1-chunked-raw-18").all().map((row) => [row.id, row]));

const median = (values) => {
  const v = values.filter((x) => typeof x === "number").sort((a, b) => a - b);
  return v[Math.floor(v.length / 2)];
};
const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;

// ---------------------------------------------------------------- saturation

console.log("=== 饱和程度：一个 Choice 里有多少选项真的拿到概率 ===");
const nonzero = ids.map((id) => one.get(id).nonzero);
console.log(`  一个 Choice（141 选项 + none）：非零选项数 中位 ${median(nonzero)}，分布 ${[1, 2, 3, 5, 10].map((n) => `≤${n}: ${nonzero.filter((c) => c <= n).length}`).join("  ")}`);
console.log(`  前 3 名的概率：第一名中位 ${median(ids.map((id) => one.get(id).top1p)).toFixed(3)}，第二名中位 ${median(ids.map((id) => one.get(id).rankedTop[1]?.p ?? 0)).toFixed(3)}，第三名中位 ${median(ids.map((id) => one.get(id).rankedTop[2]?.p ?? 0)).toFixed(3)}`);
const allZero = ids.filter((id) => (one.get(id).rankedTop[1]?.p ?? 0) === 0).length;
console.log(`  第二、三名概率恰好为 0 的题：${allZero}/100 → 这些题的"前三名"其实是并列的 0`);

// ---------------------------------------------------------------- quality + cost

function wilson(successes, total) {
  const z = 1.96;
  const p = successes / total;
  const den = 1 + (z * z) / total;
  const centre = p + (z * z) / (2 * total);
  const spread = z * Math.sqrt((p * (1 - p)) / total + (z * z) / (4 * total * total));
  return [(centre - spread) / den, (centre + spread) / den];
}

function scoreConfig(rows, { pick, answer }, budget) {
  const answerOf = (id, t) => {
    const row = rows.get(id);
    if (!row) return null;
    if (pick === "none" && (row.noneProbability === null || row.noneProbability >= t)) return null;
    return answer(row);
  };
  const grid = [];
  for (let t = 0.05; t <= 0.95; t += 0.02) grid.push(Number(t.toFixed(2)));

  let first = 0;
  let positives = 0;
  let falseSuggest = 0;
  let negatives = 0;
  const picks = [];
  for (let fold = 0; fold < 5; fold += 1) {
    const train = ids.filter((_, i) => i % 5 !== fold);
    const test = ids.filter((_, i) => i % 5 === fold);
    let best = null;
    for (const t of grid) {
      let tc = 0;
      let tf = 0;
      for (const id of train) {
        const reference = goldOf(id);
        const a = answerOf(id, t);
        if (reference.length === 0) {
          if (a !== null) tf += 1;
        } else if (a !== null && a === reference[0]) tc += 1;
      }
      if (tf > budget) continue;
      if (!best || tc > best.tc) best = { t, tc };
    }
    if (!best) {
      picks.push("无解");
      continue;
    }
    picks.push(best.t);
    for (const id of test) {
      const reference = goldOf(id);
      const a = answerOf(id, best.t);
      if (reference.length === 0) {
        negatives += 1;
        if (a !== null) falseSuggest += 1;
      } else {
        positives += 1;
        if (a !== null && a === reference[0]) first += 1;
      }
    }
  }
  return { first, positives, falseSuggest, negatives, picks };
}

const oneTokens = median(ids.map((id) => one.get(id).tokens_total));
const chunkedTokens = median(ids.map((id) => chunked.get(id).tokens_total));

console.log(`\n=== 5 折留出对比（预算 1 = 允许 1 道误推，即发布版的工作点）===`);
console.log("  方案                                首答正确（留出）      95% CI            误推     每题 token   费用/题");
const rowsFor = [
  ["全部放一块（1 个 Choice，141 选项）", one, { pick: "none", answer: (row) => row.stage2Answer }, oneTokens],
  ["分块 18（8 块，发布）", chunked, { pick: "none", answer: (row) => row.stage2Answer }, chunkedTokens],
];
for (const [label, rows, cfg, tokens] of rowsFor) {
  const s = scoreConfig(rows, cfg, 1);
  const [low, high] = wilson(s.first, s.positives);
  console.log(
    `  ${label.padEnd(34)} ${String(s.first).padStart(2)}/${s.positives} = ${pct(s.first, s.positives).padStart(6)}      [${(low * 100).toFixed(1)}%, ${(high * 100).toFixed(1)}%]   ${String(s.falseSuggest).padStart(2)}/${s.negatives}    ${String(tokens).padStart(8)}   $${((tokens * 0.042) / 1e6).toFixed(5)}`,
  );
}

console.log(`\n=== 零误推预算（更严）===`);
for (const [label, rows, cfg, tokens] of rowsFor) {
  const s = scoreConfig(rows, cfg, 0);
  console.log(`  ${label.padEnd(34)} ${String(s.first).padStart(2)}/${s.positives} = ${pct(s.first, s.positives).padStart(6)}   误推 ${s.falseSuggest}/${s.negatives}   阈值 ${s.picks.join(",")}`);
}

console.log(`\n=== 候选质量：正确技能有没有进到交给复核的那三个里 ===`);
const withAnswer = ids.filter((id) => goldOf(id).length > 0);
for (const [label, rows] of [["全部放一块", one], ["分块 18", chunked]]) {
  const inTop3 = withAnswer.filter((id) => {
    const row = rows.get(id);
    const top3 = label === "全部放一块" ? row.top3 : row.finalTop3;
    return top3.includes(goldOf(id)[0]);
  }).length;
  console.log(`  ${label.padEnd(12)} ${inTop3}/${withAnswer.length} = ${pct(inTop3, withAnswer.length)}`);
}

console.log(`\n=== 结论 ===`);
const oneS = scoreConfig(one, { pick: "none", answer: (row) => row.stage2Answer }, 1);
const chunkS = scoreConfig(chunked, { pick: "none", answer: (row) => row.stage2Answer }, 1);
console.log(`  全部放一块：${oneS.first}/65 = ${pct(oneS.first, 65)}，${oneTokens} token/题`);
console.log(`  分块 18：  ${chunkS.first}/65 = ${pct(chunkS.first, 65)}，${chunkedTokens} token/题`);
console.log(`  差 ${oneS.first - chunkS.first} 道；成本 ${((oneTokens / chunkedTokens) * 100).toFixed(0)}%`);
