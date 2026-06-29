import { constants as fsConstants } from "node:fs";
import { access } from "node:fs/promises";
import { dirname, join } from "node:path";

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
 * Directory of the currently running executable.
 *
 * For a `bun build --compile` single-file binary, `process.execPath` is the real
 * path of that binary (symlinks resolved, cwd-independent), so its directory is
 * where packaged builds ship co-located siblings (future-agent, future-tui)
 * inside the .app / portable folder. When running via Node (dev / npm-link),
 * this is Node's own directory, which has no such siblings — callers then fall
 * back to repo-relative resolution.
 */
export function selfDir(): string {
  return dirname(process.execPath);
}

/**
 * Resolve a sibling binary co-located next to the running executable, or null if
 * it is absent / not executable. `name` is the base name without extension; a
 * `.exe` suffix is added on Windows.
 */
export async function colocatedBinary(name: string): Promise<string | null> {
  const exe = process.platform === "win32" ? `${name}.exe` : name;
  const candidate = join(selfDir(), exe);
  return (await canAccess(candidate, fsConstants.X_OK)) ? candidate : null;
}

export { fsConstants };
