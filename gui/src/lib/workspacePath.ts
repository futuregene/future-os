import { isWindows } from "./platform";

/**
 * A file path inside the active workspace renders relative to its root; a path
 * outside the workspace keeps its absolute form so it stays unambiguous.
 * Returns the input unchanged when there's no workspace root or the path isn't
 * under it. Callers must not pass shell commands here — only file paths.
 *
 * Handles both `/` and `\` separators so Windows paths relativize too, and
 * compares case-insensitively on Windows (its paths are case-insensitive).
 */
export function relativizeWorkspacePath(path: string, workspacePath?: string | null): string {
  if (!workspacePath)
    return path;

  // Strip a trailing separator of either kind.
  const root = workspacePath.replace(/[/\\]+$/, "");
  if (samePath(path, root))
    return path;
  // Inside the workspace = starts with `root` followed by a separator.
  const separator = path[root.length];
  if ((separator === "/" || separator === "\\") && samePath(path.slice(0, root.length), root))
    return path.slice(root.length + 1);
  return path;
}

function samePath(a: string, b: string): boolean {
  return isWindows ? a.toLowerCase() === b.toLowerCase() : a === b;
}

/**
 * Last path segment, splitting on both `/` and `\` so Windows paths work too.
 * Returns "" when the path has no segment; callers supply their own fallback.
 */
export function pathBasename(path: string): string {
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] ?? "";
}
