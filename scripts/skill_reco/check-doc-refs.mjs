// Assert that every script the documentation names actually exists.
//
// The docs describe how to reproduce the evaluation, so a renamed or deleted script leaves them
// pointing at nothing — and the reader finds out only when the command fails. This is the cheapest
// possible guard against that; it deliberately fails loudly when it cannot even read the docs, since
// "measured nothing" must never print as success (the same trap as the batch-D artefact in §2.2.6).
//
//   node check-doc-refs.mjs
import path from "node:path";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url)); // scripts/skill_reco
const docsDir = path.join(here, "..", "..", "docs", "internals", "skill_reco");

const scripts = new Set(
  [...readdirSync(here), ...readdirSync(path.join(here, "bench"))]
    .filter((f) => f.endsWith(".mjs")),
);

// A script name in the docs, e.g. `bench/stage1-prompt.mjs` or `localRank.mjs`.
// Mixed case matters: a lowercase-only class matches "ank.mjs" inside "localRank.mjs".
const SCRIPT = /(?:bench\/)?([A-Za-z0-9_-]+\.mjs)/g;

const docs = ["evaluation.md", "evaluation.zh-CN.md", "harness.md", "harness.zh-CN.md"]
  .concat(["integration-plan.md", "integration-plan.zh-CN.md"]);

let read = 0;
const missing = new Map();
for (const name of docs) {
  let text;
  try {
    text = readFileSync(path.join(docsDir, name), "utf8");
  } catch {
    console.error(`cannot read ${path.join(docsDir, name)} — the check cannot run`);
    process.exit(2);
  }
  read += 1;
  for (const match of text.matchAll(SCRIPT)) {
    const script = match[1];
    if (scripts.has(script)) continue;
    if (!missing.has(script)) missing.set(script, new Set());
    missing.get(script).add(name);
  }
}

if (read !== docs.length) {
  console.error(`read ${read} of ${docs.length} documents — refusing to report a pass`);
  process.exit(2);
}
if (missing.size === 0) {
  console.log(`OK — every script named by the ${read} documents exists`);
  process.exit(0);
}
console.log("DANGLING references (the docs name a script that does not exist):");
for (const [script, where] of [...missing].sort()) {
  console.log(`  ${script}  ← ${[...where].join(", ")}`);
}
process.exit(1);
