/**
 * Discovery, second level: what only shows up *across* cases.
 *
 * The per-case pass looks at one run's evidence with no checklist. It cannot see
 * the class of problem that is only visible when several screens are put side by
 * side — the same object called three names, an empty state that contradicts a
 * count on the next screen, a warning dot with nothing behind it. Those are real
 * product defects and they are exactly what a person notices after a morning of
 * clicking around.
 *
 * So this pass reads only text: every case's title, expectations, verdict,
 * summary and the findings already made. No images, no screenshots — the point is
 * the relation between what different cases observed, and images would pull it
 * back into per-screen reading.
 *
 * Two rules keep it honest:
 * - A systemic claim must name the cases it came from. A finding with one case
 *   and no cross-case evidence is not systemic; it belongs to that case.
 * - It may not invent a state that no case reached. If the evidence only shows
 *   an empty page, "the list fails to render devices" is not a conclusion.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const MAX_CASES = 40;

function summarizeCase(item) {
  const findings = (item.findings ?? [])
    .filter(finding => finding.source !== "harness")
    .map(finding => `  - [${finding.severity}] ${finding.title}`)
    .join("\n");
  // A case carried over from an earlier pass is the *summary* record, where
  // `checks` is a count; a case analysed this pass holds the array. Both reach
  // this function, so it reads either.
  const checkList = Array.isArray(item.checks) ? item.checks : [];
  const failed = checkList.filter(check => !check.ok).length;
  const checks = failed ? `  - ${failed} of ${checkList.length} checks failed` : "";
  return [
    `### ${item.id} — ${item.title}`,
    `surface: ${item.surface ?? "desktop"} · status: ${item.status}`,
    item.expectations?.length ? `expectations:\n${item.expectations.map(text => `  - ${text}`).join("\n")}` : null,
    item.summary ? `verdict note: ${item.summary}` : null,
    item.judge?.summary ? `judge: ${item.judge.summary}` : null,
    checks || null,
    findings || null,
  ].filter(Boolean).join("\n");
}

export function buildCrossCasePrompt(analysis) {
  const cases = analysis.cases.slice(0, MAX_CASES);
  return [
    "You are a senior product reviewer who has just spent a day clicking through this application with a test team.",
    "Below is the record of what each session found. Read it as a whole, not session by session.",
    "",
    "Your job: name the problems that only appear when these sessions are compared. Examples of the kind:",
    "- one concept with several names, or one name for several concepts, across screens;",
    "- a count, a title or a timestamp that disagrees between two places that describe the same object;",
    "- an empty state that says something different from what the rest of the product implies;",
    "- a control that looks actionable in one place and inert in another;",
    "- the same operation reporting success in one case and silence in another;",
    "- a class of state (loading, standby, disconnected) that one screen explains and another does not.",
    "",
    "Rules:",
    "- Every finding must list the case ids it is derived from (`cases`), and at least one of them must show the problem. If you cannot name a case, do not report it.",
    "- Only report what the record shows. Do not infer a screen you cannot see, and do not assume a state the cases never reached.",
    "- If the same problem was already reported per case, report it once here only if the cross-case view adds something (it spans screens, or its scope is larger than the one case).",
    "- Be sparing: two or three solid systemic findings are worth more than ten restatements. If nothing is systemic, return an empty list.",
    "",
    "Sessions:",
    cases.map(summarizeCase).join("\n\n"),
    "",
    "Answer with strict JSON (no markdown fence):",
    `{"findings":[{"severity":"high|medium|low","title":"short English title","detail":"what is inconsistent, why it matters to a user, and how the named cases show it","cases":["case-id"],"fix":"one sentence on what the product should do instead"}]}`,
  ].join("\n");
}

/**
 * Run the cross-case pass and return findings shaped like every other finding.
 *
 * `runPrompt` is injected so this stays testable without a model.
 */
export async function discoverAcrossCases({ analysis, runDir, runPrompt, model }) {
  const prompt = buildCrossCasePrompt(analysis);
  const promptPath = join(runDir, "analysis", "cross-case.discover.md");
  mkdirSync(join(runDir, "analysis"), { recursive: true });
  writeFileSync(promptPath, prompt);
  let text = "";
  try {
    ({ text } = await runPrompt(promptPath, { model }));
    writeFileSync(join(runDir, "analysis", "cross-case.discover-answer.md"), text);
  }
  catch (error) {
    return { findings: [], error: String(error?.message ?? error) };
  }
  const parsed = parseFindings(text) ?? {};
  const known = new Set(analysis.cases.map(item => item.id));
  const findings = (parsed.findings ?? [])
    .filter(item => Array.isArray(item.cases) && item.cases.some(id => known.has(id)))
    .map((item, index) => ({
      id: `cross-case:${index}`,
      severity: ["high", "medium", "low"].includes(item.severity) ? item.severity : "low",
      category: "consistency",
      title: item.title ?? "Cross-case inconsistency",
      detail: item.detail ?? "",
      fix: item.fix ?? null,
      evidence: { caseId: item.cases[0], caseIds: item.cases, crossCase: true },
      source: "cross-case",
      discovered: true,
      confidence: "medium",
    }));
  return { findings, error: null };
}

function parseFindings(text) {
  const candidates = [];
  const fenced = text.match(/```(?:json)?\s*([\s\S]*?)```/g) ?? [];
  candidates.push(...fenced.map(block => block.replace(/```(?:json)?/g, "").replace(/```/g, "")));
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start >= 0 && end > start)
    candidates.push(text.slice(start, end + 1));
  for (const candidate of candidates) {
    try {
      const parsed = JSON.parse(candidate.trim());
      if (parsed && Array.isArray(parsed.findings))
        return parsed;
    }
    catch {}
  }
  return null;
}
