import path from "node:path";
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/**
 * Screenshot harness config for the desktop frontend (`make screenshots-desktop`).
 *
 * Renders the REAL application with the `@tauri-apps/*` boundary replaced by
 * in-page mocks (see `shot/`), so product screenshots can be captured on a
 * machine with no display — CI, a headless box, or a remote shell.
 *
 * Only the transport is faked: every component, hook, store and style under
 * `src/` is the shipped implementation.
 */
const here = path.dirname(fileURLToPath(import.meta.url));
const shotDir = path.join(here, "shot");
const mock = (name: string) => path.join(shotDir, "mock", name);

const MOCKS: Record<string, string> = {
  "@tauri-apps/api/core": mock("core.ts"),
  "@tauri-apps/api/event": mock("event.ts"),
  "@tauri-apps/api/window": mock("window.ts"),
  "@tauri-apps/api/webview": mock("webview.ts"),
  "@tauri-apps/plugin-dialog": mock("dialog.ts"),
};

/**
 * Rewrite the Tauri imports in the app's own module text rather than through
 * `resolve.alias`: Vite pre-bundles bare specifiers during dependency
 * optimization, before the alias resolver runs, so an alias alone would leave
 * the real `@tauri-apps/api` modules in the graph.
 */
function tauriMock() {
  return {
    name: "tauri-mock",
    enforce: "pre" as const,
    transform(code: string, id: string) {
      if (id.includes("node_modules") || id.startsWith("\0"))
        return null;
      let out = code;
      for (const [specifier, target] of Object.entries(MOCKS)) {
        for (const quote of ["\"", "'"]) {
          const from = `${quote}${specifier}${quote}`;
          if (out.includes(from))
            out = out.split(from).join(`${quote}/@fs${target}${quote}`);
        }
      }
      return out === code ? null : out;
    },
  };
}

export default defineConfig({
  plugins: [tauriMock(), react(), tailwindcss({ optimize: false })],
  clearScreen: false,
  resolve: {
    alias: {
      "decode-named-character-reference": require.resolve("decode-named-character-reference"),
    },
  },
  server: {
    // `vite.shot.config.ts` runs in Node; the harness passes SHOT_PORT when the
    // default is taken (see scripts/screenshots/capture.py).
    // eslint-disable-next-line node/prefer-global/process
    port: Number(process.env.SHOT_PORT ?? 5199),
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
