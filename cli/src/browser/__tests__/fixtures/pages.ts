/**
 * Fixture page definitions for browser characterization tests.
 *
 * Instead of running an HTTP server, these pages use `page.setContent()`
 * to load HTML directly into the Playwright page. This avoids the need for
 * a local server that would be blocked by sandbox restrictions.
 *
 * For pages that need to simulate navigation, we use a base URL with
 * unique paths to allow `page.goto()`-style navigation.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

const pagesDir = join(import.meta.dirname, "pages");

// Load all fixture pages at import time
const FIXTURES: Record<string, string> = {};

for (const name of [
  "basic",
  "delayed-element",
  "form",
  "navigation-source",
  "navigation-target",
  "spa",
  "console",
  "multiple-matches",
  "scroll",
  "shadow-dom",
  "tabs",
]) {
  FIXTURES[name] = readFileSync(join(pagesDir, `${name}.html`), "utf-8");
}

export function getFixture(name: string): string {
  const html = FIXTURES[name];
  if (!html) throw new Error(`Unknown fixture: ${name}`);
  return html;
}

export function getFixtureNames(): string[] {
  return Object.keys(FIXTURES);
}

/**
 * Base URL for relative path resolution in fixture pages.
 * All fixtures use a common base so that /navigation-source → /navigation-target
 * links work within the same page context.
 */
export const FIXTURE_BASE_URL = "http://fixture.test";

/**
 * Get the full URL for a named fixture page.
 */
export function getFixtureUrl(name: string): string {
  return `${FIXTURE_BASE_URL}/${name}`;
}
