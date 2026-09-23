#!/usr/bin/env node
// Scores every system against the gold answers — TOP-1 ONLY.
//
// Systems:
//   jev            stage 1 alone: 141 parallel Nouls, take the highest, refuse under the gate
//   jev+choice     stage 1, then stage 2; the three candidates ordered by the stage-2 Choice
//   jev+fit        same stage 2 response, candidates ordered by the stage-2 "does it really
//                  fit" Noul instead  ← what the demo ships
//   embed          omlx Qwen3-Embedding-0.6B cosine, refusal by threshold
//   embed-top1     pure nearest neighbour, never refuses
//   llm            deepseek-flash, same prompt as the gold
//
// Only the first name each system returns counts, plus whether it refused at all; recall of
// the gold's 2nd and 3rd names is ignored on purpose.
//
// The embedding threshold is fitted on the even half and reported on the odd half, because a
// cosine score cannot refuse by itself and any threshold picked on all 100 would be optimistic.
//
//   node score.mjs
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, ROSTER_FILE, cacheFor, readJson, writeJson } from "./common.mjs";
import { FITS_THRESHOLD } from "./second-call.mjs";
import { USD_PER_MTOK_INPUT, USD_TO_CNY } from "../jev.mjs";

const questions = readJson(QUESTIONS_FILE).questions;
const roster = readJson(ROSTER_FILE).skills;
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));
const jev = new Map(cacheFor("jev").all().map((row) => [row.id, row]));
const embedRows = new Map(cacheFor("embed").all().map((row) => [row.id, row]));
const llm = new Map(cacheFor("llm").all().map((row) => [row.id, row]));

const missing = questions.filter((q) => !gold.has(q.id) || !jev.has(q.id) || !embedRows.has(q.id) || !llm.has(q.id));
if (missing.length) {
  console.error(`incomplete: ${missing.length} questions missing an answer: ${missing.slice(0, 5).map((q) => q.id).join(", ")}`);
  process.exit(1);
}

const goldOf = (id) => gold.get(id).gold;
const goldEmpty = questions.filter((q) => goldOf(q.id).length === 0).map((q) => q.id);
const goldFilled = questions.filter((q) => goldOf(q.id).length > 0).map((q) => q.id);
const provenance = questions.filter((q) => q.source_skill).map((q) => q.id);
const oddIds = questions.filter((_, i) => i % 2 === 1).map((q) => q.id);
const evenIds = questions.filter((_, i) => i % 2 === 0).map((q) => q.id);

const embedScored = Object.fromEntries(
  questions.map((q) => [q.id, embedRows.get(q.id).ranked.map((entry) => ({ name: entry.name, score: entry.score }))]),
);

const embedTop1At = (id, threshold) => {
  const best = embedScored[id][0];
  return best && best.score >= threshold ? [best.name] : [];
};

/** Top-1 agreement: same refusal, or same first skill. */
const agree = (pred, reference) => (reference.length === 0 ? pred.length === 0 : pred[0] === reference[0]);

const GRID = Array.from({ length: 51 }, (_, i) => Number((0.30 + i * 0.01).toFixed(2)));
let best = { threshold: 0.56, score: -1 };
for (const threshold of GRID) {
  const score = evenIds.filter((id) => agree(embedTop1At(id, threshold), goldOf(id))).length / evenIds.length;
  if (score > best.score) best = { threshold, score };
}
const T = best.threshold;
console.log(`embedding refusal threshold fitted on the even half: cosine >= ${T}  (${(best.score * 100).toFixed(0)}% top-1 agreement there)`);
console.log(`its all-100 row is optimistic; the held-out half is the honest number\n`);

const systems = {
  jev: {
    label: "Jev（发布：一次调用）",
    answer: (id) => jev.get(id).gate.slice(0, 1),
    cost: (id) => ({
      tokens: jev.get(id).stage1_tokens,
      usd: ((jev.get(id).stage1_tokens ?? 0) * USD_PER_MTOK_INPUT) / 1e6,
      ms: jev.get(id).stage1_ms,
    }),
  },
  "jev+stage2": {
    // NOT shipped: the same run, but answering from the REMOVED second call (highest fit Noul)
    // instead of the Choice's top-1. Kept as a comparison so the report can show what removing it
    // cost (REPORT §2.2.5) and so the decision stays reversible on evidence.
    label: "Jev（+复核，对照）",
    answer: (id) => {
      const ranked = [...(jev.get(id).stage2 ?? [])].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0));
      return jev.get(id).gate.length > 0 && (ranked[0]?.fit ?? 0) >= FITS_THRESHOLD ? [ranked[0].name] : [];
    },
    cost: (id) => {
      const row = jev.get(id);
      const paidStage2 = row.gate.length > 0;
      const tokens = (row.stage1_tokens ?? 0) + (paidStage2 ? row.stage2_tokens ?? 0 : 0);
      const ms = (row.stage1_ms ?? 0) + (paidStage2 ? row.stage2_ms ?? 0 : 0);
      return { tokens, usd: (tokens * USD_PER_MTOK_INPUT) / 1e6, ms };
    },
  },
  embed: {
    label: `embed（余弦 ≥ ${T}）`,
    answer: (id) => embedTop1At(id, T),
    cost: (id) => ({ tokens: embedRows.get(id).tokens ?? null, usd: 0, ms: embedRows.get(id).ms ?? null, local: true }),
  },
  "embed-top1": {
    label: "embed（最近邻，从不拒答）",
    answer: (id) => [embedScored[id][0].name],
    cost: (id) => ({ tokens: embedRows.get(id).tokens ?? null, usd: 0, ms: embedRows.get(id).ms ?? null, local: true }),
  },
  llm: {
    label: "deepseek-flash",
    answer: (id) => llm.get(id).answer.slice(0, 1),
    cost: (id) => ({
      tokens: llm.get(id).usage?.prompt_tokens ?? null,
      credits: llm.get(id).usage?.credit_cost ?? null,
      ms: llm.get(id).durationMs ?? null,
    }),
  },
};

const pct = (value) => (value === null || value === undefined ? "   —  " : `${(value * 100).toFixed(1).padStart(5)}%`);
const mean = (values) => (values.length ? values.reduce((sum, v) => sum + v, 0) / values.length : null);
const median = (values) => {
  const sorted = values.filter((v) => typeof v === "number").sort((a, b) => a - b);
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : null;
};

const evaluate = (system, ids) => {
  const rows = ids.map((id) => {
    const reference = goldOf(id);
    const pred = system.answer(id);
    const question = questions.find((q) => q.id === id);
    return {
      id,
      text: question.text,
      source_skill: question.source_skill ?? null,
      gold: reference,
      pred,
      agree: agree(pred, reference),
      top1: reference.length ? pred[0] === reference[0] : null,
      // Secondary metric: a question can have several defensible answers (12 of the 65 do), and
      // comparing only against reference[0] marks a prediction wrong when it is in the set. That
      // happens once on this set (p052: reference [datamol, rdkit], prediction rdkit). Reported
      // separately so the headline number stays comparable across systems.
      inReference: reference.length ? reference.includes(pred[0]) : null,
      refused: pred.length === 0,
      falseRefuse: reference.length > 0 && pred.length === 0,
      falseSuggest: reference.length === 0 && pred.length > 0,
      sourceTop1: question.source_skill ? pred[0] === question.source_skill : null,
    };
  });
  const on = (bucket) => rows.filter((r) => bucket.includes(r.id));
  const costs = ids.map((id) => system.cost(id));
  return {
    rows,
    agree: mean(rows.map((r) => Number(r.agree))),
    top1: mean(on(goldFilled).map((r) => Number(r.top1))),
    inReference: mean(on(goldFilled).map((r) => Number(r.inReference))),
    refusal: mean(on(goldEmpty).map((r) => Number(!r.falseSuggest))),
    falseRefuse: mean(on(goldFilled).map((r) => Number(r.falseRefuse))),
    sourceTop1: mean(on(provenance).map((r) => Number(r.sourceTop1))),
    tokens: median(costs.map((c) => c.tokens)),
    usd: median(costs.map((c) => (c.usd === undefined ? null : c.usd))),
    credits: median(costs.map((c) => (c.credits === undefined ? null : c.credits))),
    ms: median(costs.map((c) => c.ms)),
    msMean: mean(costs.map((c) => c.ms)),
    local: costs.some((c) => c.local) ?? false,
  };
};

const all = Object.fromEntries(Object.entries(systems).map(([key, system]) => [key, evaluate(system, questions.map((q) => q.id))]));
const heldOut = Object.fromEntries(Object.entries(systems).map(([key, system]) => [key, evaluate(system, oddIds)]));

const printTable = (title, results, ids) => {
  console.log(`=== ${title} (n=${ids.length}: ${ids.filter((id) => goldOf(id).length).length} gold-skill, ${ids.filter((id) => !goldOf(id).length).length} gold-refusal) ===`);
  console.log("  system                          top-1一致   首答对*  首答∈参照集   正确拒答^   误拒*");
  for (const key of Object.keys(systems)) {
    const r = results[key];
    console.log(
      `  ${systems[key].label.padEnd(30)} ${pct(r.agree)} ${pct(r.top1)} ${pct(r.inReference).padStart(9)} ${pct(r.refusal).padStart(9)} ${pct(r.falseRefuse)}`,
    );
  }
  console.log("  * 只在有金标题技能的题上算   ^ 只在金标题为空的题上算");
  console.log("  首答∈参照集：预测落在参照答案集合内（12/65 道题参照给了多个可选答案）\n");
};

printTable("全部 100 题", all, questions.map((q) => q.id));
printTable("留出的奇数半（embedding 阈值未在此半拟合）", heldOut, oddIds);

console.log("=== 出处标签（不是 LLM 判断，独立对照，65 题） ===");
for (const key of Object.keys(systems)) {
  console.log(`  ${systems[key].label.padEnd(30)} 首答 = 出题所用技能 ${pct(all[key].sourceTop1)}`);
}

// ------------------------------------------------------------------ cost & latency

const usd = (value) => (value === null || value === undefined ? "     —    " : `$${value.toFixed(5)}`);
console.log(`
=== 成本（每题中位） ===`);
console.log("  system                          输入tokens   费用/题      说明");
for (const key of Object.keys(systems)) {
  const r = all[key];
  const note = r.local
    ? "本地 omlx 自建（无 API 费用）"
    : r.credits !== null && r.credits !== undefined
      ? `平台计费 ${r.credits.toFixed(5)} credits/题`
      : "按 Jev 公示价 $42/Btok 自算";
  const money = r.local ? "$0（自建）" : usd(r.usd);
  console.log(`  ${systems[key].label.padEnd(30)} ${String(r.tokens ?? "—").padStart(9)}   ${money.padEnd(11)} ${note}`);
}

console.log(`\n=== 延迟（每题，一次完整作答） ===`);
console.log("  system                          中位      平均");
for (const key of Object.keys(systems)) {
  console.log(`  ${systems[key].label.padEnd(30)} ${String(all[key].ms ?? "—").padStart(6)} ms  ${String(Math.round(all[key].msMean ?? 0)).padStart(6)} ms`);
}

// per-question cost roll-ups for the whole 100
const total = (key, field) => questions.reduce((sum, q) => sum + (systems[key].cost(q.id)[field] ?? 0), 0);
console.log(`\n=== 100 题总计 ===`);
for (const key of Object.keys(systems)) {
  const usdTotal = total(key, "usd");
  const creditsTotal = total(key, "credits");
  const msTotal = total(key, "ms");
  const parts = [`${(msTotal / 1000).toFixed(1)} s 总延迟`];
  if (usdTotal) parts.push(`$${usdTotal.toFixed(4)}`);
  if (creditsTotal) parts.push(`${creditsTotal.toFixed(3)} credits`);
  if (all[key].local) parts.push("本地算力");
  console.log(`  ${systems[key].label.padEnd(30)} ${parts.join("  ·  ")}`);
}
const goldCredits = questions.reduce((sum, q) => sum + (gold.get(q.id).usage?.credit_cost ?? 0), 0);
console.log(`  ${"(金标题 kimi-k3)".padEnd(30)} ${goldCredits.toFixed(3)} credits  ·  中位 ${median(questions.map((q) => gold.get(q.id).durationMs))} ms`);

// ------------------------------------------------------------------ stage 2 flips

const flips = goldFilled.map((id) => {
  const row = jev.get(id);
  const ranked = [...(row.stage2 ?? [])].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0));
  return { id, before: row.answer[0] ?? null, after: ranked[0]?.fit >= FITS_THRESHOLD ? ranked[0].name : null, reference: goldOf(id)[0] };
});
console.log(`\n=== 被移除的第二次调用，当时对首答的影响（${goldFilled.length} 道有答案的题）===`);
const changed = flips.filter((f) => f.before !== f.after);
const fixed = changed.filter((f) => f.after === f.reference).length;
const broke = changed.filter((f) => f.after !== f.reference && f.before === f.reference).length;
console.log(`  stage 2 改了首答 ${changed.length} 次：修对 ${fixed}、改错 ${broke}、两个都错 ${changed.length - fixed - broke}`);
for (const flip of changed) {
  console.log(`    ${flip.id} ${flip.before} → ${flip.after}  gold ${JSON.stringify(goldOf(flip.id))}`);
}

// ------------------------------------------------------------------ gold stability

let stability = null;
try {
  const write = cacheFor("gold-v1-b");
  const ids = questions.map((q) => q.id);
  const sameFirst = ids.filter((id) => (gold.get(id).gold[0] ?? null) === (write.get(id)?.gold?.[0] ?? null));
  stability = { same_first: sameFirst.length, total: ids.length };
  console.log(`\n=== 金标题自身稳定性（两次独立跑） ===`);
  console.log(`  首答相同 ${sameFirst.length}/${ids.length}（另有 ${ids.length - sameFirst.length} 道在"给不给技能"上就翻了）`);
  for (const id of ids.filter((i) => !sameFirst.includes(i))) {
    console.log(`    ${id}：A=${JSON.stringify(gold.get(id).gold)} B=${JSON.stringify(write.get(id).gold)}`);
  }
} catch {
  /* second pass not cached */
}

// ------------------------------------------------------------------ disagreements

console.log(`\n=== 各自 top-1 错的题 ===`);
for (const key of Object.keys(systems)) {
  const rows = all[key].rows.filter((r) => !r.agree);
  console.log(`\n${systems[key].label}  错 ${rows.length}/100`);
  for (const row of rows.slice(0, 6)) {
    const kind = row.falseSuggest ? "该拒答却推" : row.falseRefuse ? "该推荐却拒" : "首答错";
    console.log(`   [${kind}] gold=${row.gold.join(",") || "-"} pred=${row.pred.join(",") || "-"} :: ${row.text.slice(0, 54)}`);
  }
}

writeJson(fileURLToPath(new URL("../dataset/score.json", import.meta.url)), {
  scored_at: new Date().toISOString(),
  metric: "top-1 only: refusal decision, or the first skill returned",
  gold_model: "future/kimi-k3",
  totals: { questions: questions.length, gold_empty: goldEmpty.length, gold_filled: goldFilled.length },
  embed_threshold: { value: T, fitted_on: "even half", agreement_on_fit_half: best.score },
  gold_stability: stability,
  summary: Object.fromEntries(
    Object.entries(systems).map(([key, system]) => [
      key,
      {
        label: system.label,
        all: { ...all[key], rows: undefined },
        held_out_odd_half: { ...heldOut[key], rows: undefined },
        totals_100: { ms: total(key, "ms"), usd: total(key, "usd"), credits: total(key, "credits") },
      },
    ]),
  ),
  per_question: Object.fromEntries(Object.entries(systems).map(([key, _]) => [key, all[key].rows])),
});
console.log("\nwrote dataset/score.json");
