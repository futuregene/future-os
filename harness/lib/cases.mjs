/**
 * The case library: JSON files under `harness/cases/`.
 *
 * A case is deliberately not code. It says what a tester would do and what they
 * expect to see; the runner knows how to drive a browser, and the analysis step
 * knows how to read the evidence. That separation is what lets a case be
 * reviewed, replayed by hand, and refined over time without touching the
 * harness.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

export const CASES_DIR = resolve(import.meta.dirname, "..", "cases");

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory())
      return walk(path);
    return name.endsWith(".json") ? [path] : [];
  });
}

export function loadCases({ ids = [], tag = null, dir = CASES_DIR } = {}) {
  const cases = walk(dir).map((path) => {
    const data = JSON.parse(readFileSync(path, "utf8"));
    return { ...data, file: path };
  });
  for (const testCase of cases) {
    if (!testCase.id || !testCase.title)
      throw new Error(`case is missing id/title: ${testCase.file}`);
    if (!Array.isArray(testCase.steps) || !testCase.steps.length)
      throw new Error(`case has no steps: ${testCase.file}`);
    const duplicate = cases.find(other => other !== testCase && other.id === testCase.id);
    if (duplicate)
      throw new Error(`duplicate case id ${testCase.id}: ${testCase.file} and ${duplicate.file}`);
  }
  return cases
    .filter(testCase => !ids.length || ids.includes(testCase.id))
    .filter(testCase => !tag || (testCase.tags ?? []).includes(tag))
    .sort((a, b) => (a.priority ?? "P9").localeCompare(b.priority ?? "P9") || a.id.localeCompare(b.id));
}

export function listCases(options) {
  return loadCases(options).map(testCase => ({
    id: testCase.id,
    title: testCase.title,
    area: testCase.area,
    priority: testCase.priority,
    tags: testCase.tags ?? [],
    steps: testCase.steps.length,
  }));
}
