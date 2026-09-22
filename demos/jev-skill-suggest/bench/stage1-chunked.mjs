#!/usr/bin/env node
// The user's proposal: split the roster into chunks, give every chunk Choice a "none of these"
// option, then finish with one more Choice (also with a none option) over the chunk winners.
//
// This targets the exact flaw found in the earlier chunked-Choice test: a Choice with no none
// option ALWAYS names a winner, so "the best of chunk k" is ~1.0 for every chunk and the maxima
// are not comparable across chunks. A none option lets a chunk decline, and the final Choice
// re-ranks the survivors on one scale.
//
//   A  shipped          141 parallel Nouls + needs_skill.           Gate: top-1 Noul >= 0.70
//   D  chunked Choice   8 choices of ~18 (+none each), then a final  Gate: final none / needs_skill
//                       Choice over the chunk winners (+none)
//   E  D without the final Choice (merge by within-chunk probability) — isolates what the final
//                       Choice buys, since cross-chunk probabilities are not comparable.
//
// Stage 2 is identical in all three and works off whatever top-3 stage 1 produced.
//
//   TYPESAFE_API_KEY=... node stage1-chunked.mjs [chunkSize]
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { Suggester } from "../suggest.mjs";
import { verifySecondCall } from "./second-call.mjs";

// This script measures the SUPERSEDED 141-Noul routing, so it pins that design's gate locally
// rather than importing the shipped constant (which now belongs to the chunked design).
const GATE_THRESHOLD = 0.7;
import { loadRoster } from "../roster.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));

const CHUNK = Number(process.argv[2] ?? 18);
const client = new JevClient({ apiKey: process.env.TYPESAFE_API_KEY });
await client.listModels();

const NONE = "none_of_these";
const NONE_PER_CHUNK = "No skill in this list would help";
const NONE_FINAL = "No skill fits this request; general assistance is enough";

const chunks = [];
for (let i = 0; i < roster.skills.length; i += CHUNK) chunks.push(roster.skills.slice(i, i + CHUNK));

const NEEDS_SKILL = {
  type: "noul",
  instructions: {
    question:
      "Does completing the user's request in `request` genuinely call for one of the catalogued skills, " +
      "rather than general assistance or an everyday answer?",
    guidance:
      "Answer yes only when a catalogued skill would materially change or improve the work. Answer no for " +
      "small talk, general knowledge questions, and tasks any capable assistant handles without special instructions.",
  },
  criteria: {
    true: "A catalogued skill is genuinely required or clearly valuable",
    false: "General assistance is enough; no catalogued skill applies",
  },
};

console.log(`技能 ${roster.skills.length} 个 → ${chunks.length} 块 × 约 ${CHUNK} 个\n`);

const cache = cacheFor(`stage1-chunked-${CHUNK}`);

for (const question of questions) {
  if (cache.has(question.id)) continue;

  // ---- Request 1: one Choice per chunk (each with its own none option) + the gate question.
  const chunkQuestions = { needs_skill: NEEDS_SKILL };
  chunks.forEach((chunk, k) => {
    const criteria = {};
    for (const skill of chunk) criteria[skill.name] = skill.indexLine;
    criteria[NONE] = NONE_PER_CHUNK;
    chunkQuestions[`chunk_${k}`] = {
      type: "choice",
      instructions: {
        question: `The request in \`request\` needs a skill from \`criteria\`. Which one, or does none of them help?`,
        how_to_judge: `Pick the closest match if any is plausible, otherwise choose "${NONE}". A later question decides whether any skill is needed at all.`,
      },
      criteria,
    };
  });
  const first = await client.systemOne({ state: { request: question.text }, questions: chunkQuestions });

  // Each chunk's own winner, and how often a chunk declines.
  const perChunk = chunks.map((chunk, k) => {
    const probabilities = first.answers?.[`chunk_${k}`]?.probabilities ?? {};
    const ranked = Object.entries(probabilities)
      .map(([name, p]) => ({ name, p }))
      .sort((a, b) => b.p - a.p);
    const top = ranked[0] ?? null;
    const declined = !top || top.name === NONE || (first.answers?.[`chunk_${k}`]?.choice ?? null) === NONE;
    return { k, chunk, declined, top: declined ? null : top, ranked };
  });

  const survivors = perChunk.filter((entry) => !entry.declined && entry.top).map((entry) => entry.top.name);
  const needsSkill = first.answers?.needs_skill?.noul ?? null;

  // ---- E: merge by within-chunk probability (cross-chunk, so this is the naive merge).
  const naiveTop3 = perChunk
    .filter((entry) => !entry.declined && entry.top)
    .map((entry) => ({ ...entry.top, k: entry.k }))
    .sort((a, b) => b.p - a.p)
    .slice(0, 3)
    .map((entry) => entry.name);

  // ---- Request 2: the final Choice over the chunk winners, with a global none option.
  let finalTokens = 0;
  let finalPick = null;
  let finalNoneProbability = null;
  let finalTop3 = [];
  if (survivors.length) {
    const criteria = {};
    for (const name of survivors) criteria[name] = roster.byName.get(name)?.indexLine ?? "";
    criteria[NONE] = NONE_FINAL;
    const second = await client.systemOne({
      state: { request: question.text },
      questions: {
        best_of_all: {
          type: "choice",
          instructions: {
            question: "Of `criteria`, which single skill would help most with the request in `request`?",
            how_to_judge:
              `Choose "${NONE}" if none of them really does what the request needs. Rank by how well each ` +
              "skill's own description covers the request.",
          },
          criteria,
        },
      },
    });
    const probabilities = second.answers?.best_of_all?.probabilities ?? {};
    finalTokens = second.usage?.input_tokens ?? 0;
    finalPick = second.answers?.best_of_all?.choice ?? null;
    finalNoneProbability = probabilities[NONE] ?? null;
    finalTop3 = Object.entries(probabilities)
      .filter(([name]) => name !== NONE)
      .map(([name, p]) => ({ name, p }))
      .sort((a, b) => b.p - a.p)
      .slice(0, 3)
      .map((entry) => entry.name);
  }

  // ---- Stage 2 over this variant's top-3, exactly as shipped.
  const runStage2 = async (top3) => {
    if (!top3.length) return { answer: null, tokens: 0, bestFit: null };
    const subRoster = {
      ...roster,
      skills: top3.map((name) => roster.byName.get(name)),
      byName: new Map(top3.map((name) => [name, roster.byName.get(name)])),
    };
    const suggester = new Suggester(subRoster, { apiKey: process.env.TYPESAFE_API_KEY });
    const probe = await verifySecondCall(suggester, question.text, top3.map((name) => ({ name })));
    return { answer: probe?.winner ?? null, tokens: probe?.usage?.input_tokens ?? 0, bestFit: probe?.best_fit ?? null };
  };

  const stage2D = await runStage2(finalTop3);
  const stage2E = naiveTop3.length ? await runStage2(naiveTop3) : { answer: null, tokens: 0, bestFit: null };

  cache.put(question.id, {
    id: question.id,
    declines: perChunk.filter((entry) => entry.declined).length,
    survivors: survivors.length,
    needsSkill,
    finalNoneProbability,
    finalPick,
    finalTop3,
    naiveTop3,
    stage1_tokens: first.usage?.input_tokens ?? null,
    final_tokens: finalTokens,
    // D and E share request 1; only D pays for the final Choice.
    tokens_D: (first.usage?.input_tokens ?? 0) + finalTokens + stage2D.tokens,
    tokens_E: (first.usage?.input_tokens ?? 0) + stage2E.tokens,
    answer_D: needsSkill !== null && needsSkill < GATE_THRESHOLD ? null : stage2D.answer,
    answer_E: needsSkill !== null && needsSkill < GATE_THRESHOLD ? null : stage2E.answer,
    answer_D_byfinal: finalNoneProbability !== null && finalNoneProbability >= 0.5 ? null : stage2D.answer,
  });
  process.stdout.write(`\r${question.id}   `);
}
console.log("");

// ------------------------------------------------------------------- report

const rows = new Map(cache.all().map((row) => [row.id, row]));
const median = (values) => {
  const sorted = values.filter((v) => typeof v === "number").sort((a, b) => a - b);
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : null;
};
const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;

const score = (answerOf) => {
  let firstCorrect = 0;
  let correctRefusal = 0;
  let falseRefusal = 0;
  let falseSuggest = 0;
  let agreement = 0;
  for (const question of questions) {
    const reference = gold.get(question.id).gold;
    const answer = answerOf(rows.get(question.id));
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

const shipped = new Map(cacheFor("jev").all().map((row) => [row.id, row]));
const shippedScore = score((row) => shipped.get(row.id).answer[0] ?? null);
const shippedTokens = median(questions.map((q) => {
  const row = shipped.get(q.id);
  return (row.stage1_tokens ?? 0) + (row.stage2_tokens ?? 0);
}));

console.log(`\n=== 质量与成本（100 题：65 有答案 / 35 无答案）===`);
console.log("  方案                                首答正确        正确拒答       误拒  误推  总一致  每题 token  费用/题");
const line = (label, s, tokens) =>
  `  ${label.padEnd(34)} ${String(s.firstCorrect).padStart(2)}/65 = ${pct(s.firstCorrect, 65).padStart(6)}  ${String(s.correctRefusal).padStart(2)}/35 = ${pct(s.correctRefusal, 35).padStart(6)}  ${String(s.falseRefusal).padStart(3)}  ${String(s.falseSuggest).padStart(3)}  ${String(s.agreement).padStart(2)}/100  ${String(tokens).padStart(8)}   $${((tokens * 0.042) / 1e6).toFixed(5)}`;

console.log(line("A 现在：141 道 noul", shippedScore, shippedTokens));
console.log(line("D 分块 Choice + 最终 Choice", score((row) => row.answer_D), median(questions.map((q) => rows.get(q.id).tokens_D))));
console.log(line("E 分块 Choice（无最终 Choice）", score((row) => row.answer_E), median(questions.map((q) => rows.get(q.id).tokens_E))));
console.log(line("D' 分块 Choice + 最终 none 门控", score((row) => row.answer_D_byfinal), median(questions.map((q) => rows.get(q.id).tokens_D))));

console.log(`\n=== 分块的 none 选项起作用了吗（${chunks.length} 块/题）===`);
const declines = questions.map((q) => rows.get(q.id).declines);
const survivorsPerQ = questions.map((q) => rows.get(q.id).survivors);
console.log(`  每题有 ${median(declines)} 个块选择"none"（共 ${chunks.length} 块），幸存候选中位 ${median(survivorsPerQ)} 个`);
console.log(`  该拒答的题 vs 该推荐的题，none 块数：${median(questions.filter((q) => gold.get(q.id).gold.length === 0).map((q) => rows.get(q.id).declines))} vs ${median(questions.filter((q) => gold.get(q.id).gold.length > 0).map((q) => rows.get(q.id).declines))}`);
const finalNone = questions.map((q) => rows.get(q.id).finalNoneProbability).filter((v) => typeof v === "number");
console.log(`  最终 Choice 里 none 的概率：中位 ${median(finalNone).toFixed(3)}；≥0.5 的题 ${finalNone.filter((v) => v >= 0.5).length}/100`);

console.log(`\n=== 正确技能活到最终 Choice 的概率（分块召回上限）===`);
const survived = questions.filter((q) => gold.get(q.id).gold.length > 0 && rows.get(q.id).survivors > 0);
const inFinal = survived.filter((q) => {
  const row = rows.get(q.id);
  return row.finalTop3.includes(gold.get(q.id).gold[0]);
});
console.log(`  该推荐的 ${survived.length} 道里，正确技能进入最终 top-3 的：${inFinal.length} = ${pct(inFinal.length, survived.length)}`);
const chunkedAway = questions.filter((q) => gold.get(q.id).gold.length > 0 && rows.get(q.id).survivors === 0);
console.log(`  所有块都选 none（正确技能根本没进最终 Choice）的题：${chunkedAway.length} 道`);

console.log(`\n=== 与现在选出的候选对比 ===`);
const sameTop1 = questions.filter((q) => {
  const a = shipped.get(q.id).stage1_top?.[0]?.name ?? null;
  const d = rows.get(q.id).finalTop3[0] ?? null;
  return a === d;
}).length;
console.log(`  第一名相同 ${sameTop1}/100`);
