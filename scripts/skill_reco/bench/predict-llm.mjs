#!/usr/bin/env node
// System 3: a general-purpose LLM doing the same routing decision — deepseek-flash, at its
// own default thinking level, given the identical roster text and system prompt as the gold
// pass. This is the "just ask a chat model" baseline the decision model is competing with.
//
//   node predict-llm.mjs
import { MODELS, QUESTIONS_FILE, ROSTER_FILE, cacheFor, extractJson, extractSkillNames, llm, normalizeSkills, pool, readJson, rosterIndexBlock } from "./common.mjs";

const questions = readJson(QUESTIONS_FILE).questions;
const roster = readJson(ROSTER_FILE).skills;
const KNOWN = new Set(roster.map((skill) => skill.name));

// Identical to ground-truth.mjs, so the only difference is the model.
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

const SCHEMA = [
  "Reply with ONLY this JSON object, no other text:",
  '{"needs_skill": <true|false>, "skills": ["<catalog name>", ...], "reason": "<one short line>"}',
  "Use an empty skills list when no catalog skill applies.",
].join("\n");

const PROMPT = (question) => ["# Skill catalog", rosterIndexBlock(roster), "", "# User request", question.text, "", SCHEMA].join("\n");

const cache = cacheFor("llm");
const pending = questions.filter((q) => !cache.has(q.id));
console.log(`llm (${MODELS.llm}): ${pending.length} to do, ${questions.length - pending.length} cached`);

const results = await pool(
  pending,
  async (question) => {
    const { text, durationMs, usage } = await llm({ model: MODELS.llm, prompt: PROMPT(question), systemPrompt: SYSTEM });
    const parsed = extractJson(text);
    const names = parsed ? parsed.skills : extractSkillNames(text, KNOWN);
    const skills = normalizeSkills(names, KNOWN);
    const record = {
      id: question.id,
      answer: skills,
      needs_skill: parsed?.needs_skill ?? skills.length > 0,
      reason: parsed?.reason ?? "",
      parse_mode: parsed ? "json" : "prose",
      durationMs,
      usage,
    };
    cache.put(question.id, record);
    return record;
  },
  { concurrency: 6, label: "llm" },
);

const failures = results.filter((r) => r.result?.error);
if (failures.length) {
  console.log(`\n${failures.length} FAILED:`);
  for (const failure of failures.slice(0, 8)) console.log(`  ${failure.item.id}: ${failure.result.error}`);
}

const rows = cache.all().sort((a, b) => a.id.localeCompare(b.id));
console.log(`\n${rows.length} answers: ${rows.filter((r) => r.answer.length === 0).length} say none, ${rows.filter((r) => r.parse_mode === "prose").length} parsed from prose`);
