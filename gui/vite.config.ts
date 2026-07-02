/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
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
