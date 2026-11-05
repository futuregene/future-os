/**
 * The whole English-issue flow for one run: group → localize → merge → write →
 * file.
 *
 * Kept out of `cli.mjs` so it can be driven without the CLI (and so the steps
 * stay readable): each step has one job, and the merge step exists because the
 * first grouping runs on findings that may be written in different languages for
 * the same defect.
 */
import { draftEnglishIssues, localizeDrafts, mergeLocalized, writeEnglishIssues } from "./issues-en.mjs";
import { loadCases } from "./cases.mjs";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { runPrompt as promptWithModel } from "./analyze.mjs";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);


/**
 * A second clustering pass over the English drafts.
 *
 * Localization is per group, so two groups that describe one defect come back
 * with two different English sentences and never merge on wording alone. Asking
 * the model once, with all of them side by side, is the reliable way to find
 * those pairs: it can see that "the check at step 11 failed" and "the raw SQLite
 * text reached the user" are the same defect.
 */
async function mergeByModel(merged, { runPrompt, runDir, model }) {
  if (merged.length < 2)
    return merged;
  const lines = merged.map((entry, index) => `${index + 1}. [${entry.primary.severity}] ${entry.title} — cases: ${entry.caseIds.join(", ")}`);
  const prompt = [
    "These are issue titles written from automated test evidence for one application.",
    "Group the ones that describe the SAME underlying defect. Be strict: same user-visible cause, not merely the same screen or area.",
    "Every index must appear in exactly one group.",
    "",
    ...lines,
    "",
    'Answer with strict JSON: {"groups":[[1,5],[2],[3],...]}',
  ].join("\n");
  const promptPath = join(runDir, "issues-en", "_merge.md");
  writeFileSync(promptPath, prompt);
  try {
    const { text } = await runPrompt(promptPath, { model });
    writeFileSync(join(runDir, "issues-en", "_merge-answer.md"), text);
    const parsed = JSON.parse(text.slice(text.indexOf("{"), text.lastIndexOf("}") + 1));
    const groups = Array.isArray(parsed.groups) ? parsed.groups : [];
    const seen = new Set();
    const result = [];
    for (const group of groups) {
      const members = group.map(index => merged[index - 1]).filter(Boolean).filter((entry) => {
        if (seen.has(entry))
          return false;
        seen.add(entry);
        return true;
      });
      if (!members.length)
        continue;
      const [first, ...rest] = members;
      result.push({
        ...first,
        groups: members.flatMap(entry => entry.groups),
        texts: members.flatMap(entry => entry.texts),
        caseIds: [...new Set(members.flatMap(entry => entry.caseIds))],
        primary: members.map(entry => entry.primary).sort((a, b) => (["high", "medium", "low", "info"].indexOf(a.severity) - ["high", "medium", "low", "info"].indexOf(b.severity)))[0],
      });
    }
    // Anything the model left out keeps its own entry rather than disappearing.
    for (const entry of merged)
      if (!seen.has(entry))
        result.push(entry);
    return result;
  }
  catch {
    return merged;
  }
}

export async function runIssuesEn(runDir, { model, runPrompt, file = false, repo = null, yes = false } = {}) {
  const { analysis, cases, groups, fixtureSkipped } = draftEnglishIssues(runDir, { minSeverity: "medium" });
  if (!groups.length)
    return { message: `Nothing at medium severity or above${fixtureSkipped ? ` (${fixtureSkipped} fixture-authored finding(s) excluded)` : ""}; no issues drafted.`, drafts: [] };

  // A finding for a case that no longer exists cannot be re-run, so it cannot be
  // a useful issue. Dropped cases leave their findings behind in older analyses.
  const known = new Set(loadCases({}).map(item => item.id));
  // A cross-case finding can cite a case that has since been dropped; the issue
  // stays useful as long as one of its cases is still runnable, so prune the dead
  // ids rather than discard the finding.
  for (const group of groups)
    group.caseIds = group.caseIds.filter(id => known.has(id));
  const live = groups.filter(group => group.caseIds.length > 0);
  const stale = groups.filter(group => !live.includes(group));
  if (!live.length)
    return { message: `All ${groups.length} candidate issues belong to cases that no longer exist; nothing drafted.`, drafts: [] };

  const entries = await localizeDrafts(live, { runPrompt: runPrompt ?? promptWithModel, runDir, model });
  // Conservative: word overlap plus the model's own per-group duplicate hint.
  // A global re-clustering pass was tried and over-merged unrelated defects (a
  // batch-delete dialog merged with an approval card), which hides findings.
  const merged = mergeLocalized(entries);
  const drafts = writeEnglishIssues(runDir, merged, { analysis, cases });

  const summary = [
    `${drafts.length} issue(s) drafted in ${runDir}/issues-en`,
    // Say what was left out and why: a harness that silently drops findings is
    // indistinguishable from one that never had them.
    fixtureSkipped ? `${fixtureSkipped} finding(s) excluded: they quote a value only the test fixture produces` : null,
    stale.length ? `${stale.length} candidate(s) skipped: their cases are no longer in the library` : null,
  ].filter(Boolean).join("; ");

  if (!file || !yes)
    return { message: `${summary}. Pass --yes to file them.`, drafts, stale: stale.length };

  if (!repo)
    throw new Error("--repo <owner/name> is required to file issues");
  const results = [];
  for (const draft of drafts) {
    try {
      const { stdout } = await execFileAsync("gh", ["issue", "create", "--repo", repo, "--title", draft.title, "--body-file", draft.file]);
      results.push({ ok: true, title: draft.title, url: stdout.trim() });
    }
    catch (error) {
      results.push({ ok: false, title: draft.title, error: String(error?.stderr ?? error?.message ?? error).slice(0, 300) });
    }
  }
  return {
    message: `${summary}\n${results.map(item => (item.ok ? `filed ${item.url}` : `FAILED ${item.title}: ${item.error}`)).join("\n")}`,
    drafts,
    results,
  };
}
