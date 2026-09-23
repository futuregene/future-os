// Independent check that no covered question gives its source skill away.
//
// Two tests: the skill's name (and its spelling variants) must not appear at all, and no term
// that is unique to this skill's documentation may appear either. Run this against the shipped
// dataset rather than trusting the generator's own validation.
import { QUESTIONS_FILE, ROSTER_FILE, readJson } from "./common.mjs";

const questions = readJson(QUESTIONS_FILE).questions.filter((q) => q.source_skill);
const roster = readJson(ROSTER_FILE).skills;
const tokensOf = (text) =>
  String(text ?? "")
    .toLowerCase()
    .split(/[^a-z0-9+#.]+/)
    .filter((token) => token.length >= 4);

const variants = (name) => [name.toLowerCase(), name.toLowerCase().replace(/-/g, " "), name.toLowerCase().replace(/-/g, "")];

const namesHits = [];
const termHits = [];
for (const question of questions) {
  const haystack = question.text.toLowerCase();
  const skill = roster.find((s) => s.name === question.source_skill);
  for (const variant of variants(skill.name)) {
    if (variant.length >= 4 && haystack.includes(variant)) namesHits.push({ id: question.id, skill: skill.name, variant });
  }
  // Unique-to-this-skill vocabulary, computed against every other skill's own SKILL.md text.
  const mine = new Set(tokensOf(`${skill.name} ${skill.description} ${skill.category}`));
  const elsewhere = new Set();
  for (const other of roster) {
    if (other.name === skill.name) continue;
    for (const token of tokensOf(`${other.name} ${other.description} ${other.descriptionZh}`)) elsewhere.add(token);
  }
  for (const term of mine) {
    if (term.length < 5 || elsewhere.has(term)) continue;
    if (new RegExp(`\\b${term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`).test(haystack)) {
      termHits.push({ id: question.id, skill: skill.name, term });
    }
  }
}

console.log(`${questions.length} covered questions checked`);
console.log(`  names the source skill:            ${namesHits.length}`);
console.log(`  uses vocabulary unique to its docs: ${termHits.length}`);
for (const hit of [...namesHits, ...termHits].slice(0, 10)) console.log(`    ${hit.id} [${hit.skill}] ${hit.variant ?? hit.term}`);
console.log(`\nsample of the questions:`);
for (const question of questions.slice(0, 8)) console.log(`  [${question.source_skill}] ${question.text.slice(0, 92)}`);
