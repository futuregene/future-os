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
  plugins: [react(), tailwindcss(), pdfjsWasm()],
  clearScreen: false,
  test: {
    setupFiles: ["./src/test/i18nTestSetup.ts"],
  },
  build: {
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
