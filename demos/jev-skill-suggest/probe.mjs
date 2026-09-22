#!/usr/bin/env node
// Smoke-test the Jev API and the recommender without the browser.
//
//   TYPESAFE_API_KEY=jev_... node probe.mjs                 # key check + one query
//   TYPESAFE_API_KEY=... node probe.mjs "把报告做成PPT"      # custom query
import path from "node:path";
import { fileURLToPath } from "node:url";
import { loadRoster } from "./roster.mjs";
import { Suggester, NONE_GATE_THRESHOLD } from "./suggest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const skillsRoot = process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "skills");
const query = process.argv[2] ?? "把这份季度报告做成一版路演用的 PPT，要能直接导出 PDF";

const roster = loadRoster(skillsRoot);
const suggester = new Suggester(roster, {
  apiKey: process.env.TYPESAFE_API_KEY,
  baseUrl: process.env.TYPESAFE_BASE_URL,
  model: process.env.TYPESAFE_MODEL,
  insecureLocalOnly: process.env.LOCAL_ONLY === "1",
});

const key = process.env.TYPESAFE_API_KEY;
// Masked on purpose: enough to tell two keys apart in a shell history, not enough to reuse.
console.log(`key: ${key ? `${key.slice(0, 4)}… (len ${key.length})` : "(missing)"}`);
const status = await suggester.checkAuth();
console.log(`backend: ${status.mode}\nnote: ${status.note}`);
console.log(`roster: ${roster.skills.length} skills (builtin ${roster.builtinCount} + third-party ${roster.thirdPartyCount})\n`);

if (status.mode === "typesafe") {
  const { data } = await suggester.client.listModels();
  console.log("GET /v1/models →", JSON.stringify(data).slice(0, 400), "\n");
}

console.log(`query: ${query}\n`);
const rank = await suggester.rank(query);
console.log(`stage 1 (${rank.backend}, ${rank.ms} ms, attempts ${rank.attempts})`);
console.log("  gate:", JSON.stringify(rank.gate));
console.log(`  chunks: ${rank.gate.chunkCount} (${rank.gate.declinedChunks} declined), survivors ${rank.gate.survivorCount}`);
console.log(`  top ${rank.top.length} of ${rank.choices_seen} ranked:`);
for (const entry of rank.top) console.log(`    ${entry.name.padEnd(28)} p=${entry.p}`);
console.log("  usage:", JSON.stringify(rank.usage));

// One call, one threshold — the same decision the server makes. There is no stage 2 to mirror any
// more; if you want to see what the removed second call would have answered, the benchmark does
// that (bench/predict-jev.mjs) and REPORT §2.2.5 records why it is not here.
const gatePasses = (rank.gate.noneProbability ?? 0) < NONE_GATE_THRESHOLD;
console.log(
  gatePasses
    ? `\n决定：推荐 ${rank.top[0]?.name}（none ${rank.gate.noneProbability} < ${NONE_GATE_THRESHOLD}）`
    : `\n决定：没有合适技能（none ${rank.gate.noneProbability} ≥ ${NONE_GATE_THRESHOLD}）`,
);
