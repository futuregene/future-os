/// <reference types="vitest/config" />
import { readdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// pdf.js v6 ships its image decoders (JBIG2 / OpenJPEG / QCMS) as WebAssembly and
// resolves them at runtime from a `wasmUrl` base directory. They aren't part of the
// JS bundle, so without this scanned/image-only PDFs render as a blank page. Serve
// the wasm/ directory verbatim at `/pdfjs-wasm/` in both dev and the built bundle;
// the frontend points `getDocument({ wasmUrl })` here. Keep the served filenames
// unhashed — pdf.js appends exact names (`jbig2.wasm`, `*_nowasm_fallback.js`, …).
function pdfjsWasm(): Plugin {
  const require = createRequire(import.meta.url);
  const wasmDir = path.join(path.dirname(require.resolve("pdfjs-dist/package.json")), "wasm");
  const routePrefix = "/pdfjs-wasm/";
  const assets = () => readdirSync(wasmDir).filter(n => n.endsWith(".wasm") || n.endsWith(".js"));
  return {
    name: "pdfjs-wasm",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = req.url?.split("?")[0];
        if (!url || !url.startsWith(routePrefix))
          return next();
        const name = url.slice(routePrefix.length);
        try {
          const buf = readFileSync(path.join(wasmDir, name));
          res.setHeader("Content-Type", name.endsWith(".wasm") ? "application/wasm" : "text/javascript");
          res.end(buf);
        }
        catch {
          next();
        }
      });
    },
    generateBundle() {
      for (const name of assets()) {
        this.emitFile({
          type: "asset",
          fileName: `pdfjs-wasm/${name}`,
          source: readFileSync(path.join(wasmDir, name)),
        });
      }
    },
  };
}

export default defineConfig({
  // Let Vite minify CSS once with esbuild below. Tailwind's Lightning CSS
  // optimizer currently warns on the valid CSS Custom Highlight ::highlight().
  plugins: [react(), tailwindcss({ optimize: false }), pdfjsWasm()],
  clearScreen: false,
  resolve: {
    alias: {
      // Vite's browser condition selects index.dom.js, which touches document
      // at import time and crashes the Markdown worker. Use the DOM-free entry
      // in both main/worker graphs (dev dependency prebundling shares them).
      "decode-named-character-reference": createRequire(import.meta.url).resolve("decode-named-character-reference"),
    },
  },
  test: {
    setupFiles: ["./src/test/i18nTestSetup.ts"],
    // Cap the worker pool below the machine's core count. Vitest's default is
    // `cpus - 1`, which assumes the whole box belongs to this suite; on a shared
    // machine (a developer's laptop also running a Rust build, or CI running
    // several jobs) it oversubscribes and a worker can fail to COLLECT its file
    // at all — observed here as
    //   "Error: No test suite found in file .../SettingsDialog.test.tsx"
    // with the other 244 files green and 3152 tests passing, i.e. one file's
    // tests never registered. It reproduced only under load (≈1 run in 4 while
    // two other Node workers were busy; 6/6 green when that file ran alone) and
    // the failing run was the slowest of the four, so the mechanism is
    // contention rather than anything the file does. Halving the pool removes
    // the oversubscription instead of papering over it. Same shape as the
    // `maxWorkers: "50%"` cap in mobile/jest.config.js, which fixed the same
    // class of failure there.
    maxWorkers: "50%",
    coverage: {
      // Vitest 4 dropped `coverage.all`; `include` is now the way to pull in
      // files no test imported, so the summary reports the whole `src/` tree
      // instead of only the modules a test happened to touch.
      //
      // The sibling `packages/*` sources are NOT listable here: a `**` include
      // glob cannot ascend out of the project root (verified empirically —
      // adding `../packages/*/src/**` yields zero packages entries). Those
      // packages have their own runner at packages/markdown/vitest.config.ts,
      // which writes desktop/coverage/d-pkgs/.
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/**/*.test.{ts,tsx}",
        "src/test/**",
        "src/**/*.d.ts",
      ],
      reporter: ["text", "json-summary", "json"],
      reportsDirectory: "./coverage",
    },
  },
  build: {
    cssMinify: "esbuild",
    rolldownOptions: {
      checks: {
        // Plugin timings are profiling advice, not actionable install warnings.
        // Keep all correctness checks and other build warnings enabled.
        pluginTimings: false,
      },
    },
    chunkSizeWarningLimit: 2000, // suppress "chunk larger than 500 kB" warnings
  },
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // Don't watch the Rust side: cargo writes into src-tauri/target while it
      // builds, and on Windows the fs watcher throws EBUSY on those files and
      // crashes the dev server. (macOS tolerates it, hence Windows-only.)
      ignored: ["**/src-tauri/**"]
    }
  },
  envPrefix: ["VITE_", "TAURI_"]
});
