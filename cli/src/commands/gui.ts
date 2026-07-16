import { assertExecutableFile, which } from "../utils/files.js";
import { runInheritedProcess } from "../utils/process.js";

export async function gui(): Promise<void> {
  // Look up on PATH — make install copies the GUI binary to $PREFIX as
  // `future-gui` (the cargo binary inside src-tauri is named `futureos`).
  const onPath = await which("future-gui");
  if (onPath) {
    const result = await runInheritedProcess(onPath, []);
    process.exitCode = result.code;
    return;
  }

  // Dev / source checkout: run the GUI from the build directory.
  const binary = resolve(
    fileURLToPath(import.meta.url),
    "..", "..", "..", "..",
    "gui", "src-tauri", "target", "debug", "futureos",
  );
  await assertExecutableFile(binary, "GUI binary");
  const result = await runInheritedProcess(binary, []);
  process.exitCode = result.code;
}
