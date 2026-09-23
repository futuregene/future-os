#!/usr/bin/env node
// System 1: Jev (TypeSafe System One), the pipeline from ../suggest.mjs.
//
// Records three things per question:
//   gate    — the top-3 shortlist, empty when the Choice prefers none_of_these
//   answer  — what the SERVING pipeline answers: the top-1 of that shortlist, or nothing. This is
//             the shipped answer, and it comes from the one call the server makes.
//   stage2  — the REMOVED second call's fit Nouls, probed here so the record still supports the
//             comparison in REPORT §2.2.5 (the probe is data, not an answer: it costs tokens that
//             the serving path does not spend, which is why score.mjs prices `answer`, not this).
//
//   FUTURE_API_KEY=... node predict-jev.mjs
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, cacheFor, readJson } from "./common.mjs";
import { loadRoster } from "../roster.mjs";
import { Suggester, NONE_GATE_THRESHOLD, STAGE1_REVISION } from "../suggest.mjs";
import { FITS_THRESHOLD, STAGE2_REVISION, verifySecondCall } from "./second-call.mjs";

/** Bump when the set of recorded fields changes, so a re-run re-records them. */
const RECORD_REVISION = 6;

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;

const suggester = new Suggester(roster, { apiKey: process.env.FUTURE_API_KEY, model: process.env.FUTURE_MODEL });
const status = await suggester.checkAuth();
if (suggester.mode !== "typesafe") throw new Error(`needs a working key: ${status.note}`);
console.log(`jev: ${status.note} · none-gate ${NONE_GATE_THRESHOLD} · fits ${FITS_THRESHOLD}`);

const cache = cacheFor(process.env.JEV_RUN ?? "jev");
let done = 0;

/**
 * Resuming is only safe when the answers were produced by the same configuration. A cache hit is
 * not proof of that: changing the stage-2 wording, its payload or either threshold silently
 * leaves 100 stale files behind, and the next run then scores the old answers as if they were
 * new (this happened — `fits_threshold: 0.5` was found inside records produced after the constant
 * had moved to 0.75). Each record now carries the configuration that produced it.
 *
 * RECORD_REVISION covers the bookkeeping fields the cache stores (the winner, the recorded
 * reason);
 * without it, adding a field leaves the cache populated with records that lack it, since the
 * prompt itself did not change.
 */
const CONFIG = {
  noneGate: NONE_GATE_THRESHOLD,
  fits: FITS_THRESHOLD,
  stage1: STAGE1_REVISION,
  stage2: STAGE2_REVISION,
  record: RECORD_REVISION,
};
const isCurrent = (id) => {
  const existing = cache.get(id);
  return Boolean(existing) && JSON.stringify(existing.config) === JSON.stringify(CONFIG);
};

for (const question of questions) {
  if (isCurrent(question.id)) {
    done += 1;
    continue;
  }
  const rank = await suggester.rank(question.text);
  // The gate is the model's own none vote: refuse when it prefers none_of_these.
  const gateAnswer = rank.gate.noneProbability < NONE_GATE_THRESHOLD ? rank.top.map((entry) => entry.name) : [];
  // The serving answer: the shortlist's top-1. The shortlist is already in the Choice's probability
  // order, so nothing else is needed — this is what `server.mjs` returns.
  const answer = gateAnswer.slice(0, 1);

  // The second call is gone from the serving path (REPORT §2.2.5) but is still probed here: the
  // recorded comparison, and the evidence that the fit threshold never fires, both need its
  // numbers. Probed on refused questions too, so the gate can be evaluated offline.
  const probe = rank.top.length ? await verifySecondCall(suggester, question.text, rank.top) : null;

  cache.put(question.id, {
    id: question.id,
    config: CONFIG,
    gate: gateAnswer,
    answer,
    none_probability: rank.gate.noneProbability,
    declined_chunks: rank.gate.declinedChunks,
    survivors: rank.gate.survivorCount,
    stage1b_tokens: rank.stage1b_tokens,
    stage1_top: rank.top.map((entry) => ({ name: entry.name, p: entry.p })),
    stage2: probe?.candidates ?? null,
    stage2_winner: probe?.winner ?? null,
    stage2_reason: probe?.reason ?? null,
    stage1_ms: rank.ms,
    stage2_ms: probe?.ms ?? null,
    stage1_tokens: rank.usage?.input_tokens ?? null,
    stage2_tokens: probe?.usage?.input_tokens ?? null,
    stage2_probe_threshold: 0.3,
    gate_threshold: NONE_GATE_THRESHOLD,
    fits_threshold: FITS_THRESHOLD,
    stage2_revision: STAGE2_REVISION,
  });
  done += 1;
  process.stdout.write(`\rjev ${done}/${questions.length}   `);
}

const rows = cache.all().filter((row) => row.config && JSON.stringify(row.config) === JSON.stringify(CONFIG)).sort((a, b) => a.id.localeCompare(b.id));
console.log(`\n${rows.length}/${questions.length} answers at the current configuration ${JSON.stringify(CONFIG)}`);
const gateRefused = rows.filter((r) => r.gate.length === 0).length;
console.log(`${gateRefused} refused by the gate (the only decision point) · ${100 - gateRefused} recommended`);
console.log(`the removed second call was probed on ${rows.filter((r) => r.stage2).length} questions (data only, not answers)`);
console.log(`threshold: none-gate ${NONE_GATE_THRESHOLD} · the stage-2 fit threshold ${FITS_THRESHOLD} is benchmark-only`);
const served = rows.map((r) => r.stage1_tokens ?? 0);
const probed = rows.map((r) => (r.stage1_tokens ?? 0) + (r.stage2_tokens ?? 0));
const median = (v) => v.sort((a, b) => a - b)[Math.floor(v.length / 2)];
console.log(`tokens/question: served ${median(served)} · probed ${median(probed)} (the difference is benchmark spend)`);
