#!/usr/bin/env node
// Chunked-Choice stage 1, second pass: records everything needed to evaluate ANY operating point
// offline, so the cross-validation can vary the thresholds without re-calling the API.
//
// Per question it stores the final Choice's none probability, its pick, the stage-2 result with no
// gate applied, and the token counts. The first pass stored only pre-gated answers, which made an
// offline threshold sweep impossible.
//
//   FUTURE_API_KEY=... node stage1-chunked-raw.mjs <chunkSize>
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { Suggester } from "../suggest.mjs";
import { verifySecondCall } from "./second-call.mjs";
import { loadRoster } from "../roster.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;

const CHUNK = Number(process.argv[2] ?? 30);
const client = new JevClient({ apiKey: process.env.FUTURE_API_KEY });

const NONE = "none_of_these";
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

const cache = cacheFor(`stage1-chunked-raw-${CHUNK}`);
console.log(`${roster.skills.length} 技能 → ${chunks.length} 块 × 约 ${CHUNK}；题目 ${questions.length}\n`);

for (const question of questions) {
  if (cache.has(question.id)) continue;

  const chunkQuestions = { needs_skill: NEEDS_SKILL };
  chunks.forEach((chunk, k) => {
    const criteria = {};
    for (const skill of chunk) criteria[skill.name] = skill.indexLine;
    criteria[NONE] = "No skill in this list would help";
    chunkQuestions[`chunk_${k}`] = {
      type: "choice",
      instructions: {
        question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
        how_to_judge: `Pick the closest match if any is plausible, otherwise choose "${NONE}". A later question decides whether any skill is needed at all.`,
      },
      criteria,
    };
  });
  const first = await client.systemOne({ state: { request: question.text }, questions: chunkQuestions });

  const survivors = [];
  chunks.forEach((chunk, k) => {
    const answer = first.answers?.[`chunk_${k}`];
    const pick = answer?.choice ?? null;
    if (pick && pick !== NONE) survivors.push(pick);
  });

  let finalNone = null;
  let finalPick = null;
  let finalTokens = 0;
  let finalTop3 = [];
  if (survivors.length) {
    const criteria = {};
    for (const name of survivors) criteria[name] = roster.byName.get(name)?.indexLine ?? "";
    criteria[NONE] = "No skill fits this request; general assistance is enough";
    const second = await client.systemOne({
      state: { request: question.text },
      questions: {
        best_of_all: {
          type: "choice",
          instructions: {
            question: "Of `criteria`, which single skill would help most with the request in `request`?",
            how_to_judge: `Choose "${NONE}" if none of them really does what the request needs.`,
          },
          criteria,
        },
      },
    });
    const probabilities = second.answers?.best_of_all?.probabilities ?? {};
    finalTokens = second.usage?.input_tokens ?? 0;
    finalPick = second.answers?.best_of_all?.choice ?? null;
    finalNone = probabilities[NONE] ?? null;
    finalTop3 = Object.entries(probabilities)
      .filter(([name]) => name !== NONE)
      .map(([name, p]) => ({ name, p }))
      .sort((a, b) => b.p - a.p)
      .slice(0, 3)
      .map((entry) => entry.name);
  }

  // Stage 2 over the survivors, WITHOUT any gate: the raw result an operating point would act on.
  let stage2Answer = null;
  let stage2Fit = null;
  let stage2Tokens = 0;
  let stage2Candidates = [];
  if (finalTop3.length) {
    const subRoster = {
      ...roster,
      skills: finalTop3.map((name) => roster.byName.get(name)),
      byName: new Map(finalTop3.map((name) => [name, roster.byName.get(name)])),
    };
    const suggester = new Suggester(subRoster, { apiKey: process.env.FUTURE_API_KEY });
    const probe = await verifySecondCall(suggester, question.text, finalTop3.map((name) => ({ name })));
    stage2Answer = probe?.winner ?? null;
    stage2Fit = probe?.best_fit ?? null;
    stage2Tokens = probe?.usage?.input_tokens ?? 0;
    stage2Candidates = (probe?.candidates ?? []).map((c) => ({ name: c.name, fit: c.fit }));
  }

  cache.put(question.id, {
    id: question.id,
    chunk: CHUNK,
    chunks: chunks.length,
    survivors: survivors.length,
    needsSkill: first.answers?.needs_skill?.noul ?? null,
    finalNone,
    finalPick,
    finalTop3,
    stage2Answer,
    stage2Fit,
    stage2Candidates,
    stage1_tokens: first.usage?.input_tokens ?? null,
    final_tokens: finalTokens,
    stage2_tokens: stage2Tokens,
    tokens_total: (first.usage?.input_tokens ?? 0) + finalTokens + stage2Tokens,
  });
  process.stdout.write(`\r${CHUNK}: ${question.id}   `);
}
console.log("");
