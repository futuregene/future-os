import { constants as fsConstants } from "node:fs";
import { access } from "node:fs/promises";
import { spawn } from "node:child_process";

export async function assertReadableFile(
  path: string,
  label: string,
  hint?: string,
): Promise<void> {
  try {
    await access(path);
  } catch {
    throw new Error(`${label} not found at ${path}.${hint ? ` ${hint}` : ""}`);
  }
}

export async function assertExecutableFile(path: string, label: string): Promise<void> {
  if (!(await canAccess(path, fsConstants.X_OK))) {
    throw new Error(`${label} not found or not executable at ${path}.`);
  }
}

export async function canAccess(path: string, mode: number): Promise<boolean> {
  try {
    await access(path, mode);
    return true;
  } catch {
    return false;
  }
}

/**
 * Find an executable on PATH.  Returns the first match, or null.
 * On Unix this shells out to `which`; on Windows to `where`.
 */
export function which(name: string): Promise<string | null> {
  return new Promise((resolve) => {
    const cmd = process.platform === "win32" ? "where" : "which";
    // On Windows the extension is part of the search; "where future-agent"
    // won't find "future-agent.exe", but "where future-agent.exe" often
    // works without the extension too.  Be explicit to be safe.
    const target = process.platform === "win32" ? `${name}.exe` : name;
    const child = spawn(cmd, [target], {
      stdio: ["ignore", "pipe", "ignore"],
      windowsHide: true,
    });
    let stdout = "";
    child.stdout.on("data", (chunk: Buffer) => {
      stdout += chunk.toString();
    });
    child.on("close", (code) => {
      if (code === 0 && stdout.trim()) {
        // Take the first line (which may print multiple on Windows).
        resolve(stdout.trim().split("\n")[0].trim());
      } else {
        resolve(null);
      }
    });
    child.on("error", () => resolve(null));
  });
}

export { fsConstants };
