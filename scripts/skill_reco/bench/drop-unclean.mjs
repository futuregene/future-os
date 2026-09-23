// Drops the cached questions that failed validation, so the build re-generates only those.
import { readdirSync, readFileSync, unlinkSync } from "node:fs";
import path from "node:path";
import { RUNS_DIR } from "./common.mjs";

const dir = path.join(RUNS_DIR, "questions");
const bad = readdirSync(dir)
  .filter((file) => file.endsWith(".json"))
  .filter((file) => !JSON.parse(readFileSync(path.join(dir, file), "utf8")).clean);
for (const file of bad) unlinkSync(path.join(dir, file));
console.log(`${bad.length} cached questions dropped: ${bad.map((f) => f.replace(/\.json$/, "")).join(", ")}`);
