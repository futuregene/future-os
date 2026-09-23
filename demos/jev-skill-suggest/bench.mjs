#!/usr/bin/env node
// Compares stage-1 strategies for ranking a 141-skill roster.
//
//   FUTURE_API_KEY=... node bench.mjs                # all strategies, all queries
//   FUTURE_API_KEY=... node bench.mjs noul           # one strategy
//
// Strategies:
//   choice  one Choice over the whole roster (what the demo shipped first)
//   chunk   one Choice per chunk of N skills, all chunks in a single request
//   noul    one Noul per skill, all in a single request (independent probabilities)
//
// Queries with `expect` are covered by a skill; queries without it are expected to be
// refused. `expect` is my own annotation from reading each SKILL.md — a wrong annotation
// would show up as a strategy "failure", so the raw top-5 is always printed too.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { loadRoster } from "./roster.mjs";
import { JevClient } from "./jev.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "skills"));
const client = new JevClient({ apiKey: process.env.FUTURE_API_KEY });

const QUERIES = [
  { text: "把这份季度报告做成一版路演用的 PPT，要能直接导出 PDF", expect: ["future-slides"] },
  { text: "把这个 PDF 里面的表格抽成 markdown", expect: ["future-document"] },
  { text: "帮我生成一张红色狐狸在秋天森林里的插画", expect: ["future-image"] },
  { text: "search the web for the latest Rust release notes", expect: ["future-web"] },
  { text: "find recent papers about CRISPR off-target effects", expect: ["future-paper", "future-deep-research"] },
  { text: "single-cell RNA-seq clustering pipeline with scanpy", expect: ["scanpy"] },
  { text: "build a phylogenetic tree from a multiple sequence alignment", expect: ["phylogenetics"] },
  { text: "differential expression analysis on bulk RNA-seq counts", expect: ["bulk-rnaseq", "differential-expression"] },
  { text: "check whether my experiment has enough replicates for a two-factor design", expect: ["future-experimental-design", "statistical-analysis"] },
  { text: "peer review this manuscript and list the major validity problems", expect: ["future-peer-review"] },
  { text: "解释一下什么是 monad", expect: null },
  { text: "把这三张卡片加到我们的 Trello backlog 里", expect: null },
  { text: "给我讲个笑话", expect: null },
  { text: "帮我订一张下周二去北京的机票", expect: null },
  { text: "今天天气怎么样", expect: null },
];

const CHUNK_SIZE = Number(process.env.CHUNK_SIZE ?? 20);

const chunked = (skills, size) => {
  const chunks = [];
  for (let i = 0; i < skills.length; i += size) chunks.push(skills.slice(i, i + size));
  return chunks;
};

// ------------------------------------------------------------------- strategies

/** One Choice whose criteria map is the entire roster. */
async function strategyChoice(query) {
  const criteria = {};
  for (const skill of roster.skills) criteria[skill.name] = skill.indexLine;
  const { answers, ms, usage } = await client.systemOne({
    state: { request: query },
    questions: {
      best_skill: {
        type: "choice",
        instructions: "Which single skill from `criteria` best fits the user's request in `request`?",
        criteria,
      },
      needs_skill: {
        type: "noul",
        instructions: "Does the request in `request` genuinely call for one of the catalogued skills?",
      },
    },
  });
  const ranked = Object.entries(answers.best_skill?.probabilities || {})
    .map(([name, p]) => ({ name, p }))
    .sort((a, b) => b.p - a.p);
  return { ranked, gate: answers.needs_skill?.noul ?? null, ms, usage, nonzero: ranked.filter((r) => r.p > 0).length };
}

/** One Choice per chunk, all chunks as parallel questions in one request. */
async function strategyChunk(query) {
  const chunks = chunked(roster.skills, CHUNK_SIZE);
  const questions = {};
  chunks.forEach((chunk, i) => {
    const criteria = {};
    for (const skill of chunk) criteria[skill.name] = skill.indexLine;
    questions[`chunk_${i}`] = {
      type: "choice",
      instructions: {
        question: "Which single skill from `criteria` best fits the user's request in `request`, if any of them do?",
        criteria,
      },
      criteria,
    };
  });
  questions.needs_skill = {
    type: "noul",
    instructions: "Does the request in `request` genuinely call for one of the catalogued skills?",
  };

  const { answers, ms, usage } = await client.systemOne({ state: { request: query }, questions });
  const ranked = [];
  chunks.forEach((chunk, i) => {
    const probabilities = answers[`chunk_${i}`]?.probabilities || {};
    for (const [name, p] of Object.entries(probabilities)) ranked.push({ name, p, chunk: i });
  });
  // Chunk probabilities are normalized inside each chunk, so every chunk's best member
  // sits near 1.0 and the cross-chunk order is not comparable — sort, but flag that.
  ranked.sort((a, b) => b.p - a.p);
  return { ranked, gate: answers.needs_skill?.noul ?? null, ms, usage, nonzero: ranked.filter((r) => r.p > 0).length };
}

/** One Noul per skill: independent probabilities, so they are comparable across skills. */
async function strategyNoul(query) {
  const questions = {};
  for (const skill of roster.skills) {
    questions[skill.name] = {
      type: "noul",
      instructions: {
        question: "Would the skill in `skill` materially help with this request?",
        skill: { name: skill.name, description: skill.indexLine },
        request: query,
      },
      criteria: {
        true: "The skill's own description covers what the request needs, or a clearly required first step",
        false: "Only topical overlap, a neighbouring task, or general assistance is enough",
      },
    };
  }
  const { answers, ms, usage } = await client.systemOne({ state: { request: query }, questions });
  const ranked = Object.entries(answers)
    .map(([name, answer]) => ({ name, p: typeof answer?.noul === "number" ? answer.noul : 0 }))
    .sort((a, b) => b.p - a.p);
  return { ranked, gate: ranked[0]?.p ?? null, ms, usage, nonzero: ranked.filter((r) => r.p > 0).length };
}

const STRATEGIES = { choice: strategyChoice, chunk: strategyChunk, noul: strategyNoul };

// ------------------------------------------------------------------------ run

const wanted = process.argv.slice(2).length ? process.argv.slice(2) : Object.keys(STRATEGIES);
const results = {};

for (const name of wanted) {
  console.log(`\n########## ${name} ##########`);
  results[name] = [];
  for (const item of QUERIES) {
    let out;
    try {
      out = await STRATEGIES[name](item.text);
    } catch (error) {
      console.log(`  ERROR ${error.status ?? ""} ${error.message}`);
      results[name].push({ ...item, error: `${error.status}: ${error.message}` });
      continue;
    }
    const top = out.ranked.slice(0, 5).map((entry) => `${entry.name}=${Number(entry.p).toFixed(3)}`);
    const hit = item.expect ? out.ranked.slice(0, 3).some((entry) => item.expect.includes(entry.name)) : null;
    console.log(
      `\n  ${item.expect ? (hit ? "HIT " : "MISS") : "NEG "} ${item.text}\n` +
        `    gate=${out.gate === null ? "—" : Number(out.gate).toFixed(3)} nonzero=${out.nonzero} ms=${out.ms} in=${out.usage?.input_tokens}\n` +
        `    ${top.join("  ")}`,
    );
    results[name].push({
      ...item,
      gate: out.gate,
      nonzero: out.nonzero,
      ms: out.ms,
      input_tokens: out.usage?.input_tokens,
      top: out.ranked.slice(0, 5),
      hit,
    });
  }

  const covered = results[name].filter((r) => r.expect);
  const uncovered = results[name].filter((r) => !r.expect);
  const rate = (list, index) =>
    list.length ? (list.filter((r) => r.top?.slice(0, index).some((e) => r.expect?.includes(e.name))).length / list.length).toFixed(2) : "—";
  console.log(
    `\n  ${name}: top1 ${rate(covered, 1)}  top3 ${rate(covered, 3)}  ` +
      `覆盖题 ${covered.length} 条 · 无关题 ${uncovered.length} 条（top1 均值 ${(uncovered.reduce((s, r) => s + (r.top?.[0]?.p ?? 0), 0) / Math.max(1, uncovered.length)).toFixed(3)}）`,
  );
}

const outPath = path.join(here, "bench-results.json");
const { writeFileSync } = await import("node:fs");
writeFileSync(outPath, JSON.stringify(results, null, 2));
console.log(`\nwrote ${outPath}`);
