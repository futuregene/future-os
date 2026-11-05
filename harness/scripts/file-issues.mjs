/**
 * File the drafts that `issues-en` already wrote, without re-running the model.
 *
 * Reading the manifest is also the safe path: the files and the titles it names
 * are the ones a human can review before this runs.
 */
import { readFileSync } from "node:fs";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const runDir = process.argv[2] ?? "harness/runs/final-v12";
const repo = process.argv[3] ?? "futuregene/future-os";
const manifest = JSON.parse(readFileSync(`${runDir}/issues-en/drafts.json`, "utf8"));

const { stdout: existingRaw } = await execFileAsync("gh", ["issue", "list", "--repo", repo, "--state", "all", "--limit", "200", "--json", "title,url"]);
const existing = new Map(JSON.parse(existingRaw || "[]").map(item => [item.title, item.url]));

let filed = 0;
for (const draft of manifest.drafts) {
  if (existing.has(draft.title)) {
    console.log(`skip (exists): ${existing.get(draft.title)}`);
    continue;
  }
  try {
    const { stdout } = await execFileAsync("gh", ["issue", "create", "--repo", repo, "--title", draft.title, "--body-file", draft.file]);
    filed += 1;
    console.log(`filed: ${stdout.trim()}`);
  }
  catch (error) {
    console.log(`FAILED: ${draft.title} — ${String(error?.stderr ?? error?.message ?? error).slice(0, 200)}`);
  }
}
console.log(`\n${filed} filed, ${manifest.drafts.length} drafted.`);
