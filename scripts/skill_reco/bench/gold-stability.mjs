// How stable is the gold itself? Two independent gold passes over the same questions.
// This bounds how much any score difference between systems can mean.
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { RUNS_DIR } from "./common.mjs";

const load = (name) => {
  const dir = path.join(RUNS_DIR, name);
  const rows = new Map();
  for (const file of readdirSync(dir)) {
    if (file.endsWith(".json")) rows.set(file.replace(/\.json$/, ""), JSON.parse(readFileSync(path.join(dir, file), "utf8")));
  }
  return rows;
};

const [aName, bName] = process.argv.slice(2);
const a = load(aName);
const b = load(bName);
const ids = [...a.keys()].filter((id) => b.has(id));

const sameFirst = ids.filter((id) => (a.get(id).gold[0] ?? null) === (b.get(id).gold[0] ?? null));
const sameSet = ids.filter((id) => JSON.stringify(a.get(id).gold) === JSON.stringify(b.get(id).gold));
const differ = ids.filter((id) => JSON.stringify(a.get(id).gold) !== JSON.stringify(b.get(id).gold));

console.log(`${aName} vs ${bName} (${ids.length} questions)`);
console.log(`  first skill identical: ${sameFirst.length}/${ids.length}  (${((sameFirst.length / ids.length) * 100).toFixed(1)}%)`);
console.log(`  whole answer identical: ${sameSet.length}/${ids.length}  (${((sameSet.length / ids.length) * 100).toFixed(1)}%)`);
console.log(`\n  ${differ.length} disagreements:`);
for (const id of differ) {
  console.log(`    ${id}  A=${JSON.stringify(a.get(id).gold)}  B=${JSON.stringify(b.get(id).gold)}`);
}
