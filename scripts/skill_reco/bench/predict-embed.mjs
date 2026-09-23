#!/usr/bin/env node
// System 2: embedding retrieval on the omlx server (Qwen3-Embedding-0.6B-4bit-DWQ).
//
// The document for each skill is what the roster shows the other systems — name plus one-line
// description — so this is the "embeddings instead of a decision model" baseline. It has no
// built-in way to say "no skill applies": cosine always yields a nearest neighbour, so the
// refusal has to come from a score threshold, tuned on one half of the questions and reported
// on the other (see score.mjs).
//
//   node predict-embed.mjs
import path from "node:path";
import { fileURLToPath } from "node:url";
import { MODELS, QUESTIONS_FILE, ROSTER_FILE, cacheFor, cosine, embed, readJson, writeJson } from "./common.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const questions = readJson(QUESTIONS_FILE).questions;
const roster = readJson(ROSTER_FILE).skills;
console.log(`embedding ${roster.length} skills and ${questions.length} questions with ${MODELS.embed}`);

/** Skill vectors are cached on the roster text, so editing a description rebuilds them. */
const VECTORS_FILE = path.join(here, "..", "runs", "embed-skill-vectors.json");
const document = (skill) => `${skill.name}: ${skill.description}`;
const fingerprint = JSON.stringify(roster.map(document));

let skillVectors = readJson(VECTORS_FILE);
if (!skillVectors || skillVectors.fingerprint !== fingerprint) {
  const { vectors } = await embed(roster.map(document));
  skillVectors = { fingerprint, model: MODELS.embed, vectors };
  writeJson(VECTORS_FILE, skillVectors);
  console.log(`embedded roster (${roster.length} vectors of ${vectors[0].length} dims)`);
} else {
  console.log("reusing cached roster vectors");
}

const cache = cacheFor("embed");
let done = 0;

for (const question of questions) {
  if (cache.has(question.id)) {
    done += 1;
    continue;
  }
  const startedAt = Date.now();
  const { vectors: [vector], tokens } = await embed([question.text]);
  const embedMs = Date.now() - startedAt;
  const scored = roster
    .map((skill, i) => ({ name: skill.name, score: cosine(vector, skillVectors.vectors[i]) }))
    .sort((a, b) => b.score - a.score);
  cache.put(question.id, {
    id: question.id,
    ranked: scored.slice(0, 5),
    margin: Number((scored[0].score - scored[1].score).toFixed(4)),
    ms: embedMs,
    // The embedding server reports the prompt token count for this question.
    tokens,
  });
  done += 1;
  process.stdout.write(`\rembed ${done}/${questions.length}   `);
}

const rows = cache.all().sort((a, b) => a.id.localeCompare(b.id));
const scores = rows.map((r) => r.ranked[0].score).sort((a, b) => a - b);
console.log(`\n${rows.length} answers`);
console.log(`top-1 cosine: min ${scores[0].toFixed(3)}  median ${scores[Math.floor(scores.length / 2)].toFixed(3)}  max ${scores.at(-1).toFixed(3)}`);
