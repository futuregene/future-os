#!/usr/bin/env node
// What stage 1 costs and does with a Choice instead of 141 Nouls, measured end to end.
//
// The Choice was rejected early on for saturation (one winner at 1.0, the other 140 exactly 0,
// so "top three" is a tie among zeros), but the cost side was never measured, and the gate needs
// rethinking: a Choice always names a winner, so it cannot express "no skill applies" and the
// gate has to come from something else. Three full pipelines over the same 100 questions:
//
//   A  shipped    141 Nouls + needs_skill Noul. Gate = top-1 Noul >= 0.70
//   B  choice     one 141-option Choice + needs_skill Noul. Gate = needs_skill >= 0.70
//   C  choice     the same Choice, gated on the winner's own probability >= 0.70
//
// Stage 2 is identical in all three (its three candidates come from stage 1's ranking), so any
// difference is stage 1's. Tokens are whatever the API reports in `usage`.
//
//   TYPESAFE_API_KEY=... node stage1-choice.mjs
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { Suggester } from "../suggest.mjs";
import { FITS_THRESHOLD } from "./second-call.mjs";

// Compares against the SUPERSEDED 141-Noul routing: its gate is pinned here, not imported.
const GATE_THRESHOLD = 0.7;
import { loadRoster } from "../roster.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));

const client = new JevClient({ apiKey: process.env.TYPESAFE_API_KEY });
await client.listModels();

const QUESTION = "Would the skill in `skill` materially help with the request in `request`?";
const CRITERIA = {
  true: "Covers the request, directly or as a clearly required first step",
  false: "Only topical overlap, or general help would do",
};
const NEEDS_SKILL = {
  type: "noul",
  instructions: {
    question:
      "Does completing the user's request in `request` genuinely call for one of the catalogued " +
      "skills, rather than general assistance or an everyday answer?",
    guidance:
      "Answer yes only when a catalogued skill would materially change or improve the work. Answer no " +
      "for small talk, general knowledge questions, and tasks any capable assistant handles without " +
      "special instructions.",
  },
  criteria: {
    true: "A catalogued skill is genuinely required or clearly valuable",
    false: "General assistance is enough; no catalogued skill applies",
  },
};

/** Stage 1 as 141 parallel Nouls — what ships. */
const stage1Noul = async (query) => {
  const questionsFor = {
    needs_skill: NEEDS_SKILL,
    ...Object.fromEntries(
      roster.skills.map((skill, i) => [
        `s_${i}`,
        {
          type: "noul",
          instructions: { question: QUESTION, skill: { name: skill.name, description: skill.indexLine } },
          criteria: CRITERIA,
        },
      ]),
    ),
  };
  const { answers, usage, ms } = await client.systemOne({ state: { request: query }, questions: questionsFor });
  const ranked = roster.skills
    .map((skill, i) => ({ name: skill.name, p: answers[`s_${i}`]?.noul ?? 0 }))
    .sort((a, b) => b.p - a.p);
  return {
    ranked,
    needsSkill: answers.needs_skill?.noul ?? null,
    gate: ranked[0]?.p ?? 0,
    winnerProbability: ranked[0]?.p ?? 0,
    tokens: usage?.input_tokens ?? null,
    ms,
  };
};

/** Stage 1 as a single Choice over all 141 skills. */
const stage1Choice = async (query) => {
  const criteria = {};
  for (const skill of roster.skills) criteria[skill.name] = skill.indexLine;
  const questionsFor = {
    needs_skill: NEEDS_SKILL,
    best_skill: {
      type: "choice",
      instructions: {
        question: "Which single skill from `criteria` would help most with the request in `request`?",
        how_to_judge: "Pick the closest match even if the fit is weak; another question decides whether any is needed.",
      },
      criteria,
    },
  };
  const { answers, usage, ms } = await client.systemOne({ state: { request: query }, questions: questionsFor });
  const probabilities = answers.best_skill?.probabilities ?? {};
  const ranked = Object.entries(probabilities)
    .map(([name, p]) => ({ name, p }))
    .sort((a, b) => b.p - a.p);
  const top = ranked[0];
  return {
    ranked,
    needsSkill: answers.needs_skill?.noul ?? null,
    gate: top?.p ?? 0,
    winnerProbability: top?.p ?? 0,
    choice: answers.best_skill?.choice ?? null,
    confidence: answers.best_skill?.confidence ?? null,
    // How sharp is the distribution? A saturated Choice gives 1.0 / 0.0 / 0.0.
    nonzero: ranked.filter((entry) => entry.p > 0.001).length,
    tokens: usage?.input_tokens ?? null,
    ms,
  };
};

const VARIANTS = {
  A_noul: { label: "A 现在：141 道 noul", stage1: stage1Noul, gateOf: (s) => s.gate, gateName: "top-1 noul" },
  B_choice_needs: { label: "B Choice + needs_skill 门控", stage1: stage1Choice, gateOf: (s) => s.needsSkill, gateName: "needs_skill" },
  C_choice_winner: { label: "C Choice + 赢家概率门控", stage1: stage1Choice, gateOf: (s) => s.winnerProbability, gateName: "winning p" },
};

const cache = cacheFor("stage1-choice");
console.log(`题目 ${questions.length} · 技能 ${roster.skills.length}\n`);

for (const [key, variant] of Object.entries(VARIANTS)) {
  let done = 0;
  for (const question of questions) {
    if (cache.has(`${question.id}__${key}`)) {
      done += 1;
      continue;
    }
    const stage1 = await variant.stage1(question.text);
    const gateValue = variant.gateOf(stage1);
    const gatePasses = gateValue >= GATE_THRESHOLD;
    const top3 = stage1.ranked.slice(0, 3);

    // Stage 2 exactly as shipped, over whichever three candidates this variant produced.
    const subRoster = {
      ...roster,
      skills: top3.map((entry) => roster.byName.get(entry.name)),
      byName: new Map(top3.map((entry) => [entry.name, roster.byName.get(entry.name)])),
    };
    const suggester = new Suggester(subRoster, { apiKey: process.env.TYPESAFE_API_KEY });
    const probe = gatePasses ? await verifySecondCall(suggester, question.text, top3) : null;

    cache.put(`${question.id}__${key}`, {
      id: question.id,
      variant: key,
      gateValue,
      gatePasses,
      top3: top3.map((entry) => ({ name: entry.name, p: entry.p })),
      nonzero: stage1.nonzero ?? null,
      stage1_tokens: stage1.tokens,
      stage2_tokens: probe?.usage?.input_tokens ?? null,
      answer: gatePasses && probe?.winner ? probe.winner : null,
      best_fit: probe?.best_fit ?? null,
      stage1_ms: stage1.ms,
    });
    done += 1;
    process.stdout.write(`\r${key} ${done}/${questions.length}   `);
  }
  console.log("");
}

// ------------------------------------------------------------------- report

const rowsOf = (key) => new Map(questions.map((q) => [q.id, cache.get(`${q.id}__${key}`)]));
const median = (values) => {
  const sorted = values.filter((v) => typeof v === "number").sort((a, b) => a - b);
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : null;
};
const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;

console.log(`\n=== 质量（100 题：65 有答案 / 35 无答案，判定规则与发布版一致）===`);
console.log("  变体                          首答正确        正确拒答       误拒  误推  总一致  每题 token");
for (const [key, variant] of Object.entries(VARIANTS)) {
  const rows = rowsOf(key);
  let firstCorrect = 0;
  let correctRefusal = 0;
  let falseRefusal = 0;
  let falseSuggest = 0;
  let agreement = 0;
  for (const question of questions) {
    const row = rows.get(question.id);
    const reference = gold.get(question.id).gold;
    const answer = row.answer;
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
  const tokens = median(questions.map((q) => (rows.get(q.id).stage1_tokens ?? 0) + (rows.get(q.id).stage2_tokens ?? 0)));
  console.log(
    `  ${variant.label.padEnd(28)} ${String(firstCorrect).padStart(2)}/65 = ${pct(firstCorrect, 65).padStart(6)}  ` +
      `${String(correctRefusal).padStart(2)}/35 = ${pct(correctRefusal, 35).padStart(6)}  ${String(falseRefusal).padStart(3)}  ${String(falseSuggest).padStart(3)}  ${String(agreement).padStart(2)}/100  ${String(tokens).padStart(8)}`,
  );
}

console.log(`\n=== 成本（每题 token，全部取自 API 返回的 usage.input_tokens）===`);
console.log("  变体                          stage1 中位   stage2 中位   合计    相对 A    费用/题");
const baseTokens = median(questions.map((q) => {
  const row = rowsOf("A_noul").get(q.id);
  return (row.stage1_tokens ?? 0) + (row.stage2_tokens ?? 0);
}));
for (const [key, variant] of Object.entries(VARIANTS)) {
  const rows = rowsOf(key);
  const s1 = median(questions.map((q) => rows.get(q.id).stage1_tokens));
  const s2 = median(questions.map((q) => rows.get(q.id).stage2_tokens));
  const total = median(questions.map((q) => (rows.get(q.id).stage1_tokens ?? 0) + (rows.get(q.id).stage2_tokens ?? 0)));
  console.log(
    `  ${variant.label.padEnd(28)} ${String(s1).padStart(10)}   ${String(s2).padStart(10)}   ${String(total).padStart(6)}  ${((total / baseTokens) * 100).toFixed(0).padStart(5)}%   $${((total * 0.042) / 1e6).toFixed(5)}`,
  );
}

console.log(`\n=== Choice 的概率分布有多饱和（B/C 共用同一次 stage 1）===`);
const choiceRows = questions.map((q) => cache.get(`${q.id}__B_choice_needs`));
const nonzeroCounts = choiceRows.map((row) => row.nonzero);
console.log(`  每题非零概率的选项个数：中位 ${median(nonzeroCounts)}（141 个选项里）`);
console.log(`  分布情况：${[1, 2, 3, 5, 10].map((n) => `${n} 个以内: ${nonzeroCounts.filter((c) => c <= n).length} 题`).join("   ")}`);
const winners = choiceRows.map((row) => row.top3[0]);
console.log(`  赢家的概率：中位 ${median(winners.map((w) => w.p)).toFixed(3)}，最小 ${Math.min(...winners.map((w) => w.p)).toFixed(3)}`);
console.log(`  → 门控用赢家概率时，阈值 0.70 以下能通过的比例：${pct(winners.filter((w) => w.p >= GATE_THRESHOLD).length, winners.length)}`);

console.log(`\n=== 两种方案选出的候选有多不同 ===`);
const a = rowsOf("A_noul");
const b = rowsOf("B_choice_needs");
const sameTop1 = questions.filter((q) => a.get(q.id).top3[0].name === b.get(q.id).top3[0].name).length;
const sameSet = questions.filter((q) => {
  const x = new Set(a.get(q.id).top3.map((e) => e.name));
  const y = new Set(b.get(q.id).top3.map((e) => e.name));
  return x.size === y.size && [...x].every((name) => y.has(name));
}).length;
console.log(`  第一名相同 ${sameTop1}/100 · 前三名集合相同 ${sameSet}/100`);
