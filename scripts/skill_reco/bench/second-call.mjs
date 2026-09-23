// The REMOVED second call, kept only so the evaluation stays reproducible.
//
// This is not part of the serving path. `../suggest.mjs` answers from a single Choice, and
// `server.mjs` never imports anything from here. This file exists because the recorded comparison in
// `dataset/score.json` (and REPORT §2.2.5) was produced by asking a second question — the same one
// with the full description and the SKILL.md opening for each candidate, one Noul per candidate — and
// deleting the code would delete the ability to re-derive or revisit that decision.
//
// It was removed because, measured on the same recorded stage-1 answers, it decided 3 of 65 answers
// while costing every request a second network round trip (p50 +284 ms, p90 +186 ms) and 10.1% more
// tokens. Its own fit threshold never fired in 100 questions, so refusal never depended on it. See
// `bench/drop-stage2.mjs` for the three-way comparison and REPORT §2.2.5 for the reasoning.
//
// Three things in the payload were measured rather than assumed (bench/payload-ablation.mjs,
// bench/excerpt-shape.mjs, bench/stage2-zh.mjs): `tagline` was a byte-prefix of `description` on all
// 141 skills and `chinese_description` changed nothing (a paired A/B moved the median fit by 0.000
// and the end-to-end rerun changed 0 of 100 answers), so both are gone; and the SKILL.md opening is
// the only genuinely new information, since dropping it collapses the zero-false-refusal threshold
// from 0.79 to 0.29. Putting the candidates in `state` once and referencing them by name is worse
// still (refusal 97% -> 89%), so each Noul carries its candidate inline.
import { localVerify } from "../localRank.mjs";

/**
 * Fit-Noul threshold for this (removed) call. Chosen from bench/payload-ablation.mjs when it was in
 * the serving path: with the shipped payload the answerable questions bottom out at 0.79 while the
 * gate-passing refusals it caught sit at or below 0.74, so 0.75 refused four of them with no false
 * refusal. Kept in the cache fingerprint so a change here re-runs the benchmark instead of silently
 * reusing records produced under the old value.
 */
export const FITS_THRESHOLD = Number(process.env.FITS_THRESHOLD ?? 0.75);

/** Request-shape revision for this call; part of the benchmark cache fingerprint. */
export const STAGE2_REVISION = "stage2-fit-only-v7";

const number = (answer, key) => (typeof answer?.[key] === "number" ? answer[key] : null);
const round = (value) => (typeof value === "number" ? Number(value.toFixed(4)) : null);

/** One Noul per candidate: the same question stage 1 asks, with the candidate's fuller evidence. */
function stageTwoQuestions(roster, candidates) {
  const questions = {};
  candidates.forEach((candidate, i) => {
    const skill = roster.byName.get(candidate.name);
    questions[`fit_${i}`] = {
      type: "noul",
      instructions: {
        question: "Would the skill in `skill` materially help with the request in `request`?",
        skill: {
          name: candidate.name,
          description: skill?.description || "",
          instructions_excerpt: skill?.excerpt || "",
        },
      },
      criteria: {
        true: "The skill's own description or instructions cover what the request needs, directly or as a clearly required first step",
        false: "Only topical overlap, a neighbouring task, or general assistance would be enough",
      },
    };
  });
  return questions;
}

/**
 * Ask the second question about `candidates` and return the ranked fits.
 *
 * Throws on a Jev error rather than demoting to the local scorer: this exists only for the
 * benchmark, which requires a working key anyway (`predict-jev.mjs` refuses to run without one), so a
 * silent fallback here would quietly record local-BM25 numbers as Jev's.
 */
export async function verifySecondCall(suggester, query, candidates) {
  if (suggester.mode !== "typesafe") {
    const scored = localVerify(suggester.deepIndex, query, candidates);
    const winner = scored[0];
    const passes = winner && (winner.fit ?? 0) >= FITS_THRESHOLD;
    return {
      backend: "local",
      candidates: scored,
      winner: passes ? winner.name : null,
      best_fit: winner?.fit ?? null,
      decision: passes ? "suggest" : "nothing",
      reason: `本地启发式（BM25，非 Jev）：top-1 拟合度 ${winner?.fit ?? "—"} ${passes ? "≥" : "<"} ${FITS_THRESHOLD}`,
    };
  }

  const { answers, model, usage, ms, attempts, request } = await suggester.client.systemOne({
    state: { request: query },
    questions: stageTwoQuestions(suggester.roster, candidates),
  });

  const fits = candidates.map((candidate, i) => ({
    name: candidate.name,
    fit: round(number(answers[`fit_${i}`], "noul")),
    fit_confidence: round(number(answers[`fit_${i}`], "confidence")),
    builtin: candidate.builtin ?? null,
    category: candidate.category ?? null,
    description: candidate.description ?? "",
  }));

  const byFit = [...fits].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0));
  const winner = byFit[0];
  const passes = Boolean(winner) && (winner.fit ?? 0) >= FITS_THRESHOLD;

  return {
    backend: "typesafe",
    model: model || suggester.client.model,
    ms,
    usage,
    attempts,
    request,
    candidates: byFit,
    winner: passes ? winner.name : null,
    best_candidate: winner?.name ?? null,
    best_fit: winner?.fit ?? null,
    decision: passes ? "suggest" : "nothing",
    reason: passes
      ? `fit noul ${winner.fit} ≥ ${FITS_THRESHOLD}`
      : `fit noul ${winner?.fit ?? "—"} < ${FITS_THRESHOLD}，判定为没有合适技能`,
  };
}
