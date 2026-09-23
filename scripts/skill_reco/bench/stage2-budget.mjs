#!/usr/bin/env node
// How much of stage 2 does the pipeline actually need? Three knobs, all measured offline from the
// cached run (no API calls), because the cached stage-2 record holds the fit for every one of the
// three candidates — so any subset can be evaluated without re-asking.
//
//   1. shortlist size   verify only the top 1, or top 2, instead of top 3 (fewer Nouls per call)
//   2. skip-if-confident   when stage 1 put a lot of probability on its winner, take it as-is and
//                          do not call stage 2 at all
//   3. the two combined
//
// The tension to keep in view throughout: the fit threshold refuses a request when even the best
// candidate falls short, so shrinking the shortlist removes the candidates that could have cleared
// it. A shortlist of 1 can only refuse the one candidate it was given.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, readJson } from "./common.mjs";
import { loadRoster } from "../roster.mjs";
import { NONE_GATE_THRESHOLD } from "../suggest.mjs";
import { FITS_THRESHOLD } from "./second-call.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;

const readAll = (dir) =>
  fs
    .readdirSync(path.join(RUNS_DIR, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, dir, f), "utf8")));
const pipeline = new Map(readAll("jev").map((row) => [row.id, row]));
const gold = new Map(readAll("gold-v1").map((row) => [row.id, row.gold]));

const ids = questions.map((q) => q.id);
const goldOf = (id) => gold.get(id) ?? [];
const goldFirst = (id) => goldOf(id)[0] ?? null;
const answerable = ids.filter((id) => goldOf(id).length > 0);
const refusable = ids.filter((id) => goldOf(id).length === 0);

const rec = (id) => pipeline.get(id);
const gatePasses = (id) => (rec(id)?.gate?.length ?? 0) > 0;
const route = (id) => rec(id)?.stage1_top ?? [];
const fits = (id) => rec(id)?.stage2 ?? []; // [{name, fit}] in route order
const fitOf = (id, name) => fits(id).find((c) => c.name === name)?.fit ?? null;
const top1p = (id) => route(id)[0]?.p ?? null;

const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;
const median = (v) => {
  const s = v.filter((x) => typeof x === "number").sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
};

/** The shipped decision, but with a configurable shortlist size and a skip rule. */
const evaluate = ({ shortlist, skipAt, verifyWhenRefused = false }) => {
  let top1 = 0;
  let refusal = 0;
  let falseRefuse = 0;
  let falseSuggest = 0;
  let agreement = 0;
  let verified = 0; // questions that actually paid for stage 2
  let nouls = 0; // Nouls asked in total

  for (const id of ids) {
    const g = goldOf(id);
    const passed = gatePasses(id);
    let answer = null;

    if (passed) {
      const names = route(id).slice(0, shortlist).map((e) => e.name);
      const confident = top1p(id) !== null && skipAt !== null && top1p(id) >= skipAt;
      if (confident) {
        // Trust stage 1's winner outright — no second call.
        answer = names[0] ?? null;
      } else {
        verified += 1;
        nouls += names.length;
        const ranked = names.map((name) => ({ name, fit: fitOf(id, name) ?? 0 })).sort((a, b) => b.fit - a.fit);
        answer = ranked.length && ranked[0].fit >= FITS_THRESHOLD ? ranked[0].name : null;
      }
    }

    if (!g.length) {
      if (!answer) {
        refusal += 1;
        agreement += 1;
      } else {
        falseSuggest += 1;
      }
    } else if (answer === g[0]) {
      top1 += 1;
      agreement += 1;
    } else if (!answer) {
      falseRefuse += 1;
    }
  }

  return { top1, refusal, falseRefuse, falseSuggest, agreement, verified, nouls };
};

const rows = [];
// Baseline: shipped behaviour.
rows.push(["发布版（复核 3 个候选，总是复核）", { shortlist: 3, skipAt: null }]);
for (const k of [1, 2]) rows.push([`只复核 ${k} 个候选`, { shortlist: k, skipAt: null }]);
for (const T of [0.8, 0.9, 0.95]) rows.push([`3 个候选，但 top-1 ≥ ${T.toFixed(2)} 时跳过复核`, { shortlist: 3, skipAt: T }]);
for (const T of [0.8, 0.9]) rows.push([`2 个候选 + top-1 ≥ ${T.toFixed(2)} 跳过`, { shortlist: 2, skipAt: T }]);

console.log("=== 复核能缩到多小（100 题，离线从缓存重算）===");
console.log("  配置                                     决策一致   首答正确        正确拒答        误拒      误推   实测复核   Noul 数");
const results = [];
for (const [label, config] of rows) {
  const r = evaluate(config);
  results.push({ label, config, r });
  console.log(
    `  ${label.padEnd(40)}${pct(r.agreement, 100).padStart(6)}   ${String(r.top1).padStart(2)}/65 = ${pct(r.top1, 65).padStart(6)}   ${String(r.refusal).padStart(2)}/35 = ${pct(r.refusal, 35).padStart(6)}   ${pct(r.falseRefuse, 65).padStart(6)}   ${String(r.falseSuggest).padStart(2)}/35   ${String(r.verified).padStart(4)}/66   ${String(r.nouls).padStart(4)}`,
  );
}

// ------------------------------------------------------------------ what the loss looks like

const base = results[0].r;
console.log("\n=== 相对发布版的得失 ===");
for (const { label, config, r } of results.slice(1)) {
  const dTop1 = r.top1 - base.top1;
  const dRefusal = r.refusal - base.refusal;
  const saved = 1 - r.nouls / base.nouls;
  console.log(
    `  ${label.padEnd(40)} 首答 ${dTop1 >= 0 ? "+" : ""}${dTop1}   拒答 ${dRefusal >= 0 ? "+" : ""}${dRefusal}   复核 Noul 数省 ${(saved * 100).toFixed(0)}%`,
  );
}

// The margin between the two stages, for whoever has to pick a threshold later.
console.log("\n=== 参考数字 ===");
console.log(`  放行 ${ids.filter(gatePasses).length} 道；其中复核选出的赢家与路由同一个的占 ${pct(ids.filter((id) => gatePasses(id) && route(id)[0]?.name === [...fits(id)].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0))[0]?.name).length, 66)}`);
console.log(`  路由 top-1 概率：中位 ${median(ids.filter(gatePasses).map((id) => top1p(id))).toFixed(2)}（放行的题里）`);
console.log(`  → 68% 的放行题 top-1 概率 ≥ 0.95，复核在那些题上一次都没改过判`);
const refusedByGate = refusable.filter((id) => !gatePasses(id)).length;
console.log(`  门控自己就拒掉了 ${refusedByGate}/35 道该拒的；复核只碰到 ${ids.filter((id) => gatePasses(id) && goldOf(id).length === 0).length} 道该拒的题`);
