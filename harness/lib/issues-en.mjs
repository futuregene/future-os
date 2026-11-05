/**
 * English issue drafts from a run's findings.
 *
 * Two jobs beyond "write the title and body":
 *
 * 1. **Say which case produced it.** A finding is only actionable if someone can
 *    re-run it: the body carries the case id, the case's own expectations and the
 *    exact command, so a regression fix starts from a reproduction instead of a
 *    description.
 * 2. **Say how it was found.** A failed assertion, a model verdict and a model's
 *    free-form discovery carry different weight. The body states the source and
 *    the confidence, so a reader can calibrate instead of trusting every line the
 *    same amount.
 *
 * Screenshots are never referenced: the artefacts stay with the run, and an issue
 * has to stand on prose (a picture in an issue goes stale the moment the UI
 * changes, and these are filed before anyone has looked twice).
 */
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

const SEVERITY_ORDER = ["high", "medium", "low", "info"];

function slug(text) {
  return String(text).toLowerCase().replace(/[^\w]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 60) || "issue";
}

/** Words that carry no meaning for "are these two the same problem". */
const STOP = new Set(["the", "a", "an", "is", "are", "in", "on", "of", "to", "and", "or", "for", "with", "when", "after", "before", "while", "shows", "show", "does", "not"]);

function tokens(text) {
  return new Set(
    String(text ?? "")
      .toLowerCase()
      .split(/[^\p{L}\p{N}]+/u)
      .filter(word => word.length > 1 && !STOP.has(word)),
  );
}

/** Jaccard overlap; enough to catch "the same sentence, twice", and no more. */
function similarity(a, b) {
  const left = tokens(a);
  const right = tokens(b);
  if (!left.size || !right.size)
    return 0;
  let shared = 0;
  for (const word of left)
    if (right.has(word))
      shared += 1;
  return shared / (left.size + right.size - shared);
}

/** Deterministic first, then model verdicts, then free discovery. */
const SOURCE_ORDER = { check: 0, judge: 1, model: 2, "cross-case": 3, harness: 4 };

function sourceLabel(finding) {
  if (finding.source === "cross-case")
    return "a cross-case discovery pass (a model comparing several sessions)";
  if (finding.source === "judge" && finding.discovered)
    return "a model discovery pass over the screenshots and logs of one session";
  if (finding.source === "judge")
    return "a model verdict against the case's own expectations";
  if (finding.source === "model")
    return "a model reading the case timeline and its expected outcome";
  if (finding.source === "check" || finding.kind === "check")
    return "a deterministic assertion in the case";
  return "the harness";
}

function caseLookup(analysis) {
  const map = new Map();
  for (const item of analysis.cases ?? [])
    map.set(item.id, item);
  return map;
}

function reproduceCommand(caseId) {
  return `node harness/cli.mjs run --case ${caseId} --analyze full`;
}

function bodyFor(group, analysis, cases, localized = null) {
  const primary = group.primary;
  const caseIds = group.caseIds;
  const caseTitles = localized?.case_titles ?? {};
  const caseLines = caseIds.map((id) => {
    const title = caseTitles[id];
    return `- \`${id}\`${title ? ` — ${title}` : ""}`;
  });

  const expectations = [];
  for (const id of caseIds) {
    const item = cases.get(id);
    for (const expectation of item?.expectations ?? [])
      if (!expectations.includes(expectation))
        expectations.push(expectation);
  }

  // The observations are rewritten in English by the localizer; the raw finding
  // text stays with the run. Falling back to the original would put another
  // language into an English issue, which is the one thing the body must not do.
  const occurrences = (localized?.notes?.length ? localized.notes : []).map((note, index) => {
    const finding = group.findings[index] ?? group.findings[0];
    const where = finding?.evidence?.caseId ?? "unknown case";
    return `- ${note} _(${finding?.severity ?? "info"}, from ${sourceLabel(finding ?? {})}, case \`${where}\`)_`;
  });

  const what = localized?.what ?? (primary.detail ? String(primary.detail).trim() : null);
  const expectedText = localized?.expected ?? null;
  const impact = localized?.impact ?? null;

  return [
    "## What happens",
    "",
    what ?? "(see the observations below)",
    "",
    "## How it was found",
    "",
    `This was produced automatically by the FutureOS UI harness, from ${sourceLabel(primary)}.`,
    primary.confidence ? `Confidence: **${primary.confidence}**.` : null,
    primary.demoted ? `Note: ${primary.demoted}` : null,
    "",
    "## Where it comes from (regression cases)",
    "",
    ...caseLines,
    "",
    caseIds.length
      ? ["Reproduce with:", "", "```bash", ...caseIds.map(id => reproduceCommand(id)), "```"].join("\n")
      : null,
    "",
    expectedText
      ? ["## Expected behaviour", "", expectedText].join("\n")
      : (expectations.length ? ["## Expected behaviour", "", ...expectations.map(item => `- ${item}`)].join("\n") : null),
    "",
    "## Observations collected",
    "",
    ...occurrences,
    "",
    impact ? ["## Why it matters", "", impact].join("\n") : null,
    primary.fix && !impact ? ["## Suggested direction", "", primary.fix].join("\n") : null,
    "",
    "## Environment",
    "",
    "- Desktop: the real frontend rendered by Vite, Tauri IPC served by the harness fixture server over a copy of a real `app.db`.",
    "- Mobile / remote: the Expo web build of the mobile app; remote pairs run against a self-hosted relay with real processes.",
    "- Everything is in the repository's `harness/` directory; `harness/README.md` describes the surfaces and `harness/COVERAGE.md` what is exercised.",

  ].filter(Boolean).join("\n");
}

/**
 * Collapse findings into one issue per problem.
 *
 * Grouping is deliberately conservative: the same case plus a high title overlap,
 * or the same normalised title across cases. Anything less certain stays a
 * separate issue — a merged issue that hides half its evidence is worse than two
 * issues a maintainer closes as duplicates.
 */
export function groupFindings(findings) {
  const groups = [];
  for (const finding of [...findings].sort((a, b) => (SOURCE_ORDER[a.source] ?? 9) - (SOURCE_ORDER[b.source] ?? 9))) {
    const caseId = finding.evidence?.caseId ?? "unknown";
    const match = groups.find((group) => {
      const titleOverlap = similarity(group.primary.title, finding.title);
      const detailOverlap = similarity(group.primary.detail ?? "", finding.detail ?? "");
      // Same case and the same words in either field is the same defect said
      // twice; across cases it takes a much closer match to merge.
      if (group.caseIds.includes(caseId) && (titleOverlap >= 0.35 || detailOverlap >= 0.5))
        return true;
      return titleOverlap >= 0.7;
    });
    if (match) {
      match.findings.push(finding);
      if (!match.caseIds.includes(caseId))
        match.caseIds.push(caseId);
      if (SEVERITY_ORDER.indexOf(finding.severity) < SEVERITY_ORDER.indexOf(match.primary.severity))
        match.primary = finding;
      continue;
    }
    groups.push({ primary: finding, findings: [finding], caseIds: [caseId] });
  }
  return groups;
}

export function draftEnglishIssues(runDir, { minSeverity = "medium", include = ["check", "judge", "model", "cross-case"] } = {}) {
  runDir = resolve(runDir);
  const analysisPath = join(runDir, "analysis.json");
  if (!existsSync(analysisPath))
    throw new Error(`no analysis.json in ${runDir}; run analyze first`);
  const analysis = JSON.parse(readFileSync(analysisPath, "utf8"));
  const cases = caseLookup(analysis);

  const cutoff = SEVERITY_ORDER.indexOf(minSeverity);
  const findings = (analysis.findings ?? []).filter((finding) => {
    if (!include.includes(finding.source))
      return false;
    // A failed assertion is a defect whatever its severity label says: the case
    // said this must hold and it did not.
    if (finding.source === "check")
      return true;
    return SEVERITY_ORDER.indexOf(finding.severity) <= cutoff;
  });

  for (const finding of findings) {
    const item = cases.get(finding.evidence?.caseId);
    if (!finding.expectations && item?.expectations?.length)
      finding.expectations = item.expectations;
  }

  const groups = groupFindings(findings);
  groups.sort((a, b) => SEVERITY_ORDER.indexOf(a.primary.severity) - SEVERITY_ORDER.indexOf(b.primary.severity));
  return { analysis, cases, groups, runId: analysis.runId };
}

/** Write one markdown file per (already merged) issue and return their manifest. */
export function writeEnglishIssues(runDir, merged, { analysis, cases }) {
  const dir = join(runDir, "issues-en");
  mkdirSync(dir, { recursive: true });
  // Drafts from an earlier pass share the numeric prefixes but not the slugs;
  // leaving them behind makes the directory disagree with drafts.json.
  for (const name of readdirSync(dir))
    if (name.endsWith(".md") && !name.startsWith("_"))
      rmSync(join(dir, name), { force: true });
  const drafts = merged.map((entry, index) => {
    const group = { primary: entry.primary, findings: entry.groups.flatMap(item => item.findings), caseIds: entry.caseIds };
    const text = entry.texts.find(Boolean) ?? null;
    const file = join(dir, `${String(index + 1).padStart(2, "0")}-${slug(text?.title ?? entry.primary.title)}.md`);
    writeFileSync(file, bodyFor(group, analysis, cases, text));
    return {
      file,
      title: `[UI harness] ${text?.title ?? entry.primary.title}`,
      severity: entry.primary.severity,
      caseIds: entry.caseIds,
      occurrences: group.findings.length,
      discovered: group.findings.some(finding => finding.discovered),
    };
  });
  writeFileSync(join(dir, "drafts.json"), JSON.stringify({ runId: analysis.runId, generatedAt: new Date().toISOString(), drafts }, null, 2));
  return drafts;
}

export async function localizeDrafts(groups, { runPrompt, runDir, model, batch = 8 } = {}) {
  const results = new Map();
  // Batched on purpose: one call covering thirty groups tends to come back with
  // fewer entries than it was given, and an entry without English text is an
  // issue in the wrong language. Eight at a time is small enough that a model
  // answers all of them, and a batch that still loses entries is retried alone.
  for (let offset = 0; offset < groups.length; offset += batch) {
    const slice = groups.slice(offset, offset + batch);
    const prompt = buildLocalizePrompt(slice, offset);
    const promptPath = join(runDir, "issues-en", `_localize-${offset / batch + 1}.md`);
    mkdirSync(join(runDir, "issues-en"), { recursive: true });
    writeFileSync(promptPath, prompt);
    const collected = await askForEnglish({ promptPath, runPrompt, model, runDir, offset, slice });
    for (const [index, text] of collected)
      results.set(index, text);
  }
  return groups.map((group, index) => {
    const text = results.get(index + 1) ?? null;
    return { group, text, duplicateOf: text?.duplicate_of == null ? null : Number(text.duplicate_of) };
  });
}

function buildLocalizePrompt(slice, offset) {
  return [
    "You are writing GitHub issues in English for a UI bug tracker.",
    "For each numbered finding below, write: a short imperative title (max 90 chars, no trailing period),",
    "a factual description of what the user sees, what should happen instead, and why it matters.",
    "Rules:",
    "- English only, plain language, no marketing tone. UI strings quoted from the product keep their original characters.",
    "- Describe only what the finding says; do not invent screens, causes or numbers.",
    "- If the finding is a model observation rather than a verified defect, phrase it as such (\"appears to\", \"may\").",
    "- Keep each field under 90 words.",
    "- An entry whose source is \"a deterministic assertion in the case\" is a failed check. Title it around the user-visible failure that assertion detected (the wrong output, the missing state), never around the assertion or a step number — a reader cannot act on \"step 11 failed\".",
    "- `notes` must have exactly one English sentence per observation listed under that entry, in the same order.",
    "- `case_titles` maps every case id named under the entry to an English title for that test case.",
    "",
    "Answer with strict JSON (no markdown fence):",
    `{"issues":[{"index":${offset + 1},"title":"...","what":"...","expected":"...","impact":"...","notes":["..."],"case_titles":{"case-id":"English title"},"duplicate_of":null}]}`,
    "`index` continues the numbering used below.",
    "If two entries describe the same defect, set `duplicate_of` on the later one to the earlier index; otherwise null.",
    "Two entries from the SAME case are the same defect when one is a failed assertion and the other describes the same user-visible failure — merge those.",
    "",
    ...slice.map((group, index) => [
      `#${offset + index + 1} severity=${group.primary.severity} source=${sourceLabel(group.primary)} cases=${group.caseIds.join(",")}`,
      `observed: ${String(group.primary.detail ?? group.primary.title).replace(/\s+/g, " ").slice(0, 900)}`,
      group.findings.length > 1 ? `also reported: ${group.findings.slice(1).map(item => item.title).join(" | ").slice(0, 500)}` : null,
      group.primary.fix ? `suggested: ${group.primary.fix}` : null,
      group.primary.expectations?.length ? `case expectations: ${group.primary.expectations.join(" | ").slice(0, 400)}` : null,
    ].filter(Boolean).join("\n")).join("\n\n"),
  ].join("\n");
}

async function askForEnglish({ promptPath, runPrompt, model, runDir, offset, slice, attempt = 1 }) {
  const collected = new Map();
  try {
    const { text } = await runPrompt(promptPath, { model });
    writeFileSync(join(runDir, "issues-en", `_localize-${offset / 8 + 1}-answer-${attempt}.md`), text);
    const parsed = JSON.parse(text.slice(text.indexOf("{"), text.lastIndexOf("}") + 1));
    for (const item of parsed.issues ?? [])
      if (item?.index >= offset + 1 && item?.index <= offset + slice.length && item?.title)
        collected.set(Number(item.index), item);
  }
  catch {}
  const missing = slice.map((_, index) => offset + index + 1).filter(index => !collected.has(index));
  if (missing.length && attempt < 3) {
    // Retry only what is missing, one entry at a time, so a bad batch cannot cost
    // the whole set.
    for (const index of missing) {
      const one = slice[index - offset - 1];
      const path = join(runDir, "issues-en", `_localize-retry-${index}.md`);
      writeFileSync(path, buildLocalizePrompt([one], index - 1));
      const more = await askForEnglish({ promptPath: path, runPrompt, model, runDir, offset: index - 1, slice: [one], attempt: attempt + 1 });
      for (const [key, value] of more)
        collected.set(key, value);
    }
  }
  return collected;
}

/**
 * Merge drafts whose English titles say the same thing.
 *
 * The first clustering runs on the findings as produced, which are often in
 * different languages and wordings for one defect — a raw failed-check message,
 * a Chinese sentence, a model's paraphrase. Once every group has an English
 * title, the same defect reads the same way twice, and merging on that is
 * reliable in a way that merging on the original text was not.
 */
export function mergeLocalized(entries, { threshold = 0.45 } = {}) {
  const merged = [];
  for (const [index, entry] of entries.entries()) {
    const title = entry.text?.title ?? entry.group.primary.title;
    // The model is asked to name duplicates; its answer outranks string
    // similarity, because "replace the SQLite text" and "show a readable
    // duplicate-name message" are the same defect in different words.
    const declared = entry.duplicateOf ? merged[entry.duplicateOf - 1] : null;
    const existing = declared ?? merged.find(item => similarity(item.title, title) >= threshold);
    if (!existing) {
      merged.push({ title, groups: [entry.group], texts: [entry.text], caseIds: [...entry.group.caseIds], primary: entry.group.primary });
      continue;
    }
    existing.groups.push(entry.group);
    existing.texts.push(entry.text);
    for (const id of entry.group.caseIds)
      if (!existing.caseIds.includes(id))
        existing.caseIds.push(id);
    const rank = SEVERITY_ORDER.indexOf(entry.group.primary.severity);
    if (rank < SEVERITY_ORDER.indexOf(existing.primary.severity))
      existing.primary = entry.group.primary;
  }
  return merged;
}

export async function fileEnglishIssues(runDir, { repo, yes = false, labels = ["bug", "ui-harness"] } = {}) {
  const { dir, drafts } = draftEnglishIssues(runDir);
  if (!yes)
    return { message: `Drafted ${drafts.length} issue(s) in ${dir}. Re-run with --yes to file them.`, drafts };
  if (!repo)
    throw new Error("--repo <owner/name> is required to file issues");
  const results = [];
  for (const draft of drafts) {
    try {
      const args = ["issue", "create", "--repo", repo, "--title", draft.title, "--body-file", draft.file];
      const { stdout } = await execFileAsync("gh", args);
      results.push({ ok: true, title: draft.title, url: stdout.trim() });
    }
    catch (error) {
      results.push({ ok: false, title: draft.title, error: String(error?.stderr ?? error?.message ?? error).slice(0, 300) });
    }
  }
  return { message: results.map(item => (item.ok ? `filed: ${item.url}` : `failed: ${item.title} — ${item.error}`)).join("\n"), results };
}
