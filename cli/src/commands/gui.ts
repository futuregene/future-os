import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { assertExecutableFile, colocatedBinary } from "../utils/files.js";
import { runInheritedProcess } from "../utils/process.js";

export async function gui(): Promise<void> {
  // Packaged builds ship the GUI binary next to this executable.
  const colocated = await colocatedBinary("future-gui");
  if (colocated) {
    const result = await runInheritedProcess(colocated, []);
    process.exitCode = result.code;
    return;
  }

  // Dev / source checkout: run the GUI from the build directory.
  const currentFile = fileURLToPath(import.meta.url);
  const cliRoot = resolve(currentFile, "..", "..", "..");
  const guiRoot = resolve(cliRoot, "..", "gui", "src-tauri");
  const binary = resolve(guiRoot, "target", "debug", "futureos");
  await assertExecutableFile(binary, "GUI binary");
  const result = await runInheritedProcess(binary, []);
  process.exitCode = result.code;
}
