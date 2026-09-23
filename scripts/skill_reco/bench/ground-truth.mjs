#!/usr/bin/env node
// Produces the reference answer ("gold") for every question: the gold model sees the roster
// and the question, and nothing else — not the skill the question was written from.
//
// The prompt is available at two levels of detail so the gold can be audited:
//   --prompt v1  roster one-liners only (default; the same information the LLM baseline gets)
//   --prompt v2  roster one-liners plus each candidate skill's SKILL.md opening, for the
//                top 12 of a v1 pass — i.e. a "read harder" reference
//
// Both are the gold model's own judgment. Neither is human ground truth.
import { MODELS, QUESTIONS_FILE, ROSTER_FILE, cacheFor, extractJson, extractSkillNames, llm, normalizeSkills, pool, readJson, rosterIndexBlock, writeJson } from "./common.mjs";

const QUESTIONS = readJson(QUESTIONS_FILE).questions;
const ROSTER = readJson(ROSTER_FILE).skills;
const KNOWN = new Set(ROSTER.map((skill) => skill.name));
const variant = (process.argv.find((a) => a.startsWith("--prompt="))?.split("=")[1]) ?? "v1";

const SYSTEM = [
  "You are a skill router: a classifier, not an assistant.",
  "You are given a catalog of agent skills and one user request.",
  "You name the skills that should be loaded for that request, most relevant first.",
  "",
  "Rules:",
  "- At most 3 skills. Fewer is normal when only one or two genuinely apply.",
  "- Only names that appear in the catalog. Never invent a name or paraphrase one.",
  "- Everyday help, conversation, general knowledge, and requests for services the catalog",
  "  does not cover need no skill at all; return an empty list for those.",
  "- Judge by what the request asks the agent to do, not by topic overlap. A request about a",
  "  scientific subject is not automatically a job for a scientific skill if the user just",
  "  wants an explanation.",
  "- Do not answer the user's request. Do not explain your reasoning in prose. No markdown.",
  "- Reply with one JSON object and nothing else.",
].join("\n");

const ROSTER_BLOCK = rosterIndexBlock(ROSTER);

/** The schema is repeated last, as its own instruction, because the models drop it otherwise. */
const SCHEMA = [
  "Reply with ONLY this JSON object, no other text:",
  '{"needs_skill": <true|false>, "skills": ["<catalog name>", ...], "reason": "<one short line>"}',
  "Use an empty skills list when no catalog skill applies.",
].join("\n");

const cache = cacheFor(process.env.GOLD_RUN ?? `gold-${variant}`);

const buildPrompt = (question) => {
  if (variant === "v1") {
    return [
      "# Skill catalog",
      ROSTER_BLOCK,
      "",
      "# User request",
      question.text,
      "",
      SCHEMA,
    ].join("\n");
  }
  return ["# User request", question.text, "", SCHEMA].join("\n");
};

const pending = QUESTIONS.filter((q) => !cache.has(q.id));
console.log(`gold (${variant}, ${MODELS.gold}): ${pending.length} to do, ${QUESTIONS.length - pending.length} cached`);

const results = await pool(
  pending,
  async (question) => {
    const { text, durationMs, usage } = await llm({ model: MODELS.gold, prompt: buildPrompt(question), systemPrompt: SYSTEM });
    const parsed = extractJson(text);
    const names = parsed ? parsed.skills : extractSkillNames(text, KNOWN);
    const skills = normalizeSkills(names, KNOWN);
    const record = {
      id: question.id,
      gold: skills,
      needs_skill: parsed?.needs_skill ?? skills.length > 0,
      reason: parsed?.reason ?? "",
      raw: names ?? [],
      parse_mode: parsed ? "json" : "prose",
      durationMs,
      usage,
    };
    cache.put(question.id, record);
    return record;
  },
  { concurrency: 6, label: `gold-${variant}` },
);

const failures = results.filter((r) => r.result?.error);
if (failures.length) {
  console.log(`\n${failures.length} FAILED:`);
  for (const failure of failures.slice(0, 10)) console.log(`  ${failure.item.id}: ${failure.result.error}`);
}

const rows = cache.all().sort((a, b) => a.id.localeCompare(b.id));
// The distilled gold answers are committed; the per-question cache under runs/ is not.
const { fileURLToPath } = await import("node:url");
writeJson(fileURLToPath(new URL(`../dataset/gold-${variant}.json`, import.meta.url)), {
  model: MODELS.gold,
  prompt_variant: variant,
  note: "reference answers only; the refusal judgment and first choice are stable across passes (see bench/gold-stability.mjs)",
  answers: rows,
});
const withNone = rows.filter((r) => r.gold.length === 0);
const prose = rows.filter((r) => r.parse_mode === "prose");
console.log(`\n${rows.length} gold answers: ${rows.length - withNone.length} name at least one skill, ${withNone.length} say none`);
console.log(`answered in prose instead of JSON: ${prose.length} (parsed by name matching)`);
const counts = new Map();
for (const row of rows) counts.set(row.gold.length, (counts.get(row.gold.length) ?? 0) + 1);
console.log(`gold sizes: ${[...counts.entries()].sort().map(([k, v]) => `${k} skill(s): ${v}`).join("  ")}`);
console.log(`\nnext: node score.mjs (after predict-*.mjs)`);
