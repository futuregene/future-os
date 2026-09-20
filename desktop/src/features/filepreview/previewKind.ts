import { pathExtension } from "../../lib/workspacePath";

/**
 * File-type detection for the local-file preview overlay. Detection is purely by
 * extension — the same signal `FileLink` has (a `StoredFile` never touches the
 * filesystem). Only these kinds get an in-app preview; every other file (PDFs
 * included) keeps the "open with the OS default handler" behavior.
 */
export type PreviewKind = "image" | "json" | "markdown" | "text";

const IMAGE_RE = /\.(?:avif|bmp|gif|jpe?g|png|svg|webp)$/i;
const MARKDOWN_RE = /\.(?:md|markdown)$/i;
const JSON_RE = /\.json$/i;

/**
 * Code / config / script files read as plain monospace text. Deliberately a
 * separate list from `lib/fileType`'s `CODE_RE`: that one is a *glyph* signal
 * (which icon a row shows) and classifies some types this overlay can't read,
 * while this is the *capability* signal. It is also wider than the phone's
 * text routes (`mobile/src/remote/fileTypes.ts`) — the overlay has no transfer
 * budget and reads whatever the OS can hand it.
 */
const TEXT_RE
  = /\.(?:[cfhmr]|asm|bash|bat|cc|cfg|clj|cljs|cmd|conf|cpp|cs|csh|css|csv|cxx|dart|diff|edn|el|elm|env|erl|ex|exs|f90|f95|fish|go|gradle|graphql|groovy|hh|hpp|hs|htm|html|hxx|ini|java|jl|js|jsonl|jsx|kt|kts|less|lisp|lua|mm|ndjson|nim|pas|patch|php|pl|pm|properties|ps1|py|pyi|rb|rs|sass|scala|scm|scss|sh|sol|sql|svelte|swift|tf|toml|ts|tsv|tsx|vb|vue|xhtml|xml|yaml|yml|zig|zsh)$/i;

export function previewKindForPath(path: string): PreviewKind | null {
  if (IMAGE_RE.test(path))
    return "image";
  if (MARKDOWN_RE.test(path))
    return "markdown";
  if (JSON_RE.test(path))
    return "json";
  if (TEXT_RE.test(path))
    return "text";
  return null;
}

/**
 * True for paths whose bytes the text-preview backend command can render —
 * plain code / config text, plus the Markdown and JSON files that have a richer
 * preview kind of their own. Callers that just want "is this readable text"
 * (the artifact detail's file-backed preview) use this instead of switching on
 * {@link PreviewKind}.
 */
export function isTextReadablePath(path: string): boolean {
  return MARKDOWN_RE.test(path) || JSON_RE.test(path) || TEXT_RE.test(path);
}

const IMAGE_MIME: Record<string, string> = {
  avif: "image/avif",
  bmp: "image/bmp",
  gif: "image/gif",
  jpeg: "image/jpeg",
  jpg: "image/jpeg",
  png: "image/png",
  svg: "image/svg+xml",
  webp: "image/webp",
};

/** MIME type for a data-URL `<img src>`, keyed off the extension. */
export function imageMimeForPath(path: string): string {
  return IMAGE_MIME[pathExtension(path)] ?? "application/octet-stream";
}
