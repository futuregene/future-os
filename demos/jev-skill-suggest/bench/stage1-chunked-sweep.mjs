#!/usr/bin/env node
// Is D' (chunked Choice + a final none-gated Choice) genuinely better than the shipped pipeline,
// or did its gate just get lucky at 0.5? The chunked run cached the final none probability and
// the stage-2 answer for every question, so every threshold is free to evaluate here.
//
// Also sweeps the shipped pipeline's own gate over the same questions, so the comparison is
// tuned-vs-tuned rather than tuned-vs-guessed.
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";

const questions = readJson(QUESTIONS_FILE).questions;
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));
const chunked = new Map(cacheFor(process.env.CHUNK_RUN ?? "stage1-chunked-18").all().map((row) => [row.id, row]));
const shipped = new Map(cacheFor("jev").all().map((row) => [row.id, row]));

const ids = questions.map((q) => q.id);
const goldOf = (id) => gold.get(id).gold;

const score = (answerOf) => {
  let firstCorrect = 0;
  let correctRefusal = 0;
  let falseRefusal = 0;
  let falseSuggest = 0;
  let agreement = 0;
  for (const id of ids) {
    const reference = goldOf(id);
    const answer = answerOf(id);
    if (reference.length === 0) {
      if (answer === null) {
        correctRefusal += 1;
        agreement += 1;
      } else falseSuggest += 1;
    } else if (answer === null) falseRefusal += 1;
    else if (answer === reference[0]) {
      firstCorrect += 1;
      agreement += 1;
    }
  }
  return { firstCorrect, correctRefusal, falseRefusal, falseSuggest, agreement };
};

const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;

// ---------------------------------------------------------------- D' gate sweep

/** Stage-2 answer from the chunked run (before any gate is applied). */
const chunkedAnswer = (id) => chunked.get(id).answer_D_byfinal ?? null;
const rawStage2 = (id) => {
  const row = chunked.get(id);
  // answer_D_byfinal is null when the final none was >= 0.5; recover the ungated answer by
  // re-deriving from the recorded top-3 through the same gate rules the run used.
  return row.answer_D_byfinal !== null ? row.answer_D_byfinal : row.finalTop3[0] ?? null;
};
void rawStage2;

console.log("=== D'：用最终 Choice 的 none 概率当门控，扫阈值（stage 2 不变）===");
console.log("  阈值    首答正确        正确拒答       误拒  误推  总一致");
const sweep = [];
for (const threshold of [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]) {
  const s = score((id) => {
    const row = chunked.get(id);
    // Refuse when the final Choice put at least this much probability on "none".
    if (row.finalNoneProbability !== null && row.finalNoneProbability >= threshold) return null;
    if (row.finalNoneProbability === null) return null; // every chunk declined -> no candidates
    return row.answer_D_byfinal ?? null;
  });
  sweep.push({ threshold, ...s });
  console.log(
    `  ${threshold.toFixed(1)}    ${String(s.firstCorrect).padStart(2)}/65 = ${pct(s.firstCorrect, 65).padStart(6)}  ` +
      `${String(s.correctRefusal).padStart(2)}/35 = ${pct(s.correctRefusal, 35).padStart(6)}  ${String(s.falseRefusal).padStart(3)}  ${String(s.falseSuggest).padStart(3)}  ${String(s.agreement).padStart(2)}/100`,
  );
}
const bestChunked = sweep.reduce((a, b) => (b.firstCorrect > a.firstCorrect ? b : a));
console.log(`  最优阈值 ${bestChunked.threshold}（首答正确 ${bestChunked.firstCorrect}/65，误拒 ${bestChunked.falseRefusal}，误推 ${bestChunked.falseSuggest}）`);

// ---------------------------------------------------------------- shipped gate sweep

console.log(`\n=== 对照：现在发布的流水线，同样扫它的门控阈值（100 题）===`);
console.log("  阈值    首答正确        正确拒答       误拒  误推  总一致");
const shippedSweep = [];
const maxFit = (row) => (row.stage2 ? Math.max(...row.stage2.map((c) => c.fit ?? 0)) : 0);
const topFitName = (row) => [...(row.stage2 ?? [])].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0))[0]?.name ?? null;
for (const gate of [0.55, 0.6, 0.65, 0.7, 0.75, 0.8]) {
  const s = score((id) => {
    const row = shipped.get(id);
    if (row.top1_noul < gate) return null;
    if (!row.stage2) return row.gate?.[0] ?? null;
    return maxFit(row) >= 0.75 ? topFitName(row) : null;
  });
  shippedSweep.push({ gate, ...s });
  console.log(
    `  ${gate.toFixed(2)}   ${String(s.firstCorrect).padStart(2)}/65 = ${pct(s.firstCorrect, 65).padStart(6)}  ` +
      `${String(s.correctRefusal).padStart(2)}/35 = ${pct(s.correctRefusal, 35).padStart(6)}  ${String(s.falseRefusal).padStart(3)}  ${String(s.falseSuggest).padStart(3)}  ${String(s.agreement).padStart(2)}/100`,
  );
}
const bestShipped = shippedSweep.reduce((a, b) => (b.firstCorrect > a.firstCorrect ? b : a));
console.log(`  最优阈值 ${bestShipped.gate}（首答正确 ${bestShipped.firstCorrect}/65，误拒 ${bestShipped.falseRefusal}，误推 ${bestShipped.falseSuggest}）`);

// ---------------------------------------------------------------- fair comparison

console.log(`\n=== 调优后的正面对比（两边都用各自的最优阈值）===`);
const tokensShipped = 21357;
const tokensChunked = 11402;
console.log(`  现在发布：首答正确 ${bestShipped.firstCorrect}/65 = ${pct(bestShipped.firstCorrect, 65)}，总一致 ${bestShipped.agreement}/100，${tokensShipped} token/题（$${((tokensShipped * 0.042) / 1e6).toFixed(5)}）`);
console.log(`  分块+none：首答正确 ${bestChunked.firstCorrect}/65 = ${pct(bestChunked.firstCorrect, 65)}，总一致 ${bestChunked.agreement}/100，${tokensChunked} token/题（$${((tokensChunked * 0.042) / 1e6).toFixed(5)}）`);
console.log(`  → 分块方案${bestChunked.firstCorrect > bestShipped.firstCorrect ? "更准" : bestChunked.firstCorrect === bestShipped.firstCorrect ? "打平" : "更差"}，成本 ${((tokensChunked / tokensShipped) * 100).toFixed(0)}%`);

// ---------------------------------------------------------------- per-question disagreement

console.log(`\n=== 逐题差异（各自最优阈值）===`);
const finalOf = (id) => {
  const row = chunked.get(id);
  if (row.finalNoneProbability === null || row.finalNoneProbability >= bestChunked.threshold) return null;
  return row.answer_D_byfinal ?? null;
};
const shippedOf = (id) => {
  const row = shipped.get(id);
  if (row.top1_noul < bestShipped.gate) return null;
  if (!row.stage2) return row.gate?.[0] ?? null;
  return maxFit(row) >= 0.75 ? topFitName(row) : null;
};
const differs = ids.filter((id) => finalOf(id) !== shippedOf(id));
let chunkedWins = 0;
let shippedWins = 0;
for (const id of differs) {
  const reference = goldOf(id)[0] ?? null;
  const c = finalOf(id);
  const s = shippedOf(id);
  if (c === reference) chunkedWins += 1;
  if (s === reference) shippedWins += 1;
}
console.log(`  不同 ${differs.length} 道：分块对 ${chunkedWins}、现在对 ${shippedWins}`);
for (const id of differs.slice(0, 10)) {
  const reference = goldOf(id)[0] ?? "拒答";
  console.log(`    ${id}: 分块=${finalOf(id) ?? "拒答"}  现在=${shippedOf(id) ?? "拒答"}  参照=${reference}`);
}
