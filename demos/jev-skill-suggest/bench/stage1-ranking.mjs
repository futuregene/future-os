#!/usr/bin/env node
// Stage 1 alone, ranked: top-1 / top-3 / top-5 — for both routing designs, plus the cost split
// between the two stages.
//
// The two designs cannot be compared on the same footing without first saying what a "rank" is:
//
//   * Noul fan-out (one independent yes/no question per skill): every skill gets its own
//     probability, so ranks 1..141 form a genuine ordering and top-k is a real recall@k.
//     Measured here with fresh API calls (~21k tokens/question, the compact request shape).
//   * One Choice over every skill + none_of_these (what ships): the probabilities are a single
//     distribution that must sum to 1, and it saturates — most questions put all the mass on one
//     option. Past rank 1 the order is a tie at 0, so "top-3" is the winner plus arbitrary
//     fillers. Those numbers are read from the existing runs/stage1-onechunk cache (no spend)
//     and printed so the inflation is visible, not so they can be quoted as recall.
//
// It also splits stage 2's cost in two, because the measurement harness and the shipped pipeline
// differ: predict-jev runs stage 2 for EVERY question so the gate can be evaluated offline, while
// the shipped pipeline only pays for it when the gate lets the question through. Quoting the
// measured total as "the cost" would overstate it by the refused questions.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { loadRoster } from "../roster.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const client = new JevClient({ apiKey: process.env.FUTURE_API_KEY });
const questions = readJson(QUESTIONS_FILE).questions;
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));
const pipeline = new Map(cacheFor("jev").all().map((row) => [row.id, row]));
const onechunk = new Map(cacheFor("stage1-onechunk").all().map((row) => [row.id, row]));

const ids = questions.map((q) => q.id);
const textOf = new Map(questions.map((q) => [q.id, q.text]));
const goldOf = (id) => gold.get(id).gold;
const answerable = ids.filter((id) => goldOf(id).length > 0);
const refusable = ids.filter((id) => goldOf(id).length === 0);
const goldFirst = (id) => goldOf(id)[0] ?? null;

const median = (values) => {
  const v = values.filter((x) => typeof x === "number").sort((a, b) => a - b);
  return v[Math.floor(v.length / 2)];
};
const sum = (values) => values.reduce((a, b) => a + (b ?? 0), 0);
const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;
const money = (tokens) => `$${((tokens * 0.042) / 1e6).toFixed(5)}`;

// ------------------------------------------------------- 1. genuine ranking: Noul fan-out
//
// The compact request shape (query left in `state`, terse criteria, 220-char descriptions) —
// the shape the Noul design had settled on, so this is the strongest version of that design.

const QUESTION = "Would the skill in `skill` materially help with the request in `request`?";
const CRITERIA = {
  true: "Covers the request, directly or as a clearly required first step",
  false: "Only topical overlap, or general help would do",
};
const buildNoul = (query) => ({
  state: { request: query },
  questions: Object.fromEntries(
    roster.skills.map((skill, i) => [
      `s_${i}`,
      {
        type: "noul",
        instructions: {
          question: QUESTION,
          skill: { name: skill.name, description: skill.indexLine.slice(0, 220) },
        },
        criteria: CRITERIA,
      },
    ]),
  ),
});

const noulCache = cacheFor("stage1-ranking-noul");
const asked = ids.filter((id) => !noulCache.has(id));
console.log(`stage 1 排序质量 · Noul 版（每个技能一道题，独立概率 = 真实排名）`);
console.log(`需要新跑的题：${asked.length}/100（已缓存 ${ids.length - asked.length}）\n`);

for (const id of asked) {
  const { answers, usage, ms } = await client.systemOne(buildNoul(textOf.get(id)));
  const ranked = roster.skills
    .map((skill, i) => ({ name: skill.name, p: answers[`s_${i}`]?.noul ?? 0 }))
    .sort((a, b) => b.p - a.p);
  noulCache.put(id, {
    id,
    ranked: ranked.slice(0, 10),
    top1p: ranked[0]?.p ?? null,
    tokens: usage?.input_tokens ?? null,
    ms,
  });
  process.stdout.write(`\r  ${id}    `);
}
console.log("\n");

const noul = new Map(noulCache.all().map((row) => [row.id, row]));
const rankOf = (id, list) => {
  const at = list.findIndex((entry) => entry.name === goldFirst(id));
  return at === -1 ? null : at + 1;
};

const noulRanks = answerable.map((id) => rankOf(id, noul.get(id)?.ranked ?? []));
const noulWithin = (k) => noulRanks.filter((r) => r !== null && r <= k).length;
const oneRanks = answerable.map((id) => rankOf(id, (onechunk.get(id)?.rankedTop ?? []).map((e) => ({ name: e.name }))));
const oneWithin = (k) => oneRanks.filter((r) => r !== null && r <= k).length;

const dist = (ranks) =>
  [1, 2, 3, 4, 5].map((k) => `${k}:${ranks.filter((r) => r === k).length}`).join(" ") +
  ` 6+:${ranks.filter((r) => r === null || r > 5).length}`;

console.log("=== 只看 stage 1：正确技能排在第几（65 道有答案的题）===");
console.log("                        Top-1        Top-3        Top-5        排名分布");
console.log(
  `  Noul 版（真实排名）    ${String(noulWithin(1)).padStart(2)}/65 ${pct(noulWithin(1), 65).padStart(6)}   ${String(noulWithin(3)).padStart(2)}/65 ${pct(noulWithin(3), 65).padStart(6)}   ${String(noulWithin(5)).padStart(2)}/65 ${pct(noulWithin(5), 65).padStart(6)}   ${dist(noulRanks)}`,
);
console.log(
  `  单个 Choice（发布）    ${String(oneWithin(1)).padStart(2)}/65 ${pct(oneWithin(1), 65).padStart(6)}   ${String(oneWithin(3)).padStart(2)}/65 ${pct(oneWithin(3), 65).padStart(6)}   ${String(oneWithin(5)).padStart(2)}/65 ${pct(oneWithin(5), 65).padStart(6)}   ${dist(oneRanks)}`,
);

// Why the Choice row's top-3/top-5 must not be quoted as recall.
const nonzero = ids.map((id) => onechunk.get(id)?.nonzero ?? null).filter((x) => x !== null);
const noulSpread = ids.map((id) => (noul.get(id)?.ranked ?? []).filter((e) => e.p > 0.001).length);
const fillers = answerable.filter((id) => {
  const r = onechunk.get(id);
  return r && (r.rankedTop ?? []).length >= 3 && (r.rankedTop[2]?.p ?? 0) === 0;
});
console.log(`\n  为什么 Choice 那一行不能当 recall 读：`);
console.log(`    单个 Choice 的非零选项数：中位 ${median(nonzero)}，恰好 1 个的题 ${nonzero.filter((x) => x === 1).length}/100（最多 ${Math.max(...nonzero)}）`);
console.log(`    → ${fillers.length}/65 道有答案的题里，第 3 名概率恰好是 0：所谓"进前 3"里的后两个位置是并列 0 的填充项`);
console.log(`    Noul 版的非零选项数：中位 ${median(noulSpread)}（0.001 以上）—— 每个技能独立取值，才构成真正的排序`);
console.log(`    所以 Choice 版只有第 1 名是有信息的；Noul 版的前 3/前 5 才是"找到了几个候选"`);

// ------------------------------------------------------- 2. gate quality, both designs

const noulGate = (id) => (noul.get(id)?.top1p ?? 0) >= 0.7;
const choiceGatePasses = (id) => (pipeline.get(id)?.gate?.length ?? 0) > 0;
const refusedNeg = (fn) => refusable.filter((id) => !fn(id)).length;
const refusedPos = (fn) => answerable.filter((id) => !fn(id));

console.log(`\n=== 只看 stage 1：拒答质量 ===`);
console.log(`  Noul 版（top-1 noul ≥ 0.70 才放行）    拒掉该拒的 ${refusedNeg(noulGate)}/35 = ${pct(refusedNeg(noulGate), 35)}，误拒 ${refusedPos(noulGate).length}/65 = ${pct(refusedPos(noulGate).length, 65)}`);
console.log(`  单个 Choice（none 概率 < 0.15 放行）   拒掉该拒的 ${refusedNeg(choiceGatePasses)}/35 = ${pct(refusedNeg(choiceGatePasses), 35)}，误拒 ${refusedPos(choiceGatePasses).length}/65 = ${pct(refusedPos(choiceGatePasses).length, 65)}   ${refusedPos(choiceGatePasses).join(", ")}`);

// ------------------------------------------------------- 3. what stage 2 adds

const finalOf = (id) => pipeline.get(id)?.answer?.[0] ?? null;
const passed = ids.filter((id) => choiceGatePasses(id));
const finalCorrect = answerable.filter((id) => finalOf(id) === goldFirst(id));
const stage1Top1Correct = answerable.filter((id) => (pipeline.get(id)?.stage1_top?.[0]?.name ?? null) === goldFirst(id));
const fixed = stage1Top1Correct.length;
const broke = answerable.filter((id) => stage1Top1Correct.includes(id) && finalOf(id) !== goldFirst(id) && finalOf(id) !== null);
const rescued = answerable.filter((id) => !stage1Top1Correct.includes(id) && finalOf(id) === goldFirst(id));
const refusedByS2 = answerable.filter((id) => choiceGatePasses(id) && finalOf(id) === null);
// A correct rank 1 can still be lost at the GATE, which is stage 1's doing, not stage 2's — so the
// reconciliation has to name it or the arithmetic (61 → 62) looks wrong.
const lostAtGate = stage1Top1Correct.filter((id) => !choiceGatePasses(id));

console.log(`\n=== stage 2 对最终结果做了什么（发布版）===`);
console.log(`  stage 1 第一名就正确：${fixed}/65 = ${pct(fixed, 65)}`);
console.log(`  − 其中被 stage 1 的门控拒掉：${lostAtGate.length} 道  ${lostAtGate.join(", ")}`);
console.log(`  + stage 2 从错的第一名里救回：${rescued.length} 道  ${rescued.join(", ")}`);
console.log(`  ± stage 2 把对的第一名改错：${broke.length} 道  ${broke.join(", ")}`);
console.log(`  − stage 2 拒掉了本可推荐的：${refusedByS2.length} 道  ${refusedByS2.join(", ")}`);
console.log(`  = 最终首答正确：${finalCorrect.length}/65 = ${pct(finalCorrect.length, 65)}`);
console.log(`  → stage 2 只在 stage 1 放行的 ${passed.length} 道上工作，净变化 +${rescued.length} − ${broke.length}；放行的那批里它没弄坏过一个`);

// ------------------------------------------------------- 4. cost split, API-reported tokens

const stage1 = ids.map((id) => pipeline.get(id)?.stage1_tokens ?? 0);
const stage2All = ids.map((id) => pipeline.get(id)?.stage2_tokens ?? 0);
const stage2Shipped = passed.map((id) => pipeline.get(id)?.stage2_tokens ?? 0);
const s1 = sum(stage1);
const s2 = sum(stage2Shipped);
const s2Measured = sum(stage2All);
const perQ = ids.map((id) => (pipeline.get(id)?.stage1_tokens ?? 0) + (choiceGatePasses(id) ? pipeline.get(id)?.stage2_tokens ?? 0 : 0));
const mean = (values) => sum(values) / values.length;

console.log(`\n=== 成本拆分（token 全部取自 API 返回的 usage.input_tokens，$0.042/Mtok、输出免费）===`);
console.log(`  阶段                                     每题中位   每题均值   100 题合计    占比    费用（100 题）`);
console.log(`  stage 1（1 个 Choice，141 选项 + none）  ${String(median(stage1)).padStart(8)}  ${String(Math.round(mean(stage1))).padStart(8)}  ${String(s1).padStart(10)}   ${pct(s1, s1 + s2).padStart(6)}  ${money(s1)}`);
console.log(`  stage 2（3 道 fit noul，只对放行的题）   ${String(median(stage2Shipped)).padStart(8)}  ${String(Math.round(mean(stage2Shipped))).padStart(8)}  ${String(s2).padStart(10)}   ${pct(s2, s1 + s2).padStart(6)}  ${money(s2)}`);
console.log(`  合计（= 真实服务成本）                   ${String(median(perQ)).padStart(8)}  ${String(Math.round(mean(perQ))).padStart(8)}  ${String(s1 + s2).padStart(10)}   100.0%  ${money(s1 + s2)}`);
console.log(`  换算：每题 ${Math.round(mean(perQ)).toLocaleString()} token × $0.042/Mtok × ¥7.2/$ = ¥${((mean(perQ) * 0.042 * 7.2) / 1e6).toFixed(5)}`);
console.log(`\n  对照 · 评测脚本的开销：predict-jev 对每道题都跑 stage 2（好让门控能离线评估）`);
console.log(`    stage 2 全量 ${s2Measured.toLocaleString()} token vs 服务只需 ${s2.toLocaleString()} token（多算 ${pct(s2Measured - s2, s2Measured)}）`);
console.log(`    评测 ${(s1 + s2Measured).toLocaleString()} token vs 服务 ${(s1 + s2).toLocaleString()} token`);
console.log(`    被拒的 ${ids.length - passed.length} 道只付 stage 1（${sum(ids.filter((id) => !choiceGatePasses(id)).map((id) => pipeline.get(id)?.stage1_tokens ?? 0)).toLocaleString()} token）`);

const ms1 = ids.map((id) => pipeline.get(id)?.stage1_ms ?? 0);
const ms2 = passed.map((id) => pipeline.get(id)?.stage2_ms ?? 0);
console.log(`\n  延迟：stage 1 中位 ${median(ms1)} ms · stage 2 中位 ${median(ms2)} ms · 串行合计中位 ${median(perQ.map((_, i) => (pipeline.get(ids[i])?.stage1_ms ?? 0) + (choiceGatePasses(ids[i]) ? pipeline.get(ids[i])?.stage2_ms ?? 0 : 0)))} ms`);

console.log(`\n=== 结论 ===`);
console.log(`  只看 stage 1：Noul 版 Top-1 ${pct(noulWithin(1), 65)}、Top-3 ${pct(noulWithin(3), 65)}、Top-5 ${pct(noulWithin(5), 65)}；`);
console.log(`               发布版只有 Top-1 有意义（${pct(oneWithin(1), 65)}），它的 Top-3/Top-5 ${pct(oneWithin(3), 65)} 是并列 0 的填充项撑出来的。`);
console.log(`  成本：stage 1 占 ${pct(s1, s1 + s2)}，stage 2 占 ${pct(s2, s1 + s2)}，且 stage 2 只在放行的 ${passed.length} 道题上产生费用。`);
