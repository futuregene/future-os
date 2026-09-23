// Drops the cached answers for the covered questions (p*), which changed when the dataset was
// rewritten, while keeping the hand-written negatives (n*), whose text did not change. Without
// this the re-run would silently score the old answers against the new questions.
import { readdirSync, unlinkSync } from "node:fs";
import path from "node:path";
import { RUNS_DIR } from "./common.mjs";

const steps = process.argv.slice(2);
let total = 0;
for (const step of steps) {
  const dir = path.join(RUNS_DIR, step);
  const stale = readdirSync(dir).filter((file) => file.startsWith("p") && file.endsWith(".json"));
  for (const file of stale) unlinkSync(path.join(dir, file));
  total += stale.length;
  console.log(`${step}: dropped ${stale.length} covered-question answers`);
}
console.log(`${total} files dropped`);
