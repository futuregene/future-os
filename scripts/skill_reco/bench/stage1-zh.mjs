#!/usr/bin/env node
// The mirror of stage2-zh.mjs: if the Chinese line earns nothing in stage 2, does it earn anything in
// stage 1, where the option line is English-only (`indexLine`, a 220-char description truncation)?
//
// Both variants are asked in the SAME request as two Choice questions over the same 141 skills, so
// the comparison is paired: same state, same options, same moment. Read for both:
//
//   * rank 1 — whose probability wins, and whether it is the reference answer;
//   * recall@3 — whether the reference answer is in the top three stage 2 would receive. On a
//     saturated Choice this is mostly "winner + zero fillers", so it is reported but not leaned on;
//   * the none probability, which is what the gate actually reads.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, cacheFor, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { loadRoster } from "../roster.mjs";
import { NONE_OF_THESE } from "../suggest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;
const client = new JevClient({ apiKey: process.env.FUTURE_API_KEY });

const readAll = (dir) =>
  fs
    .readdirSync(path.join(RUNS_DIR, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, dir, f), "utf8")));
const gold = new Map(readAll("gold-v1").map((row) => [row.id, row.gold]));

const criteriaFor = ({ zh }) => {
  const criteria = {};
  for (const skill of roster.skills) {
    criteria[skill.name] = zh
      ? `${skill.indexLine}${skill.descriptionZh ? ` / ${skill.descriptionZh}` : ""}`
      : skill.indexLine;
  }
  criteria[NONE_OF_THESE] = "No skill in this list would help with the request";
  return criteria;
};

const askFor = (name, { zh }) => ({
  [name]: {
    type: "choice",
    instructions: {
      question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
      how_to_judge: `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". Choosing it is a normal answer here, not a fallback.`,
    },
    criteria: criteriaFor({ zh }),
  },
});

const ids = questions.map((q) => q.id);
const textOf = new Map(questions.map((q) => [q.id, q.text]));
const goldOf = (id) => gold.get(id) ?? [];
const goldFirst = (id) => goldOf(id)[0] ?? null;
const answerable = ids.filter((id) => goldOf(id).length > 0);
const refusable = ids.filter((id) => goldOf(id).length === 0);

const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;
const median = (v) => {
  const s = v.filter((x) => typeof x === "number").sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
};

const cache = cacheFor("stage1-zh-ab");
const todo = ids.filter((id) => !cache.has(id));
console.log(`stage 1 选项表 A/B（英文 indexLine vs 加上 34 字中文摘要，同一个请求内成对提问）`);
console.log(`需要新跑的题：${todo.length}/100（已缓存 ${100 - todo.length}）\n`);

for (const id of todo) {
  const { answers, usage, ms } = await client.systemOne({
    state: { request: textOf.get(id) },
    questions: { ...askFor("chunk_en", { zh: false }), ...askFor("chunk_zh", { zh: true }) },
  });
  const ranked = (answerName) => {
    const probabilities = answers[answerName]?.probabilities ?? {};
    return {
      none: probabilities[NONE_OF_THESE] ?? null,
      ranked: Object.entries(probabilities)
        .filter(([name]) => name !== NONE_OF_THESE)
        .map(([name, p]) => ({ name, p }))
        .sort((a, b) => b.p - a.p),
    };
  };
  cache.put(id, {
    id,
    en: ranked("chunk_en"),
    zh: ranked("chunk_zh"),
    tokens: usage?.input_tokens ?? null,
    ms,
  });
  process.stdout.write(`\r  ${id}    `);
}
console.log("\n");

const rows = new Map(cache.all().map((r) => [r.id, r]));
const variants = [
  ["英文 indexLine（发布）", "en"],
  ["加上中文摘要", "zh"],
];

console.log("=== 只看 stage 1（选项表两种写法）===");
console.log("  选项表写法            Top-1 正确          在 Top-3 内            拒答正确            误拒");
for (const [label, key] of variants) {
  const top1 = answerable.filter((id) => (rows.get(id)?.[key]?.ranked?.[0]?.name ?? null) === goldFirst(id)).length;
  const top3 = answerable.filter((id) => (rows.get(id)?.[key]?.ranked ?? []).slice(0, 3).some((e) => e.name === goldFirst(id))).length;
  // The gate reads the none probability; 0.15 is the shipped threshold.
  const refused = refusable.filter((id) => (rows.get(id)?.[key]?.none ?? 0) >= 0.15).length;
  const falseRefuse = answerable.filter((id) => (rows.get(id)?.[key]?.none ?? 0) >= 0.15).length;
  console.log(
    `  ${label.padEnd(20)}${String(top1).padStart(2)}/65 = ${pct(top1, 65).padStart(6)}      ${String(top3).padStart(2)}/65 = ${pct(top3, 65).padStart(6)}      ${String(refused).padStart(2)}/35 = ${pct(refused, 35).padStart(6)}      ${pct(falseRefuse, 65)}`,
  );
}

const enTop1 = answerable.filter((id) => (rows.get(id)?.en?.ranked?.[0]?.name ?? null) === goldFirst(id)).length;
const zhTop1 = answerable.filter((id) => (rows.get(id)?.zh?.ranked?.[0]?.name ?? null) === goldFirst(id)).length;
const flipped = answerable.filter(
  (id) => (rows.get(id)?.en?.ranked?.[0]?.name ?? null) !== (rows.get(id)?.zh?.ranked?.[0]?.name ?? null),
);
console.log(`\n  第一名不同：${flipped.length}/65  ${flipped.map((id) => `${id}(英 ${rows.get(id).en.ranked[0]?.name} → 中 ${rows.get(id).zh.ranked[0]?.name})`).join(" · ") || ""}`);
const noneDelta = ids.map((id) => (rows.get(id)?.zh?.none ?? 0) - (rows.get(id)?.en?.none ?? 0));
console.log(`  none 概率之差：均值 ${(noneDelta.reduce((a, b) => a + b, 0) / noneDelta.length).toFixed(4)}  中位绝对差 ${median(noneDelta.map(Math.abs)).toFixed(4)}`);
console.log(`  每题 token 中位 ${median([...rows.values()].map((r) => r.tokens))}（此请求同时问了两套，单套约一半）`);

console.log(`\n=== 结论 ===`);
if (enTop1 === zhTop1) {
  console.log(`  两种写法的 Top-1 完全相同（都是 ${enTop1}/65）→ 中文摘要对 stage 1 也没有可测的价值。`);
} else {
  console.log(`  英文 ${enTop1}/65 vs 加中文 ${zhTop1}/65 → 差 ${Math.abs(zhTop1 - enTop1)} 道，留待更大样本确认。`);
}
