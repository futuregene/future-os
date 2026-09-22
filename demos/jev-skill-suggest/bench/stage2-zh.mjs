#!/usr/bin/env node
// Does the Chinese one-liner in the stage-2 payload earn its ~110 tokens?
//
// `chinese_description` comes from the skill catalog (`skills.json`), not from SKILL.md, and it is a
// 34-character compression of a ~380-character English description. Two ways to ask whether it
// helps, and both are here because they answer different questions:
//
//   --ab    Paired A/B inside ONE request: for every candidate, ask the fit question twice, once
//           with the Chinese line and once without (fit_i_zh / fit_i_nozh). Same request, same
//           state, same cost — so any difference is the payload and not sampling drift. This is
//           the sensitive measurement; it can see a 0.02 shift.
//   --full  End-to-end: rebuild the whole stage-2 decision with the Chinese line removed and score
//           it against the 100-question reference set. This is the one that says whether the final
//           answer (and the refusal) changes at all.
//
// Candidates are read from the cached stage-1 run, so both variants see exactly the shortlist the
// shipped pipeline would have produced.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, cacheFor, readJson } from "./common.mjs";
import { JevClient } from "../jev.mjs";
import { loadRoster } from "../roster.mjs";
import { FITS_THRESHOLD } from "./second-call.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;
const client = new JevClient({ apiKey: process.env.TYPESAFE_API_KEY });

const readAll = (dir) =>
  fs
    .readdirSync(path.join(RUNS_DIR, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, dir, f), "utf8")));
const pipeline = new Map(readAll("jev").map((row) => [row.id, row]));
const gold = new Map(readAll("gold-v1").map((row) => [row.id, row.gold]));

const QUESTION = "Would the skill in `skill` materially help with the request in `request`?";
const CRITERIA = {
  true: "The skill's own description or instructions cover what the request needs, directly or as a clearly required first step",
  false: "Only topical overlap, a neighbouring task, or general assistance would be enough",
};

const skillPayload = (name, { zh }) => {
  const skill = roster.byName.get(name);
  return {
    name,
    description: skill?.description || "",
    ...(zh ? { chinese_description: skill?.descriptionZh || "" } : {}),
    instructions_excerpt: skill?.excerpt || "",
  };
};

const questionsFor = (candidates, { zh }) =>
  Object.fromEntries(
    candidates.map((name, i) => [
      zh ? `fit_${i}_zh` : `fit_${i}_nozh`,
      { type: "noul", instructions: { question: QUESTION, skill: skillPayload(name, { zh }) }, criteria: CRITERIA },
    ]),
  );

const ids = questions.map((q) => q.id);
const textOf = new Map(questions.map((q) => [q.id, q.text]));
const gatePasses = (id) => (pipeline.get(id)?.gate?.length ?? 0) > 0;
const candidatesOf = (id) => (pipeline.get(id)?.stage1_top ?? []).slice(0, 3).map((e) => e.name);
const passed = ids.filter(gatePasses);

const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;
const median = (v) => {
  const s = v.filter((x) => typeof x === "number").sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
};
const mean = (v) => v.reduce((a, b) => a + b, 0) / v.length;

const MODE = process.argv.includes("--full") ? "full" : "ab";

if (MODE === "ab") {
  // ---------------------------------------------------------- paired A/B
  const cache = cacheFor("stage2-zh-ab");
  const todo = passed.filter((id) => !cache.has(id));
  console.log(`stage 2 载荷 A/B（带中文 vs 不带中文，同一个请求内成对提问）`);
  console.log(`需要新跑的题：${todo.length}/${passed.length}（已缓存 ${passed.length - todo.length}）\n`);

  for (const id of todo) {
    const candidates = candidatesOf(id);
    const { answers, usage, ms } = await client.systemOne({
      state: { request: textOf.get(id) },
      questions: { ...questionsFor(candidates, { zh: true }), ...questionsFor(candidates, { zh: false }) },
    });
    cache.put(id, {
      id,
      candidates,
      withZh: candidates.map((_, i) => answers[`fit_${i}_zh`]?.noul ?? null),
      withoutZh: candidates.map((_, i) => answers[`fit_${i}_nozh`]?.noul ?? null),
      tokens: usage?.input_tokens ?? null,
      ms,
    });
    process.stdout.write(`\r  ${id}    `);
  }
  console.log("\n");

  const rows = cache.all();
  const pairs = []; // one entry per (question, candidate)
  for (const row of rows) {
    row.candidates.forEach((name, i) => {
      if (row.withZh[i] !== null && row.withoutZh[i] !== null) {
        pairs.push({ id: row.id, name, zh: row.withZh[i], nozh: row.withoutZh[i], delta: row.withZh[i] - row.withoutZh[i] });
      }
    });
  }

  const absDelta = pairs.map((p) => Math.abs(p.delta));
  console.log(`=== 逐候选对比（${pairs.length} 个「题 × 候选」配对）===`);
  console.log(`  带中文   均值 ${mean(pairs.map((p) => p.zh)).toFixed(3)}  中位 ${median(pairs.map((p) => p.zh)).toFixed(3)}`);
  console.log(`  不带中文 均值 ${mean(pairs.map((p) => p.nozh)).toFixed(3)}  中位 ${median(pairs.map((p) => p.nozh)).toFixed(3)}`);
  console.log(`  差值的均值 ${mean(pairs.map((p) => p.delta)).toFixed(4)}（正=带中文时更高）  中位绝对差 ${median(absDelta).toFixed(4)}  最大绝对差 ${Math.max(...absDelta).toFixed(3)}`);
  const big = pairs.filter((p) => Math.abs(p.delta) >= 0.1);
  console.log(`  |差| ≥ 0.10 的配对：${big.length}/${pairs.length} = ${pct(big.length, pairs.length)}`);

  // Does the winner change?
  let winnerChanged = 0;
  const changed = [];
  for (const row of rows) {
    const argmax = (v) => row.candidates[v.indexOf(Math.max(...v))];
    const a = argmax(row.withZh);
    const b = argmax(row.withoutZh);
    if (a !== b) {
      winnerChanged += 1;
      changed.push(`${row.id}: ${a} → ${b}`);
    }
  }
  console.log(`\n  赢家改变：${winnerChanged}/${rows.length} = ${pct(winnerChanged, rows.length)}  ${changed.join(" · ") || ""}`);

  // Decision changes, at the shipped threshold.
  let decisionChanged = 0;
  const decisionDiffs = [];
  for (const row of rows) {
    const decide = (v) => {
      const best = Math.max(...v);
      const name = row.candidates[v.indexOf(best)];
      return best >= FITS_THRESHOLD ? name : null;
    };
    const a = decide(row.withZh);
    const b = decide(row.withoutZh);
    if (a !== b) {
      decisionChanged += 1;
      decisionDiffs.push(`${row.id}: ${a ?? "拒答"} → ${b ?? "拒答"}`);
    }
  }
  console.log(`  最终判定改变：${decisionChanged}/${rows.length}  ${decisionDiffs.join(" · ") || ""}`);
  console.log(`  每题 token 中位 ${median(rows.map((r) => r.tokens))}（此请求同时问了两套，实际单套约一半）`);
} else {
  // ---------------------------------------------------------- end to end
  const cache = cacheFor("stage2-zh-nozh");
  const todo = passed.filter((id) => !cache.has(id));
  console.log(`端到端：把中文描述从 stage 2 里去掉，重跑全部决策`);
  console.log(`需要新跑的题：${todo.length}/${passed.length}\n`);

  for (const id of todo) {
    const candidates = candidatesOf(id);
    const { answers, usage, ms } = await client.systemOne({
      state: { request: textOf.get(id) },
      questions: questionsFor(candidates, { zh: false }),
    });
    cache.put(id, {
      id,
      candidates,
      fits: candidates.map((_, i) => answers[`fit_${i}_nozh`]?.noul ?? null),
      tokens: usage?.input_tokens ?? null,
      ms,
    });
    process.stdout.write(`\r  ${id}    `);
  }
  console.log("\n");

  const rows = new Map(cache.all().map((r) => [r.id, r]));
  const decide = (candidates, fits) => {
    const ranked = candidates.map((name, i) => ({ name, fit: fits[i] ?? 0 })).sort((a, b) => b.fit - a.fit);
    return ranked[0].fit >= FITS_THRESHOLD ? ranked[0].name : null;
  };

  const answerable = ids.filter((id) => (gold.get(id) ?? []).length > 0);
  const refusable = ids.filter((id) => (gold.get(id) ?? []).length === 0);
  const goldFirst = (id) => gold.get(id)[0] ?? null;

  const scoreboard = (label, answerOf) => {
    let agree = 0;
    let top1 = 0;
    let refusal = 0;
    let falseRefuse = 0;
    for (const id of ids) {
      const g = gold.get(id) ?? [];
      const pred = answerOf(id);
      if (!g.length) {
        if (!pred) {
          agree += 1;
          refusal += 1;
        }
      } else if (pred === g[0]) {
        agree += 1;
        top1 += 1;
      } else if (!pred) {
        falseRefuse += 1;
      }
    }
    console.log(
      `  ${label.padEnd(26)} 决策一致 ${pct(agree, 100).padStart(6)} 首答正确 ${String(top1).padStart(2)}/65 = ${pct(top1, 65).padStart(6)}` +
        ` 正确拒答 ${String(refusal).padStart(2)}/35 = ${pct(refusal, 35).padStart(6)} 误拒 ${pct(falseRefuse, 65)}`,
    );
    return { agree, top1, refusal, falseRefuse };
  };

  const shipped = (id) => pipeline.get(id)?.answer?.[0] ?? null;
  console.log("=== 端到端（100 题）===");
  const a = scoreboard("发布版（带中文描述）", (id) => (gatePasses(id) ? shipped(id) : null));
  const b = scoreboard("去中文（不带 chinese_description）", (id) => (gatePasses(id) ? decide(rows.get(id)?.candidates ?? [], rows.get(id)?.fits ?? []) : null));

  const diffs = passed.filter((id) => (shipped(id) ?? null) !== decide(rows.get(id)?.candidates ?? [], rows.get(id)?.fits ?? []));
  console.log(`\n  逐题差异：${diffs.length} 道  ${diffs.map((id) => `${id}(发布 ${shipped(id) ?? "拒答"} → 去中文 ${decide(rows.get(id)?.candidates ?? [], rows.get(id)?.fits ?? []) ?? "拒答"})`).join(" · ") || "无"}`);
  console.log(`  每题 token：带中文 ${median(ids.filter(gatePasses).map((id) => pipeline.get(id)?.stage2_tokens ?? 0))} → 不带中文 ${median(passed.map((id) => rows.get(id)?.tokens ?? 0))}`);
  console.log(`  → ${b.top1 === a.top1 && b.refusal === a.refusal ? "在本次 100 题上，去掉中文描述**没有改变任何一项指标**" : "指标有变化，见上表"}`);
}
