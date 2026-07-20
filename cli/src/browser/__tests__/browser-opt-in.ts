/**
 * Opt-in gate for real-browser test suites (validation + characterization).
 *
 * These suites launch actual Chrome/Safari processes and can take minutes.
 * Raw `bun test` must stay fast and side-effect free, so they only run when
 * explicitly requested:
 *
 *   FUTURE_BROWSER_TESTS=1 bun test            # or: npm run test:browser
 *
 * Without the flag, every suite degrades to its existing "browser not
 * available" skip path (tests pass with a `[skip]` log line).
 */
export const RUN_BROWSER_TESTS = !!process.env.FUTURE_BROWSER_TESTS;

export function logBrowserSuiteSkipped(tag: string): void {
  console.log(`  [${tag}] real-browser suite skipped — set FUTURE_BROWSER_TESTS=1 (npm run test:browser) to enable`);
}
