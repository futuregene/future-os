import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
    maxWorkers: 2,
    // CommonJS Noise resolves sodium before Vite can apply ESM aliases.
    pool: "forks",
    execArgv: ["--require", fileURLToPath(new URL("./tests/noiseBackend.cjs", import.meta.url))],
  },
});
