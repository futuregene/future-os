#!/usr/bin/env node
// Prompt variants for the single Choice call, measured against each other.
//
// The method matters more than the variants here. Stage 1 alone moves by ~2 questions between two
// identical runs (bench/stage1-stability.mjs), which is as large as any effect a prompt tweak could
// have. So variants are NOT run in separate sessions and compared: several variants go into ONE
// request as separate Choice questions over the same `state`, and every variant is scored against
// the others inside that same request. Two consequences:
//
//   * the comparison is paired, so run-to-run variance cancels;
//   * but the variants share a request, so contamination is possible — the shipped wording is
//     included as a control in every batch, and it is also compared against the standalone
//     recordings in runs/ (runs/stage1-confidence), which measures exactly that.
//
// Variants are grouped into batches small enough to stay well inside Jev's context window, since
// each Choice carries its own full copy of the 141-option table.
//
//   FUTURE_API_KEY=... node stage1-prompt.mjs [--batch A|B|all]
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, cacheFor, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { loadRoster } from "../roster.mjs";
import { NONE_GATE_THRESHOLD, NONE_OF_THESE } from "../suggest.mjs";

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
const standalone = new Map(readAll("stage1-confidence").map((row) => [row.id, row]));

const ids = questions.map((q) => q.id);
const textOf = new Map(questions.map((q) => [q.id, q.text]));
const goldOf = (id) => gold.get(id) ?? [];
const goldFirst = (id) => goldOf(id)[0] ?? null;
const answerable = ids.filter((id) => goldOf(id).length > 0);
const refusable = ids.filter((id) => goldOf(id).length === 0);

// ------------------------------------------------------------------ the variants

/** Option text for one skill. */
const optionText = {
  desc220: (skill) => skill.indexLine,
  desc110: (skill) => skill.description.slice(0, 110),
  descFull: (skill) => skill.description,
  nameAndDesc: (skill) => `${skill.name}: ${skill.indexLine}`,
  descAndCategory: (skill) => `${skill.indexLine} [${skill.category}]`,
};

/** The none option's own wording — the thing the gate reads, so worth its own axis. */
const noneText = {
  plain: () => "No skill in this list would help with the request",
  explicit: () =>
    "The request does not need any of these skills: a general assistant could answer it from " +
    "general knowledge, or the user is asking about a tool that is not in this list",
  terse: () => "None of them",
};

const VARIANTS = {
  // ---- batch A: how the question is asked -------------------------------------------------
  shipped: {
    batch: "A",
    label: "发布版（对照）",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.desc220,
    none: noneText.plain,
  },
  judge_short: {
    batch: "A",
    label: "更短的提问",
    question: "Which skill in `criteria` would help with the request in `request`?",
    howToJudge: `Choose "${NONE_OF_THESE}" if none of them would.`,
    option: optionText.desc220,
    none: noneText.plain,
  },
  judge_bar: {
    batch: "A",
    label: "给\"该选技能\"下明确门槛",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      "Choose a skill only when the request is asking for the task that skill performs. " +
      `If the closest skill only shares the topic, or general assistance would do, choose "${NONE_OF_THESE}".`,
    option: optionText.desc220,
    none: noneText.plain,
  },
  judge_general: {
    batch: "A",
    label: "强调\"通用助手能答就不用技能\"",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Choose "${NONE_OF_THESE}" when the user could be served without any of these skills — ` +
      "because they want an explanation or general advice rather than the task itself, or because " +
      `the tool they name is not in \`criteria\`. Otherwise pick the skill that performs the task.`,
    option: optionText.desc220,
    none: noneText.plain,
  },
  // ---- batch D: where is the description-length knee? -------------------------------------
  // 110 chars loses 4 answers and full/240 gains nothing (batches B, C), but the table is 86% of
  // the request, so a point between 110 and 220 that still holds 61/65 would be a direct cost win.
  // This sweeps the gap.
  shipped_d: {
    batch: "D",
    label: "发布版（对照，第四批）",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.desc220,
    none: noneText.plain,
  },
  desc_150: {
    batch: "D",
    label: "选项描述 150 字符",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: (skill) => skill.description.slice(0, 150),
    none: noneText.plain,
  },
  desc_180: {
    batch: "D",
    label: "选项描述 180 字符",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: (skill) => skill.description.slice(0, 180),
    none: noneText.plain,
  },
  desc_190: {
    batch: "D",
    label: "选项描述 190 字符",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: (skill) => skill.description.slice(0, 190),
    none: noneText.plain,
  },
  // The proposed CHUNK-level default: 256 chars, i.e. 36 above the measured knee. Worth a row
  // because the option table is ~86% of the request, so the length is the one lever that moves
  // total cost — and "full description" already measured *worse* than 220 (batch B).
  desc_256: {
    batch: "D",
    label: "选项描述 256 字符（提议值）",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: (skill) => skill.description.slice(0, 256),
    none: noneText.plain,
  },
  // ---- batch C: isolate one confound from batch A -----------------------------------------
  // `judge_short` changed two things at once: it shortened the wording AND dropped the
  // "not a fallback" sentence. The shipped `question` also presupposes a skill is needed
  // ("needs a skill from `criteria`"), which is exactly the kind of bias that the earlier
  // "A later question decides..." sentence turned out to be. These isolate those two.
  shipped_c: {
    batch: "C",
    label: "发布版（对照，第三批）",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.desc220,
    none: noneText.plain,
  },
  q_no_presuppose: {
    batch: "C",
    label: "去掉「需要技能」的前提",
    question: "Which skill in `criteria`, if any, would help with the request in `request`?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.desc220,
    none: noneText.plain,
  },
  q_ask_first: {
    batch: "C",
    label: "先问「要不要」再问「哪个」",
    question:
      "Does the request in `request` need one of the skills in `criteria`, and if so, which one?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.desc220,
    none: noneText.plain,
  },
  desc_240: {
    batch: "C",
    label: "选项描述放到 240 字符",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: (skill) => skill.description.slice(0, 240),
    none: noneText.plain,
  },
  // ---- batch B continues ------------------------------------------------------------------
  // ---- batch B: what the options say -----------------------------------------------------
  shipped_b: {
    batch: "B",
    label: "发布版（对照，第二批）",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.desc220,
    none: noneText.plain,
  },
  desc_110: {
    batch: "B",
    label: "选项描述减半（110 字符）",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.desc110,
    none: noneText.plain,
  },
  desc_full: {
    batch: "B",
    label: "选项描述不截断",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.descFull,
    none: noneText.plain,
  },
  none_explicit: {
    batch: "B",
    label: "把 none 选项写清楚",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.desc220,
    none: noneText.explicit,
  },
  cat_tag: {
    batch: "B",
    label: "选项里加分类标签",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.descAndCategory,
    none: noneText.plain,
  },

  // ---- batch E: the 256-char variant ON ITS OWN ---------------------------------------------
  // In batch D, `desc_256` refused all 100 questions while the four variants ahead of it behaved
  // normally. Batch D packs five full option tables into one request (median 30,526 tokens), and
  // the 256 table is both the largest and the last question in it, so the plausible reading is that
  // the request ran past the context window and the trailing question degraded — not that 256 chars
  // is a bad length. Same two variants, two tables instead of five, isolates which it is.
  shipped_e: {
    batch: "E",
    label: "发布版（对照，第五批）",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: optionText.desc220,
    none: noneText.plain,
  },
  desc_256_solo: {
    batch: "E",
    label: "选项描述 256 字符（单独请求）",
    question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
    howToJudge:
      `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
      "Choosing it is a normal answer here, not a fallback.",
    option: (skill) => skill.description.slice(0, 256),
    none: noneText.plain,
  },
};

const criteriaFor = (variant) => {
  const criteria = {};
  for (const skill of roster.skills) criteria[skill.name] = variant.option(skill);
  criteria[NONE_OF_THESE] = variant.none();
  return criteria;
};

// ------------------------------------------------------------------ run

const batchArg = (() => {
  const i = process.argv.indexOf("--batch");
  if (i === -1) return process.argv.find((a) => a.startsWith("--batch="))?.split("=")[1] ?? "all";
  return process.argv[i + 1] ?? "all";
})();
const BATCH = batchArg;
if (!["A", "B", "C", "D", "E", "all"].includes(BATCH)) throw new Error(`未知 batch: ${BATCH}`);
const wanted = Object.entries(VARIANTS).filter(([, v]) => BATCH === "all" || v.batch === BATCH);
const cache = cacheFor(`stage1-prompt-${BATCH}`);

const todo = ids.filter((id) => !cache.has(id));
console.log(`prompt 变体实验 · batch ${BATCH} · ${wanted.length} 个变体 · 需要新跑的题 ${todo.length}/100\n`);

for (const id of todo) {
  const questions = Object.fromEntries(wanted.map(([key, v]) => [key, {
    type: "choice",
    instructions: { question: v.question, how_to_judge: v.howToJudge },
    criteria: criteriaFor(v),
  }]));
  const { answers, usage, ms } = await client.systemOne({ state: { request: textOf.get(id) }, questions });

  const record = { id, tokens: usage?.input_tokens ?? null, ms };
  for (const [key] of wanted) {
    const answer = answers[key] ?? {};
    const probabilities = answer.probabilities ?? {};
    const ranked = Object.entries(probabilities)
      .filter(([name]) => name !== NONE_OF_THESE)
      .map(([name, p]) => ({ name, p }))
      .sort((a, b) => b.p - a.p);
    record[key] = { pick: answer.choice ?? null, none: probabilities[NONE_OF_THESE] ?? null, top: ranked.slice(0, 3) };
  }
  cache.put(id, record);
  process.stdout.write(`\r  ${id}    `);
}
console.log("\n");

// ------------------------------------------------------------------ score

const rows = new Map(cache.all().map((r) => [r.id, r]));
const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;
const median = (v) => [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)];

// A variant that is absent from a record is NOT a refusal, but every score below reads
// `!r -> null -> refused`, so a variant left out of the recording silently scores as "refuses
// everything". That is how a variant added to VARIANTS after the cache was already full appeared to
// refuse all 100 questions (it had never run: the cache was keyed by question id, which does not
// change when VARIANTS does). Fail loudly instead of reporting a number that means nothing.
const missing = [];
for (const id of ids) {
  for (const [key] of wanted) {
    if (!rows.get(id)?.[key]) missing.push(`${id}/${key}`);
  }
}
if (missing.length) {
  throw new Error(
    `有 ${missing.length} 个「题 × 变体」组合没有记录，分数会把它当成拒答而全是假的。` +
      `例如：${missing.slice(0, 5).join(", ")}。` +
      `\n  变体是在缓存写满之后才加进去的（缓存按题号索引，加变体不会让它失效）。` +
      `\n  删除 runs/stage1-prompt-${BATCH} 后重跑即可。`,
  );
}

/** Refusal decision: none probability at or above the shipped gate implies "no skill". */
const decision = (id, key, gate = NONE_GATE_THRESHOLD) => {
  const r = rows.get(id)?.[key];
  if (!r) return null;
  return (r.none ?? 0) >= gate ? null : (r.top[0]?.name ?? null);
};

const score = (key, gate = NONE_GATE_THRESHOLD) => {
  let top1 = 0;
  let inRef = 0;
  let refusal = 0;
  let falseRefuse = 0;
  let falseSuggest = 0;
  for (const id of ids) {
    const g = goldOf(id);
    const pred = decision(id, key, gate);
    if (!g.length) {
      if (pred === null) refusal += 1;
      else falseSuggest += 1;
    } else if (pred === g[0]) {
      top1 += 1;
      inRef += 1;
    } else {
      if (pred !== null && g.includes(pred)) inRef += 1;
      if (pred === null) falseRefuse += 1;
    }
  }
  return { top1, inRef, refusal, falseRefuse, falseSuggest };
};

console.log(`=== batch ${BATCH}：同一请求内各变体（门控 ${NONE_GATE_THRESHOLD}）===`);
console.log("  变体                          首答正确        首答∈参照集     正确拒答        误拒    误推");
const results = {};
for (const [key, v] of wanted) {
  const s = score(key);
  results[key] = s;
  console.log(
    `  ${v.label.padEnd(26)} ${String(s.top1).padStart(2)}/65 ${pct(s.top1, 65).padStart(6)}   ${String(s.inRef).padStart(2)}/65 ${pct(s.inRef, 65).padStart(6)}   ${String(s.refusal).padStart(2)}/35 ${pct(s.refusal, 35).padStart(6)}   ${pct(s.falseRefuse, 65).padStart(6)}   ${String(s.falseSuggest).padStart(2)}/35`,
  );
}

// Contamination check: the shipped wording inside this batch vs the standalone recording of the
// identical request (runs/stage1-confidence).
// Derived from the batch, not hard-coded per branch: batch C was added later and a missing key here
// silently made every comparison run against `undefined` (which `decision()` returns null for), so
// the "difference" list was really "questions where the reference recording refused".
const controlKey = { A: "shipped", B: "shipped_b", C: "shipped_c", D: "shipped_d", E: "shipped_e" }[BATCH];
if (!controlKey) throw new Error(`no control variant for batch ${BATCH}`);
const withStandalone = ids.filter((id) => standalone.has(id) && rows.has(id));
const same = withStandalone.filter((id) => {
  const a = standalone.get(id);
  const aDecision = (a.none ?? 0) >= NONE_GATE_THRESHOLD ? null : (a.ranked?.[0]?.name ?? null);
  return aDecision === decision(id, controlKey);
}).length;
console.log(
  `\n  污染检查：本批里的对照变体 vs 独立请求记录（runs/stage1-confidence），判定一致 ${same}/${withStandalone.length} = ${pct(same, withStandalone.length)}`,
);

// Per-question differences against the control, so a change can be inspected rather than trusted.
for (const [key, v] of wanted) {
  if (key === controlKey) continue;
  const differ = ids.filter((id) => decision(id, key) !== decision(id, controlKey));
  if (!differ.length) continue;
  console.log(`\n  ${v.label} 与对照不同的题（${differ.length}）：`);
  for (const id of differ) {
    const g = goldFirst(id) ?? "（拒答）";
    const a = decision(id, controlKey) ?? "拒答";
    const b = decision(id, key) ?? "拒答";
    const aOk = decision(id, controlKey) === goldOf(id)[0];
    const bOk = decision(id, key) === goldOf(id)[0];
    console.log(
      `    ${id}  金标题 ${String(g).padEnd(22)} 对照 ${String(a).padEnd(22)} 本变体 ${String(b).padEnd(22)} ${aOk === bOk ? "（得分相同）" : bOk ? "✓ 变体对" : "✗ 变体错"}`,
    );
  }
}

// Cost: each variant carries its own copy of the option table, so length shows up directly.
console.log(`\n=== 单个变体在本次请求里的选项表规模 ===`);
for (const [key, v] of wanted) {
  const chars = [...roster.skills.map((s) => v.option(s)), v.none()].join("").length;
  console.log(`  ${v.label.padEnd(26)} ${String(chars).padStart(7)} 字符  ≈ ${String(Math.round(chars / 4)).padStart(6)} token`);
}
console.log(`\n  本次请求一次问 ${wanted.length} 个变体，合计 token 中位 ${median([...rows.values()].map((r) => r.tokens))}`);
