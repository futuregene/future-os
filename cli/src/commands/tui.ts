import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { assertExecutableFile, assertReadableFile, which } from "../utils/files.js";
import { runInheritedProcess } from "../utils/process.js";

// The TUI is launched either as a standalone compiled binary (packaged builds)
// or as a JS entry run with the current runtime (dev / source checkout).
type TuiTarget =
  | { kind: "binary"; path: string }
  | { kind: "entry"; path: string };

export async function tui(args: string[]): Promise<void> {
  const target = await resolveTuiTarget();
  const result = target.kind === "binary"
    ? await runInheritedProcess(target.path, args)
    : await runInheritedProcess(process.execPath, [target.path, ...args]);
  process.exitCode = result.code;
}

async function resolveTuiTarget(): Promise<TuiTarget> {
  // Explicit compiled-binary override.
  const binOverride = process.env.FUTURE_TUI_BIN;
  if (binOverride) {
    await assertExecutableFile(binOverride, "FUTURE_TUI_BIN");
    return { kind: "binary", path: binOverride };
  }

  // Explicit JS-entry override (run with the current runtime).
  const entryOverride = process.env.FUTURE_TUI_ENTRY;
  if (entryOverride) {
    await assertReadableFile(entryOverride, "FUTURE_TUI_ENTRY");
    return { kind: "entry", path: entryOverride };
  }

  // Look up the compiled binary on PATH — covers make install to $PREFIX
  // and packaged builds where future-tui sits beside the CLI.
  const onPath = await which("future-tui");
  if (onPath) return { kind: "binary", path: onPath };

  // Dev / source checkout: run the TUI's JS entry with the current runtime.
  const currentFile = fileURLToPath(import.meta.url);
  const cliRoot = resolve(dirname(currentFile), "..", "..");
  const entry = resolve(cliRoot, "..", "tui", "dist", "index.js");
  await assertReadableFile(
    entry,
    "TUI entry",
    "Build the TUI first with `cd future-os && make build-tui`.",
  );
  return { kind: "entry", path: entry };
}
