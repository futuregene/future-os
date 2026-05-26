import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { assertReadableFile } from "../utils/files.js";
import { runInheritedProcess } from "../utils/process.js";

export async function tui(args: string[]): Promise<void> {
  const entry = await resolveTuiEntry();
  const result = await runInheritedProcess(process.execPath, [entry, ...args]);
  process.exitCode = result.code;
}

async function resolveTuiEntry(): Promise<string> {
  const override = process.env.FUTURE_TUI_ENTRY;
  if (override) {
    await assertReadableFile(override, "FUTURE_TUI_ENTRY");
    return override;
  }

  const currentFile = fileURLToPath(import.meta.url);
  const cliRoot = resolve(dirname(currentFile), "..", "..");
  const entry = resolve(cliRoot, "..", "tui", "dist", "index.js");
  await assertReadableFile(
    entry,
    "TUI entry",
    "Build the TUI first with `cd future-os && make build-tui`.",
  );
  return entry;
}
