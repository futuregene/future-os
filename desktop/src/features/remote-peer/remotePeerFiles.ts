/**
 * Pure helpers for browsing another machine's files.
 *
 * Their own module, not part of the dialog: the path arithmetic is the part
 * that can be wrong in a way the UI hides (a wrong "up" walks the user out of
 * the session, where every read is refused), so it is unit-tested directly, and
 * a module that exports it beside a component breaks fast refresh.
 */

/** Bytes as a person reads them. */
export function formatSize(bytes: number): string {
  if (bytes < 1024)
    return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/**
 * The directory one level above `path`, stopping at `root`.
 *
 * Textual on purpose: the host's paths may use `\` or `/`, so the separator is
 * whatever the path itself used and the only boundary that matters is the
 * session root the host reported. `null` means the root is already showing —
 * there is nothing above it that the host would agree to read.
 */
export function parentOf(path: string, root: string): string | null {
  if (!path || path === root)
    return null;
  const trimmed = path.replace(/[/\\]+$/, "");
  const separator = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (separator <= 0)
    return null;
  const parent = trimmed.slice(0, separator);
  // Never above the session root: a request outside it is refused by the host,
  // so offering it would be offering a button that cannot work.
  if (root && parent.length < root.length && root.startsWith(parent))
    return root;
  return parent;
}
