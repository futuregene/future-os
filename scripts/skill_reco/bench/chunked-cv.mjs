// Chunk-size comparison under held-out validation.
//
// Two things this script used to get wrong, both from the implementation changing underneath it:
//   - it hard-coded the sizes it evaluated, so passing sizes on the command line did nothing;
//   - its "shipped" row read `top1_noul` from runs/jev, a field the chunked implementation no
//     longer records, so that row silently degraded into "no gate at all".
// The 141-Noul design is gone from the tree, so it is carried here as documented constants taken
// from the report rather than recomputed (its per-question data is no longer kept).
//
//   node chunked-cv.mjs [sizes...]        e.g. node chunked-cv.mjs 16 18 30 47
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";

const questions = readJson(QUESTIONS_FILE).questions;
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));
const ids = questions.map((q) => q.id);
const goldOf = (id) => gold.get(id).gold;

const FOLDS = 5;
const foldOf = new Map(ids.map((id, i) => [id, i % FOLDS]));
const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;

const sizes = process.argv.slice(2).map(Number).filter(Number.isFinite);
if (!sizes.length) sizes.push(16, 18, 30, 47);

/** The superseded 141-Noul design, from bench/REPORT.md §2.2.2 (same folds, same protocol). */
const HISTORICAL = {
  label: "（历史）141 道 noul",
  tokens: 21357,
  byBudget: {
    0: { firstCorrect: 40, positives: 65, falseSuggest: 2 },
    1: { firstCorrect: 62, positives: 65, falseSuggest: 2 },
    2: { firstCorrect: 62, positives: 65, falseSuggest: 2 },
  },
};

function score(idsIn, answerOf) {
  let firstCorrect = 0;
  let falseSuggest = 0;
  let falseRefusal = 0;
  let agreement = 0;
  let positives = 0;
  let negatives = 0;
  for (const id of idsIn) {
    const reference = goldOf(id);
    const answer = answerOf(id);
    if (reference.length === 0) {
      negatives += 1;
      if (answer === null) agreement += 1;
      else falseSuggest += 1;
    } else {
      positives += 1;
      if (answer === null) falseRefusal += 1;
      else if (answer === reference[0]) {
        firstCorrect += 1;
        agreement += 1;
      }
    }
  }
  return { firstCorrect, falseSuggest, falseRefusal, agreement, positives, negatives };
}

function wilson(successes, total) {
  const z = 1.96;
  const p = successes / total;
  const den = 1 + (z * z) / total;
  const centre = p + (z * z) / (2 * total);
  const spread = z * Math.sqrt((p * (1 - p)) / total + (z * z) / (4 * total * total));
  return [(centre - spread) / den, (centre + spread) / den];
}

function evaluateSize(size) {
  const rows = new Map(cacheFor(`stage1-chunked-raw-${size}`).all().map((row) => [row.id, row]));
  if (!rows.size) return null;

  const chunkCount = Math.ceil(141 / size);
  const answerOf = (id, p) => {
    const row = rows.get(id);
    if (!row) return null;
    if (row.finalNone === null || row.finalNone >= p.t) return null;
    if (p.useGate && row.needsSkill !== null && row.needsSkill < p.g) return null;
    return row.stage2Answer;
  };
  const grid = [];
  for (let t = 0.05; t <= 0.95; t += 0.02) {
    grid.push({ t: Number(t.toFixed(2)), useGate: false, family: "仅 none" });
    for (let g = 0.5; g <= 0.95; g += 0.05) {
      grid.push({ t: Number(t.toFixed(2)), g: Number(g.toFixed(2)), useGate: true, family: "none+gate" });
    }
  }

  const tokens = (() => {
    const values = [...rows.values()].map((row) => row.tokens_total).filter((v) => typeof v === "number").sort((a, b) => a - b);
    return values[Math.floor(values.length / 2)];
  })();

  const byBudget = {};
  for (const budget of [0, 1, 2]) {
    let first = 0;
    let positives = 0;
    let falseSuggest = 0;
    let negatives = 0;
    const picks = [];
    for (let fold = 0; fold < FOLDS; fold += 1) {
      const train = ids.filter((id) => foldOf.get(id) !== fold);
      const test = ids.filter((id) => foldOf.get(id) === fold);
      let best = null;
      for (const params of grid) {
        const s = score(train, (id) => answerOf(id, params));
        if (s.falseSuggest > budget) continue;
        if (!best || s.firstCorrect > best.s.firstCorrect) best = { params, s };
      }
      if (!best) {
        picks.push("无解");
        continue;
      }
      const onTest = score(test, (id) => answerOf(id, best.params));
      first += onTest.firstCorrect;
      positives += onTest.positives;
      falseSuggest += onTest.falseSuggest;
      negatives += onTest.negatives;
      picks.push(best.params.useGate ? `${best.params.t}/${best.params.g}` : `${best.params.t}`);
    }
    byBudget[budget] = { firstCorrect: first, positives, falseSuggest, negatives, picks };
  }
  return { size, chunkCount, tokens, byBudget };
}

const results = sizes.map(evaluateSize).filter(Boolean);
console.log(`5 折交叉验证：在 4 折上选操作点（限定误推预算），用到没见过的第 5 折`);
console.log(`题目 ${ids.length}（65 有答案 / 35 无答案）\n`);

console.log("=== 各分块大小的留出结果（括号里是调参折上的乐观值，便于看是否过拟合）===");
console.log("  分块大小   块数   每题 token   预算0（零误推）            预算1（=发布工作点）           预算2");
for (const r of results) {
  const cell = (budget) => {
    const b = r.byBudget[budget];
    return `${String(b.firstCorrect).padStart(2)}/65 = ${pct(b.firstCorrect, 65).padStart(6)}（误推 ${b.falseSuggest}）`;
  };
  console.log(
    `  ${String(r.size).padStart(6)}   ${String(r.chunkCount).padStart(3)}    ${String(r.tokens).padStart(9)}   ${cell(0).padEnd(28)} ${cell(1).padEnd(29)} ${cell(2)}`,
  );
}
console.log(
  `  ${HISTORICAL.label.padEnd(6)}   ${String(Math.ceil(141 / 141)).padStart(3)}    ${String(HISTORICAL.tokens).padStart(9)}   ` +
    `${String(HISTORICAL.byBudget[0].firstCorrect).padStart(2)}/65 = ${pct(HISTORICAL.byBudget[0].firstCorrect, 65).padStart(6)}（误推 ${HISTORICAL.byBudget[0].falseSuggest}）   ` +
    `${String(HISTORICAL.byBudget[1].firstCorrect).padStart(2)}/65 = ${pct(HISTORICAL.byBudget[1].firstCorrect, 65).padStart(6)}（误推 ${HISTORICAL.byBudget[1].falseSuggest}）  ` +
    `${String(HISTORICAL.byBudget[2].firstCorrect).padStart(2)}/65 = ${pct(HISTORICAL.byBudget[2].firstCorrect, 65).padStart(6)}`,
);

console.log(`\n=== 95% 置信区间（预算 1，留出）===`);
for (const r of results) {
  const b = r.byBudget[1];
  const [low, high] = wilson(b.firstCorrect, b.positives);
  console.log(`  分块 ${String(r.size).padStart(2)}（${r.chunkCount} 块）  ${pct(b.firstCorrect, b.positives).padStart(6)}  →  [${(low * 100).toFixed(1)}%, ${(high * 100).toFixed(1)}%]`);
}
{
  const [low, high] = wilson(HISTORICAL.byBudget[1].firstCorrect, HISTORICAL.byBudget[1].positives);
  console.log(`  ${HISTORICAL.label}  ${pct(HISTORICAL.byBudget[1].firstCorrect, 65).padStart(6)}  →  [${(low * 100).toFixed(1)}%, ${(high * 100).toFixed(1)}%]`);
}

console.log(`\n=== 各折挑出来的阈值（看稳定性）===`);
for (const r of results) {
  console.log(`  分块 ${String(r.size).padStart(2)}：预算0 [${r.byBudget[0].picks.join(", ")}]   预算1 [${r.byBudget[1].picks.join(", ")}]`);
}

console.log(`\n=== 结论 ===`);
const atBudget1 = results.map((r) => ({ size: r.size, chunkCount: r.chunkCount, tokens: r.tokens, ...r.byBudget[1] }));
const best = atBudget1.reduce((a, b) => (b.firstCorrect > a.firstCorrect ? b : a));
const cheapestTied = atBudget1.filter((r) => r.firstCorrect >= best.firstCorrect).sort((a, b) => a.tokens - b.tokens)[0];
console.log(`  预算 1 下最好的是分块 ${best.size}（${best.chunkCount} 块）：${best.firstCorrect}/65 = ${pct(best.firstCorrect, 65)}，${best.tokens} token/题，误推 ${best.falseSuggest}`);
for (const r of atBudget1) {
  const delta = r.firstCorrect - HISTORICAL.byBudget[1].firstCorrect;
  console.log(
    `  分块 ${String(r.size).padStart(2)}（${String(r.chunkCount).padStart(2)} 块）：${String(r.firstCorrect).padStart(2)}/65 = ${pct(r.firstCorrect, 65)}（相对历史 141 noul ${delta >= 0 ? "+" : ""}${delta} 道），token ${((r.tokens / HISTORICAL.tokens) * 100).toFixed(0)}%，误推 ${r.falseSuggest}`,
  );
}
console.log(`  成本最低且不输给最好成绩的是分块 ${cheapestTied.size}（${cheapestTied.chunkCount} 块，${cheapestTied.tokens} token/题）。`);
console.log(`  提醒：每折只有 13 道正例，这些差异大多在噪音内——结论应读作"没有观察到差异"。`);
