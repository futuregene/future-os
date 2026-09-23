#!/usr/bin/env node
// Does adding an escape option besides none_of_these help?
//
// none_of_these covers exactly one case: nothing in the list fits. But a request can also be served
// by SEVERAL skills (12 of the 65 answerable questions have a multi-skill reference set), and a
// request can need a skill that simply is not in the catalogue. The question is whether giving the
// Choice somewhere to put those cases changes the answer.
//
// Two mechanisms could make it help, and they point in opposite directions:
//
//   * the gate reads the none probability, and probabilities sum to 1 — so a new option that absorbs
//     mass LOWERS none, which could push a borderline question back under the threshold;
//   * but a new option also gives the model a way to avoid committing, which could spread the skill
//     probabilities and cost first-answer accuracy.
//
// So the option set varies and the decision rule does not: every variant is scored under the shipped
// rule (refuse if none >= gate, else the top-probability skill), plus a second reading where the
// model's own choice of an escape option counts as a refusal. The two mechanisms can only be told
// apart by looking at the none distribution and at the skill ranking, so FULL probabilities are
// recorded and every table below is computed offline from that recording.
//
// Same-request pairing as the prompt sweep (run-to-run variance moves stage 1 by ~2 questions), with
// the shipped option set as the control in every request. bench/ctx-limit.mjs is why four tables in
// one request is safe.
//
//   FUTURE_API_KEY=... node bench/escape-options.mjs
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

const readAll = (dir) =>
  fs
    .readdirSync(path.join(RUNS_DIR, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, dir, f), "utf8")));
const gold = new Map(readAll("gold-v1").map((r) => [r.id, r.gold]));
const standalone = new Map(readAll("stage1-confidence").map((r) => [r.id, r]));

const ids = questions.map((q) => q.id);
const textOf = new Map(questions.map((q) => [q.id, q.text]));
const goldOf = (id) => gold.get(id) ?? [];
const answerable = ids.filter((id) => goldOf(id).length > 0);
const refusable = ids.filter((id) => goldOf(id).length === 0);

// ------------------------------------------------------------------ the variants

const SEVERAL = "several_of_these";
const NOT_LISTED = "not_in_this_list";

/** The extra escape options, as data: their existence and their WORDING are separate axes. */
const ESCAPES = {
  [SEVERAL]: "More than one of these skills would help with the request",
  [NOT_LISTED]: "The skill this request needs is not in this list",
};

const VARIANTS = {
  shipped: { label: "发布版（对照）", extra: [] },
  multi: { label: "＋「不止一个合适」", extra: [SEVERAL] },
  other: { label: "＋「需要的技能不在这张表里」", extra: [NOT_LISTED] },
  multi_other: { label: "＋两者都加", extra: [SEVERAL, NOT_LISTED] },
};

const ESCAPE_NAMES = new Set([NONE_OF_THESE, SEVERAL, NOT_LISTED]);

const criteriaFor = (variant) => {
  const criteria = {};
  for (const skill of roster.skills) criteria[skill.name] = skill.indexLine;
  criteria[NONE_OF_THESE] = "No skill in this list would help with the request";
  for (const name of variant.extra) criteria[name] = ESCAPES[name];
  return criteria;
};

const INSTRUCTIONS = {
  question: "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
  how_to_judge:
    `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
    "Choosing it is a normal answer here, not a fallback.",
};

// ------------------------------------------------------------------ run

const cache = cacheFor("escape-options");
const wanted = Object.entries(VARIANTS);
const token = process.env.FUTURE_API_KEY ?? "";
if (!token) {
  console.error("FUTURE_API_KEY is not set");
  process.exit(2);
}
const client = new JevClient({ apiKey: token });
const todo = ids.filter((id) => !cache.has(id));
console.log(`escape 选项实验 · ${wanted.length} 个变体 · 需要新跑的题 ${todo.length}/${ids.length}\n`);

for (const id of todo) {
  const questionsObj = Object.fromEntries(
    wanted.map(([key, v]) => [key, { type: "choice", instructions: INSTRUCTIONS, criteria: criteriaFor(v) }]),
  );
  const { answers, usage, ms } = await client.systemOne({ state: { request: textOf.get(id) }, questions: questionsObj });

  const record = { id, tokens: usage?.input_tokens ?? null, ms };
  for (const [key] of wanted) {
    const a = answers?.[key] ?? {};
    // Full probabilities, not a shortlist: the none distribution and the ranking are what the
    // analysis needs, and recomputing them from a shortlist is impossible.
    record[key] = { pick: a.choice ?? null, probabilities: a.probabilities ?? {} };
  }
  cache.put(id, record);
  process.stdout.write(`\r  ${id}    `);
}
console.log("\n");

// ------------------------------------------------------------------ offline analysis

const rows = new Map(cache.all().map((r) => [r.id, r]));
const missing = ids.filter((id) => wanted.some(([key]) => !rows.get(id)?.[key]));
if (missing.length) throw new Error(`${missing.length} 道题缺少记录，删掉 ${cache.dir} 重跑。`);

const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;
const median = (v) => [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)];

const skillsOf = (id, key) =>
  Object.entries(rows.get(id)[key].probabilities)
    .filter(([name]) => !ESCAPE_NAMES.has(name))
    .map(([name, p]) => ({ name, p }))
    .sort((a, b) => b.p - a.p);
const escapeProbability = (id, key, name) => rows.get(id)[key].probabilities[name] ?? 0;

/**
 * `escapeRefuses` false = the shipped reading: the gate reads none, and the model's own choice of an
 * escape option is ignored. true = the model's verdict counts, so picking any escape option refuses.
 */
const decide = (id, key, { escapeRefuses = false, gate = NONE_GATE_THRESHOLD } = {}) => {
  const r = rows.get(id)[key];
  if (escapeRefuses) {
    const extra = VARIANTS[key].extra;
    for (const name of extra) {
      if ((r.probabilities[name] ?? 0) >= gate) return null;
      if (r.pick === name) return null;
    }
  }
  if ((r.probabilities[NONE_OF_THESE] ?? 0) >= gate) return null;
  return skillsOf(id, key)[0]?.name ?? null;
};

const score = (key, opts) => {
  let top1 = 0;
  let inRef = 0;
  let refusal = 0;
  let falseRefuse = 0;
  let falseSuggest = 0;
  for (const id of ids) {
    const g = goldOf(id);
    const pred = decide(id, key, opts);
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

console.log("=== 1. 各变体在发布版规则下（门控只看 none ≥ 0.15，答案是概率最高的技能）===\n");
console.log("  变体                            首答正确        首答∈参照集     正确拒答        误拒    误推");
for (const [key, v] of wanted) {
  const s = score(key, {});
  console.log(
    `  ${v.label.padEnd(28)} ${String(s.top1).padStart(2)}/65 ${pct(s.top1, 65).padStart(6)}   ` +
      `${String(s.inRef).padStart(2)}/65 ${pct(s.inRef, 65).padStart(6)}   ` +
      `${String(s.refusal).padStart(2)}/35 ${pct(s.refusal, 35).padStart(6)}   ` +
      `${pct(s.falseRefuse, 65).padStart(6)}   ${String(s.falseSuggest).padStart(2)}/35`,
  );
}

console.log("\n\n=== 2. 如果「模型选了 escape 选项」也算拒答 ===\n");
console.log("  变体                            首答正确        首答∈参照集     正确拒答        误拒    误推");
for (const [key, v] of wanted) {
  const s = score(key, { escapeRefuses: true });
  console.log(
    `  ${v.label.padEnd(28)} ${String(s.top1).padStart(2)}/65 ${pct(s.top1, 65).padStart(6)}   ` +
      `${String(s.inRef).padStart(2)}/65 ${pct(s.inRef, 65).padStart(6)}   ` +
      `${String(s.refusal).padStart(2)}/35 ${pct(s.refusal, 35).padStart(6)}   ` +
      `${pct(s.falseRefuse, 65).padStart(6)}   ${String(s.falseSuggest).padStart(2)}/35`,
  );
}

console.log("\n\n=== 3. 新的 escape 选项被用到了什么程度 ===\n");
for (const [key, v] of wanted.filter(([, v]) => v.extra.length)) {
  console.log(`  ${v.label}`);
  for (const name of v.extra) {
    const picked = ids.filter((id) => rows.get(id)[key].pick === name).length;
    const probs = ids.map((id) => escapeProbability(id, key, name));
    console.log(
      `    ${name.padEnd(16)} 被选中 ${String(picked).padStart(3)}/100 次   ` +
        `概率 中位 ${median(probs).toFixed(2)}  最大 ${Math.max(...probs).toFixed(2)}  非零 ${probs.filter((p) => p > 0.01).length}/100`,
    );
    // An escape option the model actually picked is the strongest possible statement of its verdict,
    // so what the shipped rule (which only reads none) does with it matters.
    for (const id of ids.filter((i) => rows.get(i)[key].pick === name)) {
      const none = escapeProbability(id, key, NONE_OF_THESE);
      const decided = decide(id, key, {});
      console.log(
        `      ${id}  参照 ${(goldOf(id)[0] ?? "（拒答）").padEnd(22)} none=${none.toFixed(2)}  ` +
          `发布版规则判为 ${decided ?? "拒答"}${decided ? "（把模型的明确弃权当没看见）" : "（与模型一致）"}`,
      );
    }
  }
}

console.log("\n\n=== 4. none 概率整体是否被摊薄 ===\n");
console.log("  变体                           该推荐的 none（最大/中位）  该拒答的 none（最小/中位）  两者之间的空带");
for (const [key, v] of wanted) {
  const a = answerable.map((id) => escapeProbability(id, key, NONE_OF_THESE)).sort((x, y) => x - y);
  const r = refusable.map((id) => escapeProbability(id, key, NONE_OF_THESE)).sort((x, y) => x - y);
  // The refusable minimum is the one that matters for the gate: it is what a threshold has to reach.
  const rMin = r.find((p) => p > 0.1) ?? r[0];
  console.log(
    `  ${v.label.padEnd(28)} ${Math.max(...a).toFixed(2)} / ${median(a).toFixed(2)}`.padEnd(62) +
      `${rMin.toFixed(2)} / ${median(r).toFixed(2)}`.padEnd(30) +
      `${Math.max(...a).toFixed(2)} → ${rMin.toFixed(2)}`,
  );
}

console.log("\n\n=== 5. 与对照不同的题（发布版规则下）===\n");
for (const [key, v] of wanted) {
  if (key === "shipped") continue;
  const differ = ids.filter((id) => decide(id, key, {}) !== decide(id, "shipped", {}));
  console.log(`  ${v.label}：${differ.length} 道`);
  for (const id of differ) {
    const g = goldOf(id);
    const a = decide(id, "shipped", {});
    const b = decide(id, key, {});
    const aOk = g.length ? a === g[0] : a === null;
    const bOk = g.length ? b === g[0] : b === null;
    const mark = aOk === bOk ? "（得分相同）" : bOk ? "✓ 变体对" : "✗ 变体错";
    console.log(
      `    ${id}  参照 ${(g[0] ?? "（拒答）").padEnd(24)} 对照 ${String(a ?? "拒答").padEnd(24)} 本变体 ${String(b ?? "拒答").padEnd(24)} ${mark}`,
    );
  }
}

console.log("\n\n=== 6. 那 12 道「参照集里有多个答案」的题，escape 选项改变了什么 ===\n");
const multiRef = answerable.filter((id) => goldOf(id).length > 1);
for (const [key, v] of wanted) {
  const hits = multiRef.filter((id) => goldOf(id).includes(decide(id, key, {}))).length;
  const refused = multiRef.filter((id) => decide(id, key, {}) === null).length;
  // Matching the reference set is the weak test; the product needs ONE skill, so the interesting
  // number is how often it is also the reference's own first choice.
  const firsts = multiRef.filter((id) => decide(id, key, {}) === goldOf(id)[0]).length;
  console.log(
    `  ${v.label.padEnd(28)} 命中参照集 ${hits}/${multiRef.length}   与参照首答一致 ${firsts}/${multiRef.length}   拒答 ${refused}/${multiRef.length}`,
  );
}

console.log("\n\n=== 7. 摊薄是可以靠调门控补回来的吗（每个变体扫自己的最优阈值）===\n");
console.log("  门控 | 发布版（对照）         | ＋不止一个             | ＋不在表里");
for (const gate of [0.05, 0.08, 0.1, 0.12, 0.15, 0.2, 0.3]) {
  const cells = ["shipped", "multi", "other"].map((key) => {
    const s = score(key, { gate });
    return `${String(s.top1).padStart(2)}/65 拒${String(s.refusal).padStart(2)}/35 误推${s.falseSuggest}`;
  });
  console.log(`  ${String(gate).padStart(4)} | ${cells[0].padEnd(21)} | ${cells[1].padEnd(21)} | ${cells[2]}`);
}

console.log("\n\n=== 8. 唯一那道被改变判定的题（n026）到底发生了什么 ===\n");
{
  const id = "n026";
  const text = (textOf.get(id) ?? "").slice(0, 90);
  console.log(`  题目：${text}…`);
  console.log(`  参照：${JSON.stringify(goldOf(id))}（即该拒答）`);
  for (const [key, v] of wanted) {
    const r = rows.get(id)[key];
    const norm = Object.entries(r.probabilities)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 4)
      .map(([n, p]) => `${n}=${p.toFixed(2)}`)
      .join("  ");
    console.log(`    ${v.label.padEnd(28)} 选中 ${String(r.pick).padEnd(22)} ${norm}`);
  }
  console.log(
    "  说明：加了「不在表里」之后，本该落在 none 上的概率被它分走了，" +
      "none 从 ≥0.15 掉到 0.14 以下，于是门控放行、推荐了一个技能——**同一个模型判断，只是归一化变了**。",
  );
}

// Contamination check, as in the prompt sweep.
const withStandalone = ids.filter((id) => standalone.has(id) && rows.has(id));
const same = withStandalone.filter((id) => {
  const a = standalone.get(id);
  const aDecision = (a.none ?? 0) >= NONE_GATE_THRESHOLD ? null : (a.ranked?.[0]?.name ?? null);
  return aDecision === decide(id, "shipped", {});
}).length;
console.log(
  `\n\n  污染检查：本请求里的对照 vs 独立请求记录（runs/stage1-confidence），判定一致 ${same}/${withStandalone.length} = ${pct(same, withStandalone.length)}`,
);
