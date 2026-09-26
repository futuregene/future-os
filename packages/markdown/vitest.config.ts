/**
 * Test project for the three pure-logic workspace packages consumed by the
 * desktop app: `@future-os/markdown`, `@future-os/thread-projection` and
 * `@future-os/json-preview`.
 *
 * They had no test project of their own, so their tests were neither run nor
 * measured: desktop's vitest root is `desktop/`, and a `**` include glob cannot
 * ascend into a sibling directory. This config roots the runner at `packages/`
 * so each package's `src` tests are discovered, and writes the coverage summary
 * into the desktop coverage directory so the shared verifier
 * (`js-module group:d-pkgs`) reads it like any other desktop subtree.
 *
 * Run from `desktop/` with:
 *   npx vitest run --config ../packages/markdown/vitest.config.ts
 *
 * No imports here on purpose: `vitest` is only installed under
 * `desktop/node_modules`, so the config must load without resolving anything
 * from a package folder.
 */
export default {
  // The `packages/` directory: `../packages` resolves to the same directory
  // whether Vite anchors it on this config file or on the `desktop/` cwd.
  root: "../packages",
  test: {
    include: ["*/src/**/*.test.ts"],
    coverage: {
      provider: "v8",
      include: ["*/src/**/*.{ts,tsx}"],
      exclude: ["*/src/**/*.test.{ts,tsx}", "*/src/**/*.d.ts"],
      reporter: ["text", "json-summary", "json"],
      reportsDirectory: "../desktop/coverage/d-pkgs",
    },
  },
};
