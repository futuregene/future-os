/** @jest-environment node */

import { readFileSync } from "node:fs";
import path from "node:path";
import { resources } from "../../../i18n/locales";

/**
 * Every i18n key the task pages reference must exist in both bundles.
 *
 * This is the guard for a bug the screenshot pass caught and code review did
 * not: the form asked for `tasks.form.modelDefault` while the mobile bundle
 * keeps that key at `tasks.modelDefault`, so the picker rendered the raw key
 * where a label belongs. Rendering it is the only way to notice at runtime, and
 * the screenshot pass is not in CI — this is.
 *
 * The mobile bundle is one flat namespace, unlike the desktop's per-namespace
 * files, so a key copied from the desktop can be right there and wrong here.
 */
function keysUsedBy(relativePath: string): string[] {
  const source = readFileSync(path.resolve(__dirname, "..", relativePath), "utf8");  const found = new Set<string>();
  for (const match of source.matchAll(/\bt\(\s*"([a-zA-Z][a-zA-Z0-9_.]*)"/g))
    found.add(match[1]!);
  // `t(key ? "a" : "b")` and `t(\`a.${x}\`)` are handled by the literal scan
  // above for the first case; the template case is covered below.
  for (const match of source.matchAll(/\bt\(\s*`([a-zA-Z][a-zA-Z0-9_.]*)\.\$\{/g))
    found.add(`${match[1]}.`);
  return [...found].sort();
}

/** Resolve `a.b.c` inside a resources bundle, tolerating a trailing `.` prefix. */
function lookup(bundle: unknown, key: string): unknown {
  const parts = key.split(".").filter(Boolean);
  let node: unknown = bundle;
  for (const part of parts) {
    if (typeof node !== "object" || node === null)
      return undefined;
    node = (node as Record<string, unknown>)[part];
  }
  // A template-prefix key only has to resolve to an object of leaves.
  return node;
}

const pages = ["TasksSettingsPage.tsx", "SettingsScreen.tsx"];

for (const locale of ["en", "zh"] as const) {
  const bundle = resources[locale].translation;
  for (const page of pages) {
    test(`${page} uses only keys that exist in the ${locale} bundle`, () => {
      const missing = keysUsedBy(page).filter(key => lookup(bundle, key) === undefined);
      expect(missing).toEqual([]);
    });
  }
}

test("the two bundles define the same task keys", () => {
  const flat = (node: unknown, prefix = ""): string[] => {
    if (typeof node !== "object" || node === null)
      return [prefix];
    return Object.entries(node as Record<string, unknown>)
      .flatMap(([key, value]) => flat(value, prefix ? `${prefix}.${key}` : key));
  };
  const en = flat(resources.en.translation.tasks).sort();
  const zh = flat(resources.zh.translation.tasks).sort();
  expect(en.filter(key => !zh.includes(key))).toEqual([]);
  expect(zh.filter(key => !en.includes(key))).toEqual([]);
});
