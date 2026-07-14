/**
 * Screenshot writer — protocol-agnostic path resolution and file writing.
 *
 * Extracted from the current browserScreenshot() logic.
 * Does NOT import any CDP or WebDriver types.
 */
import { mkdir, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { homedir } from "node:os";

const FUTURE_HOME = process.env["FUTURE_HOME"] ?? join(homedir(), ".future");
const BROWSER_DIR = join(FUTURE_HOME, "agent", "browser");
const ARTIFACTS_DIR = join(BROWSER_DIR, "artifacts");

export interface ScreenshotWriteResult {
  path: string;
  filename: string;
}

/**
 * Resolve the screenshot output path.
 * If explicitPath is provided, use it. Otherwise generate a timestamped
 * path in the artifacts directory.
 */
export function resolveScreenshotPath(explicitPath?: string): string {
  if (explicitPath) return explicitPath;
  const ts = new Date().toISOString().replace(/[:.]/g, "-");
  return join(ARTIFACTS_DIR, `browser-${ts}.png`);
}

/**
 * Write screenshot bytes to the resolved path.
 * Handles missing parent directories with a retry to the artifacts dir.
 */
export async function writeScreenshot(
  bytes: Uint8Array,
  resolvedPath: string,
): Promise<ScreenshotWriteResult> {
  try {
    await mkdir(dirname(resolvedPath), { recursive: true });
    await writeFile(resolvedPath, bytes);
    return { path: resolvedPath, filename: basename(resolvedPath) };
  } catch {
    // Fallback: write to artifacts dir
    await mkdir(ARTIFACTS_DIR, { recursive: true });
    const fallbackPath = join(ARTIFACTS_DIR, basename(resolvedPath));
    await writeFile(fallbackPath, bytes);
    return { path: fallbackPath, filename: basename(fallbackPath) };
  }
}
