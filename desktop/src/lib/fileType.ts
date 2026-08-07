/**
 * File-type classification by path/extension — the single source of truth for
 * "what category is this file" across the app (artifact badges, the file-tree
 * node icons). Pure and dependency-free so it stays unit-testable and usable
 * from `lib/`; the kind→icon mapping lives separately in
 * `components/ui/FileTypeIcon`.
 *
 * This is a *category* signal (which glyph to show), distinct from
 * `filepreview/previewKind`'s *capability* signal (can we preview it in-app).
 * They intentionally stay separate concerns.
 */
export type FileKind
  = | "folder"
    | "image"
    | "pdf"
    | "markdown"
    | "html"
    | "archive"
    | "shell"
    | "code"
    | "text";

const IMAGE_RE = /\.(?:avif|bmp|gif|ico|jpe?g|png|svg|tiff?|webp)$/i;
const PDF_RE = /\.pdf$/i;
const MARKDOWN_RE = /\.(?:markdown|md)$/i;
const HTML_RE = /\.(?:htm|html|xhtml)$/i;
const ARCHIVE_RE = /\.(?:7z|bz2|gz|rar|tar|tgz|xz|zip)$/i;
const SHELL_RE = /\.(?:bash|bat|cmd|fish|ps1|sh|zsh)$/i;
const CODE_RE = /\.(?:c|cc|cpp|cs|css|go|h|hpp|java|js|json|jsonl|jsx|kt|php|py|rb|rs|scss|swift|toml|ts|tsx|xml|ya?ml)$/i;

/**
 * Classify a path into a {@link FileKind}. Ordered most-specific first: PDF /
 * Markdown / HTML / archive / shell are matched before the generic code / text
 * fallbacks so each gets its own glyph. `isDir` short-circuits to `folder`.
 */
export function fileKind(path: string, isDir = false): FileKind {
  if (isDir)
    return "folder";
  if (IMAGE_RE.test(path))
    return "image";
  if (PDF_RE.test(path))
    return "pdf";
  if (MARKDOWN_RE.test(path))
    return "markdown";
  if (HTML_RE.test(path))
    return "html";
  if (ARCHIVE_RE.test(path))
    return "archive";
  if (SHELL_RE.test(path))
    return "shell";
  if (CODE_RE.test(path))
    return "code";
  return "text";
}
