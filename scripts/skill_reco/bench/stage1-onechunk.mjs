#!/usr/bin/env node
// "All in one chunk": a single Choice over all 141 skills, WITH a none_of_these option.
//
// This is the design rejected early on, plus the one thing it was missing. The rejection was for
// saturation — with no way to decline, the Choice has to name a winner, and of 141 options a median
// of three get any probability at all, so "the top three" is mostly zeros. Two separable questions:
//
//   1. does the none option fix the RANKING (the top-3 stage 2 receives)?
//   2. does it fix the GATE? (that part is already known to work for chunked rosters)
//
// Recorded per question: the none probability, how many options got non-zero probability, the top-3
// by probability, and the ungated stage-2 answer over those three. Everything needed to evaluate any
// threshold offline.
//
//   FUTURE_API_KEY=... node stage1-onechunk.mjs
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { Suggester, NONE_OF_THESE } from "../suggest.mjs";
import { verifySecondCall } from "./second-call.mjs";
import { loadRoster } from "../roster.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;
const client = new JevClient({ apiKey: process.env.FUTURE_API_KEY });

const criteria = {};
for (const skill of roster.skills) criteria[skill.name] = skill.indexLine;
criteria[NONE_OF_THESE] = "No skill in this list would help with the request";

const cache = cacheFor("stage1-onechunk");
console.log(`一个 Choice，${roster.skills.length} 个技能选项（含 none）；题目 ${questions.length}\n`);

for (const question of questions) {
  if (cache.has(question.id)) continue;

  const { answers, usage, ms } = await client.systemOne({
    state: { request: question.text },
    questions: {
      best_skill: {
        type: "choice",
        instructions: {
          question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
          how_to_judge: `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}".`,
        },
        criteria,
      },
    },
  });

  const probabilities = answers.best_skill?.probabilities ?? {};
  const ranked = Object.entries(probabilities)
    .filter(([name]) => name !== NONE_OF_THESE)
    .map(([name, p]) => ({ name, p }))
    .sort((a, b) => b.p - a.p);
  const top3 = ranked.slice(0, 3).map((entry) => entry.name);

  let stage2Answer = null;
  let stage2Tokens = 0;
  let stage2Candidates = [];
  if (top3.length) {
    const subRoster = {
      ...roster,
      skills: top3.map((name) => roster.byName.get(name)),
      byName: new Map(top3.map((name) => [name, roster.byName.get(name)])),
    };
    const suggester = new Suggester(subRoster, { apiKey: process.env.FUTURE_API_KEY });
    const probe = await verifySecondCall(suggester, question.text, top3.map((name) => ({ name })));
    stage2Answer = probe?.winner ?? null;
    stage2Tokens = probe?.usage?.input_tokens ?? 0;
    stage2Candidates = (probe?.candidates ?? []).map((c) => ({ name: c.name, fit: c.fit }));
  }

  cache.put(question.id, {
    id: question.id,
    noneProbability: probabilities[NONE_OF_THESE] ?? null,
    pick: answers.best_skill?.choice ?? null,
    nonzero: ranked.filter((entry) => entry.p > 0.001).length,
    rankedTop: ranked.slice(0, 6),
    top3,
    top1p: ranked[0]?.p ?? null,
    stage1_tokens: usage?.input_tokens ?? null,
    stage2_tokens: stage2Tokens,
    stage2Answer,
    stage2Candidates,
    tokens_total: (usage?.input_tokens ?? 0) + stage2Tokens,
    ms,
  });
  process.stdout.write(`\r${question.id}   `);
}
console.log("");
