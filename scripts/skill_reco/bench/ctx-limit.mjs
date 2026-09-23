#!/usr/bin/env node
// How many full option tables fit in ONE request before the trailing questions degrade?
//
// This is a method question, not a product question. The prompt sweep packs several variants into
// one request so the comparison is paired (run-to-run variance is as large as the effects being
// measured), but that only holds while the request fits. In batch D (5 tables, ~30k tokens) the
// LAST variant refused all 100 questions while the four ahead of it behaved normally — the reading
// being that the request ran past the context window and the trailing question degraded.
//
// So: send the SAME shipped table k times as questions q0..q(k-1) over one `state`, and compare each
// position against the standalone recording of the identical single-question request
// (runs/stage1-confidence). If position matters, the last positions disagree more.
//
//   FUTURE_API_KEY=... node bench/ctx-limit.mjs [maxTables] [nQuestions]
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { loadRoster } from "../roster.mjs";
import { NONE_GATE_THRESHOLD, NONE_OF_THESE } from "../suggest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;

const maxTables = Number(process.argv[2] ?? 6);
const nQuestions = Number(process.argv[3] ?? 6);

const client = new JevClient({ apiKey: process.env.FUTURE_API_KEY });
if (!client.apiKey) {
  console.error("FUTURE_API_KEY is not set");
  process.exit(2);
}

const standalone = new Map(
  fs
    .readdirSync(path.join(RUNS_DIR, "stage1-confidence"))
    .filter((f) => f.endsWith(".json"))
    .map((f) => {
      const r = JSON.parse(fs.readFileSync(path.join(RUNS_DIR, "stage1-confidence", f), "utf8"));
      return [r.id, { none: r.none ?? 0, top: r.ranked?.[0]?.name ?? null }];
    }),
);

/** The shipped table: one line per skill + the none option. */
const shippedCriteria = () => {
  const criteria = {};
  for (const skill of roster.skills) criteria[skill.name] = skill.indexLine;
  criteria[NONE_OF_THESE] = "No skill in this list would help with the request";
  return criteria;
};

const SHIPPED_INSTRUCTIONS = {
  question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
  how_to_judge:
    `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
    "Choosing it is a normal answer here, not a fallback.",
};

// Questions the standalone recording answered with a specific skill — an unambiguous reference for
// "is this copy of the table still behaving?".
const picked = questions.filter((q) => (standalone.get(q.id)?.none ?? 1) < NONE_GATE_THRESHOLD).slice(0, nQuestions);

console.log(`\n同一张已发布选项表重复 k 次放进一个请求，逐位置与单独请求的记录对比（${picked.length} 道题）\n`);
console.log("表数 k | 总 input_tokens | 各位置与单独记录一致数 | 最后一个问题选中的技能");
console.log("-------|----------------|---------------------|--------------------------");

for (let k = 2; k <= maxTables; k += 1) {
  const perPosition = Array.from({ length: k }, () => 0);
  let tokens = null;
  const lastPicks = [];

  for (const q of picked) {
    const questionsObj = {};
    for (let i = 0; i < k; i += 1) {
      questionsObj[`q${i}`] = { type: "choice", instructions: SHIPPED_INSTRUCTIONS, criteria: shippedCriteria() };
    }
    const { answers, usage } = await client.systemOne({ state: { request: q.text }, questions: questionsObj });
    tokens = usage?.input_tokens ?? tokens;

    for (let i = 0; i < k; i += 1) {
      const a = answers?.[`q${i}`] ?? {};
      const none = a.probabilities?.[NONE_OF_THESE] ?? 1;
      const top = Object.entries(a.probabilities ?? {})
        .filter(([name]) => name !== NONE_OF_THESE)
        .sort((x, y) => y[1] - x[1])[0]?.[0] ?? null;
      const ref = standalone.get(q.id);
      const same = (none >= NONE_GATE_THRESHOLD ? null : top) === (ref.none >= NONE_GATE_THRESHOLD ? null : ref.top);
      if (same) perPosition[i] += 1;
      if (i === k - 1) lastPicks.push(top ?? "拒答");
    }
  }

  console.log(
    `${String(k).padStart(6)} | ${String(tokens ?? "?").padStart(14)} | ` +
      `${perPosition.map((n, i) => `${i + 1}:${n}/${picked.length}`).join(" ").padEnd(19)} | ` +
      `${lastPicks.slice(0, 3).join(", ")}${lastPicks.length > 3 ? ", …" : ""}`,
  );
}

console.log(
  "\n读法：如果每个位置都一致，说明请求还装得下，可以继续往一个请求里塞变体；" +
    "如果只有最后一个位置掉下来，那就是上下文用满了——变体数要减，或改用「每次两张表」。",
);
