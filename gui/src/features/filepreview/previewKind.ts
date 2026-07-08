/**
 * File-type detection for the local-file preview overlay. Detection is purely by
 * extension — the same signal `FileLink` has (a `StoredFile` never touches the
 * filesystem). Only these kinds get an in-app preview; every other file (PDFs
 * included) keeps the "open with the OS default handler" behavior.
 */
export type PreviewKind = "image" | "markdown";

const IMAGE_RE = /\.(?:avif|bmp|gif|jpe?g|png|svg|webp)$/i;
const MARKDOWN_RE = /\.(?:md|markdown)$/i;

export function previewKindForPath(path: string): PreviewKind | null {
  if (IMAGE_RE.test(path))
    return "image";
  if (MARKDOWN_RE.test(path))
    return "markdown";
  return null;
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
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return IMAGE_MIME[ext] ?? "application/octet-stream";
}
