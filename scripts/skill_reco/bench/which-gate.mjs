// Which operating-point FAMILY does the CV actually pick for the chunked design — the none
// probability alone, or none plus the separate needs_skill gate? The aggregate table does not say,
// and the implementation should ship whichever the folds chose.
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";

const questions = readJson(QUESTIONS_FILE).questions;
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));
const ids = questions.map((q) => q.id);
const goldOf = (id) => gold.get(id).gold;
const FOLDS = 5;
const foldOf = new Map(ids.map((id, i) => [id, i % FOLDS]));

function score(idsIn, answerOf) {
  let firstCorrect = 0;
  let falseSuggest = 0;
  let positives = 0;
  for (const id of idsIn) {
    const reference = goldOf(id);
    const answer = answerOf(id);
    if (reference.length === 0) {
      if (answer !== null) falseSuggest += 1;
    } else {
      positives += 1;
      if (answer !== null && answer === reference[0]) firstCorrect += 1;
    }
  }
  return { firstCorrect, falseSuggest, positives };
}

for (const size of [18, 30, 47]) {
  const rows = new Map(cacheFor(`stage1-chunked-raw-${size}`).all().map((row) => [row.id, row]));
  const answerOf = (id, p) => {
    const row = rows.get(id);
    if (!row) return null;
    if (row.finalNone === null || row.finalNone >= p.t) return null;
    if (p.useGate && row.needsSkill !== null && row.needsSkill < p.g) return null;
    return row.stage2Answer;
  };
  const grid = [];
  for (let t = 0.05; t <= 0.95; t += 0.02) {
    grid.push({ t: Number(t.toFixed(2)), useGate: false, family: "仅 none" });
    for (let g = 0.5; g <= 0.95; g += 0.03) grid.push({ t: Number(t.toFixed(2)), g: Number(g.toFixed(2)), useGate: true, family: "none + needs_skill" });
  }

  const picks = [];
  for (let budget = 0; budget <= 1; budget += 1) {
    const perFold = [];
    for (let fold = 0; fold < FOLDS; fold += 1) {
      const train = ids.filter((id) => foldOf.get(id) !== fold);
      let best = null;
      for (const params of grid) {
        const s = score(train, (id) => answerOf(id, params));
        if (s.falseSuggest > budget) continue;
        if (!best || s.firstCorrect > best.s.firstCorrect) best = { params, s };
      }
      perFold.push(best ? `${best.params.family}@t=${best.params.t}${best.params.useGate ? `/g=${best.params.g}` : ""}` : "无解");
    }
    picks.push(`预算${budget}: ${perFold.join("  |  ")}`);
  }
  console.log(`分块 ${size}：`);
  for (const line of picks) console.log(`  ${line}`);
}
