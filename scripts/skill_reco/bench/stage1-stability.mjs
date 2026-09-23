#!/usr/bin/env node
// How stable is stage 1 between two identical runs?
//
// This exists because the stage-1-only score moved from 93.8% to 90.8% between two runs of the same
// code on the same questions, which is larger than the difference the whole stage-2 decision was
// being judged on. There are three independent recordings of the SAME stage-1 request in runs/:
//
//   stage1-onechunk   recorded while choosing the routing shape
//   stage1-confidence recorded while testing whether Jev's own confidence predicts a needed verify
//   jev               the pipeline's own record (re-recorded whenever a revision changes)
//
// All three send the identical Choice (141 skills + none_of_these, same wording, same criteria), so
// comparing them measures nothing but run-to-run variance. Reported two ways: how often the winner
// is the same, and — the number that matters for scoring — how often the *decision* (winner, or a
// refusal) is the same.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, readJson } from "./common.mjs";
import { NONE_GATE_THRESHOLD } from "../suggest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const questions = readJson(QUESTIONS_FILE).questions;
const gold = new Map(
  fs
    .readdirSync(path.join(RUNS_DIR, "gold-v1"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, "gold-v1", f), "utf8")))
    .map((row) => [row.id, row.gold]),
);
const readAll = (dir) =>
  fs
    .readdirSync(path.join(RUNS_DIR, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, dir, f), "utf8")));

const ids = questions.map((q) => q.id);
const goldOf = (id) => gold.get(id) ?? [];
const answerable = ids.filter((id) => goldOf(id).length > 0);

// Normalise each recording to { winner, none, decision }.
const runs = {
  "onechunk（早先）": new Map(
    readAll("stage1-onechunk").map((row) => [
      row.id,
      { winner: row.rankedTop?.[0]?.name ?? null, none: row.noneProbability ?? null },
    ]),
  ),
  "confidence（另一次）": new Map(
    readAll("stage1-confidence").map((row) => [
      row.id,
      { winner: row.ranked?.[0]?.name ?? null, none: row.none ?? null },
    ]),
  ),
  "jev（本次发布）": new Map(
    readAll("jev").map((row) => [
      row.id,
      { winner: row.stage1_top?.[0]?.name ?? null, none: row.none_probability ?? null },
    ]),
  ),
};

const decision = (row) => (row.none >= NONE_GATE_THRESHOLD ? null : row.winner);
const label = (x) => x ?? "（拒答）";
const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;

const names = Object.keys(runs);
console.log("=== 同一批题、同一个请求、三次独立运行的一致性 ===");
console.log(`  （stage-1 判定 = none 概率 < ${NONE_GATE_THRESHOLD} 则取第一名，否则拒答）\n`);
console.log("  对比                              首答相同        判定相同        首答正确(仅 stage 1)");
for (let i = 0; i < names.length; i += 1) {
  for (let j = i + 1; j < names.length; j += 1) {
    const A = runs[names[i]];
    const B = runs[names[j]];
    const shared = ids.filter((id) => A.has(id) && B.has(id));
    const sameWinner = shared.filter((id) => A.get(id).winner === B.get(id).winner).length;
    const sameDecision = shared.filter((id) => decision(A.get(id)) === decision(B.get(id))).length;
    const ans = shared.filter((id) => goldOf(id).length > 0);
    const accA = ans.filter((id) => decision(A.get(id)) === goldOf(id)[0]).length;
    const accB = ans.filter((id) => decision(B.get(id)) === goldOf(id)[0]).length;
    console.log(
      `  ${`${names[i]} vs ${names[j]}`.padEnd(34)}${String(sameWinner).padStart(3)}/${shared.length} ${pct(sameWinner, shared.length).padStart(6)}   ${String(sameDecision).padStart(3)}/${shared.length} ${pct(sameDecision, shared.length).padStart(6)}   ${pct(accA, ans.length)} vs ${pct(accB, ans.length)}`,
    );
  }
}

// Where the disagreements are, and whether they are near-ties (which would mean the model is not
// wrong, it is undecided).
const A = runs["onechunk（早先）"];
const C = runs["jev（本次发布）"];
const diffs = ids.filter((id) => A.has(id) && C.has(id) && decision(A.get(id)) !== decision(C.get(id)));
console.log(`\n=== 三次运行里判定不同的题（早先 vs 本次，共 ${diffs.length} 道）===`);
console.log("  题号   金标题               早先              本次              none(早先→本次)   是否近并列");
for (const id of diffs) {
  const a = A.get(id);
  const c = C.get(id);
  const g = goldOf(id)[0] ?? "（拒答）";
  const aOk = decision(a) === goldOf(id)[0];
  const cOk = decision(c) === goldOf(id)[0];
  console.log(
    `  ${id}  ${String(g).padEnd(20)}${label(decision(a)).padEnd(18)}${label(decision(c)).padEnd(18)}` +
      `${a.none?.toFixed(2)} → ${c.none?.toFixed(2)}        ${aOk === cOk ? "（两者得分相同）" : aOk ? "早先对" : "本次对"}`,
  );
}

// The distribution of top-1 probability, to show how much room there is for a flip.
const probs = [...C.values()].map((r) => (r.none >= NONE_GATE_THRESHOLD ? null : r.none)).filter((x) => x !== null);
console.log(`\n=== 放行题的 none 概率分布（本次）===`);
const sorted = probs.sort((x, y) => x - y);
console.log(`  最小 ${sorted[0]?.toFixed(2)}  中位 ${sorted[Math.floor(sorted.length / 2)]?.toFixed(2)}  最大 ${sorted[sorted.length - 1]?.toFixed(2)}`);
console.log(`  落在阈值附近的题（none 在 0.10–0.25）：${sorted.filter((x) => x >= 0.1 && x <= 0.25).length} 道 —— 这些题的放行/拒答会在两次运行之间翻动`);
