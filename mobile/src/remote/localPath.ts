/**
 * Classify a markdown link destination as a local filesystem path.
 *
 * Returns the normalized path when the href names a local file, or `null` when
 * it doesn't (the caller treats `null` as a remote/other link and opens it in
 * the browser). Pure string logic — no filesystem access — and cross-platform.
 * Ported from `desktop/src/features/markdown/localPath.ts`.
 *
 * Recognized as local:
 *  - `file://` URIs                       → `file:///Users/x`  → `/Users/x`
 *  - POSIX absolute                       → `/Users/x`
 *  - Windows drive absolute               → `C:/x`, `C:\x`
 *  - Windows UNC                          → `\\server\share`
 *  - Explicit relative                    → `./x`, `../x` (and backslash forms)
 *  - Bare relative that clearly names a file (models often drop the `./`):
 *      · has a path separator, first segment not a domain → `docs/readme.md`
 *      · single token with a known file extension         → `长诗.md`, `main.rs`
 *
 * NOT local (→ `null`): any other URL scheme (`http:`, `https:`, `mailto:`,
 * `futureos:`, …) and bare tokens that look like a web host (`example.com`,
 * `github.com/user/repo`) or carry no file-ish signal.
 */
export function localFilePath(href: string): string | null {
  const raw = href.trim();
  if (!raw) return null;

  // `file://` URI — decode to its plain path.
  if (/^file:\/\//i.test(raw)) {
    try {
      const decoded = decodeURIComponent(new URL(raw).pathname);
      return decoded || null;
    } catch {
      return null;
    }
  }

  // Any other explicit URL scheme (http:, https:, mailto:, futureos:, …) is not
  // a local path. The two-plus char requirement keeps a Windows drive letter
  // (`C:`) from being mistaken for a scheme so it falls through to the drive
  // check below.
  if (/^[a-z][a-z0-9+.-]+:/i.test(raw)) return null;

  // POSIX absolute.
  if (raw.startsWith("/")) return raw;

  // Windows UNC (`\\server\share`).
  if (raw.startsWith("\\\\")) return raw;

  // Windows drive absolute (`C:\` or `C:/`).
  if (/^[a-z]:[\\/]/i.test(raw)) return raw;

  // Explicit relative (`./x`, `../x`, or backslash forms). Strip a single
  // leading `./` for a cleaner path; `../` is preserved.
  if (/^\.\.?[\\/]/.test(raw)) return raw.replace(/^\.\//, "");

  // Bare relative path without a `./` prefix — models frequently drop it. These
  // overlap with scheme-less domains, so accept only clear-file shapes.
  const separator = raw.search(/[\\/]/);
  if (separator >= 0) {
    // Has a path separator: treat as a relative path unless the first segment
    // is a bare domain (`example.com/page`, `github.com/user/repo`).
    if (looksLikeDomain(raw.slice(0, separator))) return null;
    return raw;
  }

  // Single token, no separator: accept only when it carries a known file
  // extension (`长诗.md`, `config.json`); a bare `example.com` stays remote.
  if (hasKnownFileExtension(raw)) return raw;

  return null;
}

/**
 * `host.tld` / `sub.host.tld` (optional port). The final label must be an
 * alphabetic TLD of 2+ chars so a path segment like `a.b` isn't mistaken for a
 * domain.
 */
function looksLikeDomain(segment: string): boolean {
  return /^[a-z0-9-]+(?:\.[a-z0-9-]+)*\.[a-z]{2,}(?::\d+)?$/i.test(segment);
}

/**
 * Common source/doc/data/media extensions the assistant actually emits. An
 * allowlist (rather than a TLD denylist) keeps a bare `example.com` from being
 * read as a file — a missed file link is harmless, a domain opened as a path
 * is not. Extend as needed.
 */
const FILE_EXTENSIONS = new Set(
  (
    "md markdown mdx txt text rst adoc org tex " +
    "rs ts tsx js jsx mjs cjs py pyi go java kt kts scala c h cc cpp cxx hpp hh cs rb " +
    "php swift mm sh bash zsh fish ps1 bat lua pl pm jl dart ex exs erl hs elm clj cljs sql vim " +
    "html htm css scss sass less vue svelte astro " +
    "json json5 jsonc yaml yml toml ini cfg conf env xml csv tsv properties proto graphql prisma " +
    "png jpg jpeg gif svg webp ico bmp tiff pdf " +
    "lock log gitignore dockerignore mk gradle"
  ).split(" "),
);

function hasKnownFileExtension(token: string): boolean {
  const ext = token.match(/\.([a-z0-9]+)$/i)?.[1];
  return ext ? FILE_EXTENSIONS.has(ext.toLowerCase()) : false;
}

/** Last path segment, handling both POSIX `/` and Windows `\` separators. */
export function basename(path: string): string {
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] ?? path;
}
