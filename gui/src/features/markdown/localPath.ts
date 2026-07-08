/**
 * Classify a markdown link destination as a local filesystem path.
 *
 * Returns the normalized path when the href names a local file, or `null` when
 * it doesn't (the caller treats `null` as a remote/other link and hands it to
 * `SafeLink`). Pure string logic — no filesystem access — and cross-platform.
 *
 * Recognized as local:
 *  - `file://` URIs                       → `file:///Users/x`  → `/Users/x`
 *  - POSIX absolute                       → `/Users/x`
 *  - Windows drive absolute               → `C:/x`, `C:\x`
 *  - Windows UNC                          → `\\server\share`
 *  - Explicit relative                    → `./x`, `../x` (and backslash forms)
 *
 * NOT local (→ `null`): any other URL scheme (`http:`, `https:`, `mailto:`,
 * `futureos:`, …) and bare scheme-less tokens without an explicit `./`/`../`
 * prefix — those are ambiguous with bare domains (`example.com/page`), so we
 * leave them to `SafeLink`.
 *
 * The model writes the path from the write-tool result verbatim (wrapped in
 * angle brackets when it contains spaces, which remark strips before we see
 * it), so there is no percent-encoding step and thus no dropped-leading-slash
 * failure mode.
 */
export function localFilePath(href: string): string | null {
  const raw = href.trim();
  if (!raw)
    return null;

  // `file://` URI — decode to its plain path.
  if (/^file:\/\//i.test(raw)) {
    try {
      const decoded = decodeURIComponent(new URL(raw).pathname);
      return decoded || null;
    }
    catch {
      return null;
    }
  }

  // Any other explicit URL scheme (http:, https:, mailto:, futureos:, …) is not
  // a local path. The two-plus char requirement keeps a Windows drive letter
  // (`C:`) from being mistaken for a scheme so it falls through to the drive
  // check below.
  if (/^[a-z][a-z0-9+.-]+:/i.test(raw))
    return null;

  // POSIX absolute.
  if (raw.startsWith("/"))
    return raw;

  // Windows UNC (`\\server\share`).
  if (raw.startsWith("\\\\"))
    return raw;

  // Windows drive absolute (`C:\` or `C:/`).
  if (/^[a-z]:[\\/]/i.test(raw))
    return raw;

  // Explicit relative (`./x`, `../x`, or backslash forms). Strip a single
  // leading `./` for a cleaner path; `../` is preserved.
  if (/^\.\.?[\\/]/.test(raw))
    return raw.replace(/^\.\//, "");

  return null;
}
