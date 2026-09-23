#!/usr/bin/env node
// Stage 1 is 95% of the cost, and most of it is duplication. Per skill, the request currently sends:
//
//   description                  ~60 tokens   the actual signal, different per skill
//   criteria (yes/no text)       ~56 tokens   IDENTICAL for all 141
//   question sentence            ~20 tokens   IDENTICAL for all 141
//   the user's query (`request`) ~10-60      IDENTICAL for all 141, and already in the shared `state`
//   JSON key names               ~28 tokens   IDENTICAL for all 141
//
// `shipped_exact` reproduces the shipped request byte for byte, so the token accounting is
// verifiable; the other variants remove one duplication at a time. Every variant is asked in the
// same session, so run-to-run noise (stage 1 flips a near-tie now and then) is shared and the
// comparison is like for like.
//
//   FUTURE_API_KEY=... node stage1-compact.mjs          # 20-question pilot
//   FUTURE_API_KEY=... node stage1-compact.mjs --full   # all 100 questions
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { loadRoster } from "../roster.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));

const client = new JevClient({ apiKey: process.env.FUTURE_API_KEY });
await client.listModels();

const QUESTION = "Would the skill in `skill` materially help with the request in `request`?";
const RUBRICS = {
  full: {
    true: "The skill's own description covers what the request needs, directly or as a clearly required first step",
    false: "Only topical overlap, a neighbouring task, or general assistance would be enough",
  },
  terse: {
    true: "Covers the request, directly or as a clearly required first step",
    false: "Only topical overlap, or general help would do",
  },
};

const skillOf = (skill, chars) => ({ name: skill.name, description: skill.indexLine.slice(0, chars) });

/**
 * `perQuestionRequest`  puts the user's query inside every question (what ships today).
 * `withCriteria`        repeats the yes/no rubric on every question.
 * `characters`          how much of each skill description is sent.
 */
const VARIANTS = {
  shipped_exact: { perQuestionRequest: true, criteria: "full", characters: 220, label: "现在（逐题带 request + 完整 criteria）" },
  no_repeat_request: { perQuestionRequest: false, criteria: "full", characters: 220, label: "去掉逐题重复的 request" },
  terse_criteria: { perQuestionRequest: false, criteria: "terse", characters: 220, label: "＋精简 criteria（发布版）" },
  no_criteria: { perQuestionRequest: false, criteria: "none", characters: 220, label: "＋完全去掉 criteria" },
};

const build = (config, query, skills) => ({
  state: { request: query },
  questions: {
    ...Object.fromEntries(
      skills.map((skill, i) => [
        `s_${i}`,
        {
          type: "noul",
          instructions: {
            question: QUESTION,
            skill: skillOf(skill, config.characters),
            ...(config.perQuestionRequest ? { request: query } : {}),
          },
          ...(config.criteria === "none" ? {} : { criteria: RUBRICS[config.criteria] }),
        },
      ]),
    ),
  },
});

const LADDER = process.argv.includes("--ladder");
const sample = process.argv.includes("--full") ? questions : questions.filter((_, i) => i % 5 === 0);
const cache = cacheFor(process.argv.includes("--full") ? "stage1-compact-full" : "stage1-compact");
console.log(`题目 ${sample.length} 道 · 技能 ${roster.skills.length} 个\n`);

if (!LADDER) {
for (const question of sample) {
  for (const [variant, config] of Object.entries(VARIANTS)) {
    if (cache.has(`${question.id}__${variant}`)) continue;
    const { state, questions: asked } = build(config, question.text, roster.skills);
    const { answers, usage } = await client.systemOne({ state, questions: asked });
    const ranked = roster.skills
      .map((skill, i) => ({ name: skill.name, p: answers[`s_${i}`]?.noul ?? 0 }))
      .sort((a, b) => b.p - a.p);
    cache.put(`${question.id}__${variant}`, {
      id: question.id,
      variant,
      tokens: usage?.input_tokens ?? null,
      top: ranked.slice(0, 3),
      top1: ranked[0]?.name ?? null,
      top1p: ranked[0]?.p ?? null,
      needsSkill: answers.needs_skill?.noul ?? null,
    });
    process.stdout.write(`\r${variant} ${question.id}   `);
  }
}
console.log("\n");

// ------------------------------------------------------------------- compare

const ids = sample.map((q) => q.id);
const get = (id, variant) => cache.get(`${id}__${variant}`);
const median = (values) => {
  const sorted = values.filter((v) => typeof v === "number").sort((a, b) => a - b);
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : null;
};

const reference = "shipped_exact";
const refTokens = median(ids.map((id) => get(id, reference)?.tokens));

console.log("=== 与\"现在\"的排序一致率、以及 token 用量 ===");
console.log("  变体                               输入 token  相对现在  top-1 相同  top-3 集合相同  top-1 平均偏移");
for (const [variant, config] of Object.entries(VARIANTS)) {
  const values = ids.map((id) => get(id, variant)).filter(Boolean);
  if (values.length !== ids.length) continue;
  const tokens = median(values.map((v) => v.tokens));
  const sameTop1 = ids.filter((id) => get(id, variant).top1 === get(id, reference).top1).length;
  const sameSet = ids.filter((id) => {
    const a = new Set(get(id, variant).top.map((e) => e.name));
    const b = new Set(get(id, reference).top.map((e) => e.name));
    return a.size === b.size && [...a].every((name) => b.has(name));
  }).length;
  const meanDelta =
    ids.map((id) => Math.abs((get(id, variant).top1p ?? 0) - (get(id, reference).top1p ?? 0))).reduce((a, b) => a + b, 0) / ids.length;
  console.log(
    `  ${config.label.padEnd(34)} ${String(tokens).padStart(8)}  ${((tokens / refTokens) * 100).toFixed(0).padStart(5)}%   ` +
      `${String(sameTop1).padStart(2)}/${ids.length}       ${String(sameSet).padStart(2)}/${ids.length}          ${meanDelta.toFixed(3)}`,
  );
}

console.log(`\n=== 同一次会话内的自比（说明噪音多大）===`);
{
  const values = ids.map((id) => get(id, reference)).filter(Boolean);
  console.log(`  ${reference} 这次的中位 token = ${refTokens}；记录的发布版是 29,802（同一种请求，应接近）`);
  const top1spread = values.map((v) => v.top1p).filter((p) => typeof p === "number").sort((a, b) => a - b);
  console.log(`  20 道里 top-1 概率的分布：最低 ${top1spread[0]?.toFixed(2)} 中位 ${median(top1spread)?.toFixed(2)} 最高 ${top1spread.at(-1)?.toFixed(2)}`);
}

console.log(`\n=== 逐题看被改动的那几道（对照最后要保护的：最终决策）===`);
for (const variant of ["no_repeat_request", "no_repeat_no_criteria", "compact_short_desc"]) {
  const changed = ids.filter((id) => get(id, variant).top1 !== get(id, reference).top1);
  console.log(`  ${variant}: 首答不同 ${changed.length} 道`);
  for (const id of changed.slice(0, 6)) {
    const a = get(id, reference);
    const b = get(id, variant);
    console.log(
      `    ${id}: 现在 ${a.top1}(${a.top1p.toFixed(2)}) → 压缩 ${b.top1}(${b.top1p.toFixed(2)})` +
        `  参照=${gold.get(id).gold[0] ?? "拒答"}`,
    );
  }
}



}

// ------------------------------------------------------------------- ladder mode

if (process.argv.includes("--ladder")) {
  const full = questions;
  const goldOf = (id) => gold.get(id).gold;
  const ladderCache = cacheFor("stage1-ladder");
  const maxFit = (row) => (row.stage2 && row.stage2.length ? Math.max(...row.stage2.map((c) => c.fit ?? 0)) : 0);
  const topFitName = (row) => [...(row.stage2 ?? [])].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0))[0]?.name ?? null;
  /** The shipped rule, applied to whatever stage 1 handed over. */
  const decide = (row) => {
    if (row.top1_noul < 0.7) return null;
    if (!row.stage2) return row.gate?.[0] ?? null;
    return maxFit(row) >= 0.75 ? topFitName(row) : null;
  };

  const stage2 = await import("../suggest.mjs");
  for (const [variant, config] of Object.entries(VARIANTS)) {
    for (const question of full) {
      if (ladderCache.has(`${question.id}__${variant}`)) continue;
      const { state, questions: asked } = build(config, question.text, roster.skills);
      const { answers, usage } = await client.systemOne({ state, questions: asked });
      const ranked = roster.skills
        .map((skill, i) => ({ name: skill.name, p: answers[`s_${i}`]?.noul ?? 0 }))
        .sort((a, b) => b.p - a.p);
      const top = ranked.slice(0, stage2.SHORTLIST);
      // Stage 2 over this variant's own shortlist, using the shipped question builder.
      const suggester = new stage2.Suggester(
        { ...roster, byName: new Map(top.map((entry) => [entry.name, roster.byName.get(entry.name)])) },
        { apiKey: process.env.FUTURE_API_KEY },
      );
      const probe = await verifySecondCall(suggester, question.text, top);
      ladderCache.put(`${question.id}__${variant}`, {
        id: question.id,
        variant,
        stage1_tokens: usage?.input_tokens ?? null,
        stage2_tokens: probe?.usage?.input_tokens ?? null,
        top1_noul: ranked[0]?.p ?? 0,
        gate: top.map((entry) => entry.name),
        stage2: probe?.candidates ?? null,
      });
      process.stdout.write(`\r ladder ${variant} ${question.id}   `);
    }
  }
  console.log("");

  const ids = full.map((q) => q.id);
  console.log("\n=== 压缩阶梯：同一批 100 题、同一套阈值（门控 0.70 / 复核 0.75） ===");
  console.log("  配置                                每题 token  相对原样  首答对        正确拒答     误拒  误推  总一致  逐题差异");
  const baseline = new Map(ids.map((id) => [id, ladderCache.get(`${id}__shipped_exact`)]));
  let previousRows = null;
  for (const [variant, config] of Object.entries(VARIANTS)) {
    const rows = new Map(ids.map((id) => [id, ladderCache.get(`${id}__${variant}`)]));
    let firstCorrect = 0;
    let correctRefusal = 0;
    let falseRefusal = 0;
    let falseSuggest = 0;
    let agreement = 0;
    for (const id of ids) {
      const reference = goldOf(id);
      const predicted = decide(rows.get(id));
      if (reference.length === 0) {
        if (predicted === null) {
          correctRefusal += 1;
          agreement += 1;
        } else falseSuggest += 1;
      } else if (predicted === null) falseRefusal += 1;
      else if (predicted === reference[0]) {
        firstCorrect += 1;
        agreement += 1;
      }
    }
    const tokens = ids.map((id) => (rows.get(id).stage1_tokens ?? 0) + (rows.get(id).stage2_tokens ?? 0)).sort((a, b) => a - b);
    const median = tokens[Math.floor(tokens.length / 2)];
    const baseTokens = ids.map((id) => (baseline.get(id).stage1_tokens ?? 0) + (baseline.get(id).stage2_tokens ?? 0)).sort((a, b) => a - b);
    const vsBase = previousRows === null ? 0 : ids.filter((id) => decide(rows.get(id)) !== decide(previousRows.get(id))).length;
    console.log(
      `  ${config.label.padEnd(34)} ${String(median).padStart(8)}  ${((median / baseTokens[Math.floor(baseTokens.length / 2)]) * 100).toFixed(0).padStart(5)}%   ` +
        `${String(firstCorrect).padStart(2)}/65=${((firstCorrect / 65) * 100).toFixed(1)}%  ${String(correctRefusal).padStart(2)}/35=${((correctRefusal / 35) * 100).toFixed(1)}%  ` +
        `${String(falseRefusal).padStart(3)}  ${String(falseSuggest).padStart(3)}  ${String(agreement).padStart(2)}/100   ${vsBase} 处`,
    );
    for (const id of ids.filter((row) => decide(rows.get(row)) !== decide(baseline.get(row)))) {
      const before = decide(baseline.get(id));
      const after = decide(rows.get(id));
      const reference = goldOf(id)[0] ?? null;
      if (variant === "no_criteria" || variant === "terse_criteria") continue;
      console.log(`      ${id}: ${before ?? "拒答"} → ${after ?? "拒答"}（参照 ${reference ?? "拒答"}）`);
    }
    previousRows = rows;
  }

  console.log("\n=== 相对原样的逐题差异 ===");
  for (const [variant, config] of Object.entries(VARIANTS).slice(1)) {
    const rows = new Map(ids.map((id) => [id, ladderCache.get(`${id}__${variant}`)]));
    const changed = ids.filter((id) => decide(rows.get(id)) !== decide(baseline.get(id)));
    console.log(`\n  ${config.label}（${changed.length} 处）：`);
    for (const id of changed) {
      const before = decide(baseline.get(id));
      const after = decide(rows.get(id));
      const reference = goldOf(id)[0] ?? null;
      const verdict = after === reference ? "→ 变好" : before === reference ? "→ 变差" : "→ 仍错";
      console.log(`    ${id}: ${before ?? "拒答"} → ${after ?? "拒答"}（参照 ${reference ?? "拒答"}）${verdict}`);
    }
  }
}
