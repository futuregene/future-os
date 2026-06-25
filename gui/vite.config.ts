import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
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
