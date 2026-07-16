import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { assertExecutableFile, assertReadableFile, colocatedBinary } from "../utils/files.js";
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

  // Packaged builds ship the compiled future-tui next to this executable.
  const colocated = await colocatedBinary("future-tui");
  if (colocated) return { kind: "binary", path: colocated };

  // Dev / source checkout: run the TUI's JS entry with the current runtime.
  let cliRoot: string;
  try { cliRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", ".."); }
  catch { cliRoot = resolve(dirname(process.execPath), ".."); }
  const entry = resolve(cliRoot, "..", "tui", "dist", "index.js");
  await assertReadableFile(
    entry,
    "TUI entry",
    "Build the TUI first with `cd future-os && make build-tui`.",
  );
  return { kind: "entry", path: entry };
}
