/**
 * A file path inside the active workspace renders relative to its root; a path
 * outside the workspace keeps its absolute form so it stays unambiguous.
 * Returns the input unchanged when there's no workspace root or the path isn't
 * under it. Callers must not pass shell commands here — only file paths.
 */
export function relativizeWorkspacePath(path: string, workspacePath?: string | null): string {
  if (!workspacePath)
    return path;

  const root = workspacePath.replace(/\/+$/, "");
  if (path === root)
    return path;
  if (path.startsWith(`${root}/`))
    return path.slice(root.length + 1);
  return path;
}
