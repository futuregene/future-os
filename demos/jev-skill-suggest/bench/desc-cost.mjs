#!/usr/bin/env node
// What does description length actually cost per request?
//
// The option table is ~86% of a stage-1 request, so its length is the one lever that moves total
// cost. The prompt sweep measures QUALITY per length; this measures the PRICE, by sending one
// table at a time and reading `usage.input_tokens` — one variant per request, so the request's
// whole input count belongs to that variant (no need to subtract anything).
//
//   TYPESAFE_API_KEY=... node bench/desc-cost.mjs [lengths...]
//
// Only a few questions are needed: the table size does not depend on the question, so the marginal
// cost is the same for all of them and the spread across questions is pure noise.
import { JevClient } from "../jev.mjs";
import { loadRoster } from "../roster.mjs";
import { NONE_OF_THESE } from "../suggest.mjs";
import { QUESTIONS_FILE, readJson } from "./common.mjs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions.slice(0, 5);

const argLengths = process.argv.slice(2).map(Number).filter(Boolean);
const lengths = argLengths.length ? argLengths : [110, 220, 256, 0]; // 0 = full description

const client = new JevClient({ apiKey: process.env.TYPESAFE_API_KEY });
if (!client.apiKey) {
  console.error("TYPESAFE_API_KEY is not set");
  process.exit(2);
}

const optionFor = (skill, chars) => {
  // `description` is already one-lined by the roster (roster.mjs), and `indexLine` is exactly
  // `description.slice(0, 220)` — so slicing the same string at different lengths varies ONLY the
  // length, which is the axis under test. (Comparing against `excerptRaw` would change the
  // whitespace shape too, which is a different, already-measured axis.)
  return chars ? skill.description.slice(0, chars) : skill.description;
};

const rows = [];
for (const chars of lengths) {
  const criteria = {};
  for (const skill of roster.skills) criteria[skill.name] = optionFor(skill, chars);
  criteria[NONE_OF_THESE] = "No skill in this list would help with the request";

  const tokens = [];
  const charsInTable = Object.values(criteria)
    .filter((v) => v !== criteria[NONE_OF_THESE])
    .reduce((n, v) => n + v.length, 0);
  for (const q of questions) {
    const { usage } = await client.systemOne({
      state: { request: q.text },
      questions: {
        chunk_0: {
          type: "choice",
          instructions: {
            question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
            how_to_judge:
              `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
              "Choosing it is a normal answer here, not a fallback.",
          },
          criteria,
        },
      },
    });
    tokens.push(usage?.input_tokens ?? null);
  }
  const med = [...tokens].sort((a, b) => a - b)[Math.floor(tokens.length / 2)];
  rows.push({ chars, tableChars: charsInTable, tokens, median: med });
}

const base = rows.find((r) => r.chars === 220) ?? rows[0];
console.log("\n选项表长度 → 每次请求的 input_tokens（单变体请求，5 道题的中位）：\n");
console.log("描述截断 | 选项表字符数 | 每题中位 token | 相对 220 | 每题费用 | 10 万次/天");
console.log("--------|------------|--------------|---------|---------|-----------");
for (const r of rows) {
  const delta = r.median - base.median;
  const cost = (r.median / 1e6) * 0.042 * 7.2;
  console.log(
    `${String(r.chars || "不截断").padStart(7)} | ${String(r.tableChars).padStart(11)} | ` +
      `${String(r.median).padStart(12)} | ${(delta >= 0 ? "+" : "") + delta} | ¥${cost.toFixed(4)} | ¥${Math.round(cost * 100000).toLocaleString()}`,
  );
  if (r.chars !== 220 && r.tokens.length > 1) {
    const spread = Math.max(...r.tokens) - Math.min(...r.tokens);
    if (spread > 40) console.log(`${" ".repeat(8)}（5 次之间最大差 ${spread} token，属于请求间噪音）`);
  }
}
