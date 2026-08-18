export type MobileFileRoute = "image" | "markdown" | "text" | "json" | "external";

export const MAX_JSON_RICH_PREVIEW_BYTES = 1024 * 1024;

interface MobileFileType {
  mimeType: string;
  route: MobileFileRoute;
  /** Text-like external formats may fall back to a generic editor/viewer. */
  textFallback?: boolean;
}

// Business allow-list for desktop-to-phone downloads. Keep compound suffixes
// as keys and resolve them before the final single extension (for example,
// archive.tar.gz must not be treated as an arbitrary .gz file).
const MOBILE_FILE_TYPES: Readonly<Record<string, MobileFileType>> = {
  ".tar.gz": { mimeType: "application/gzip", route: "external" },
  ".markdown": { mimeType: "text/markdown", route: "markdown" },
  ".numbers": { mimeType: "application/vnd.apple.numbers", route: "external" },
  ".jsonl": { mimeType: "application/jsonl", route: "external", textFallback: true },
  ".pages": { mimeType: "application/vnd.apple.pages", route: "external" },
  ".docx": {
    mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    route: "external",
  },
  ".xlsx": {
    mimeType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    route: "external",
  },
  ".pptx": {
    mimeType: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    route: "external",
  },
  ".tiff": { mimeType: "image/tiff", route: "external" },
  ".heic": { mimeType: "image/heic", route: "external" },
  ".heif": { mimeType: "image/heif", route: "external" },
  ".html": { mimeType: "text/html", route: "external" },
  ".yaml": { mimeType: "application/yaml", route: "external", textFallback: true },
  ".gzip": { mimeType: "application/gzip", route: "external" },
  ".epub": { mimeType: "application/epub+zip", route: "external" },
  ".webp": { mimeType: "image/webp", route: "image" },
  ".jpeg": { mimeType: "image/jpeg", route: "image" },
  ".flac": { mimeType: "audio/flac", route: "external" },
  ".webm": { mimeType: "video/webm", route: "external" },
  ".json": { mimeType: "application/json", route: "json" },
  ".doc": { mimeType: "application/msword", route: "external" },
  ".xls": { mimeType: "application/vnd.ms-excel", route: "external" },
  ".ppt": { mimeType: "application/vnd.ms-powerpoint", route: "external" },
  ".pdf": { mimeType: "application/pdf", route: "external" },
  ".rtf": { mimeType: "application/rtf", route: "external" },
  ".txt": { mimeType: "text/plain", route: "text" },
  ".log": { mimeType: "text/plain", route: "text" },
  ".md": { mimeType: "text/markdown", route: "markdown" },
  ".csv": { mimeType: "text/csv", route: "external", textFallback: true },
  ".tsv": { mimeType: "text/tab-separated-values", route: "external", textFallback: true },
  ".yml": { mimeType: "application/yaml", route: "external", textFallback: true },
  ".xml": { mimeType: "application/xml", route: "external", textFallback: true },
  ".htm": { mimeType: "text/html", route: "external" },
  ".tex": { mimeType: "application/x-tex", route: "external", textFallback: true },
  ".bib": { mimeType: "application/x-bibtex", route: "external", textFallback: true },
  ".ris": {
    mimeType: "application/x-research-info-systems",
    route: "external",
    textFallback: true,
  },
  ".py": { mimeType: "text/x-python", route: "external", textFallback: true },
  ".r": { mimeType: "text/x-r-source", route: "external", textFallback: true },
  ".jl": { mimeType: "text/x-julia", route: "external", textFallback: true },
  ".m": { mimeType: "text/x-matlab", route: "external", textFallback: true },
  ".sql": { mimeType: "application/sql", route: "external", textFallback: true },
  ".sh": { mimeType: "application/x-sh", route: "external", textFallback: true },
  ".jpg": { mimeType: "image/jpeg", route: "image" },
  ".png": { mimeType: "image/png", route: "image" },
  ".gif": { mimeType: "image/gif", route: "external" },
  ".bmp": { mimeType: "image/bmp", route: "image" },
  ".tif": { mimeType: "image/tiff", route: "external" },
  ".svg": { mimeType: "image/svg+xml", route: "external" },
  ".mp3": { mimeType: "audio/mpeg", route: "external" },
  ".m4a": { mimeType: "audio/mp4", route: "external" },
  ".aac": { mimeType: "audio/aac", route: "external" },
  ".wav": { mimeType: "audio/wav", route: "external" },
  ".ogg": { mimeType: "audio/ogg", route: "external" },
  ".mp4": { mimeType: "video/mp4", route: "external" },
  ".m4v": { mimeType: "video/x-m4v", route: "external" },
  ".mov": { mimeType: "video/quicktime", route: "external" },
  ".key": { mimeType: "application/vnd.apple.keynote", route: "external" },
  ".zip": { mimeType: "application/zip", route: "external" },
  ".rar": { mimeType: "application/vnd.rar", route: "external" },
  ".7z": { mimeType: "application/x-7z-compressed", route: "external" },
  ".tar": { mimeType: "application/x-tar", route: "external" },
  ".tgz": { mimeType: "application/gzip", route: "external" },
  ".gz": { mimeType: "application/gzip", route: "external" },
  ".bz2": { mimeType: "application/x-bzip2", route: "external" },
  ".xz": { mimeType: "application/x-xz", route: "external" },
  ".zst": { mimeType: "application/zstd", route: "external" },
};

const SUFFIXES = Object.keys(MOBILE_FILE_TYPES).sort((left, right) => right.length - left.length);

export function mobileFileType(name: string): MobileFileType | null {
  const normalized = name.trim().toLowerCase();
  const suffix = SUFFIXES.find(candidate => normalized.endsWith(candidate));
  return suffix ? MOBILE_FILE_TYPES[suffix]! : null;
}

export function mobilePreviewRoute(name: string, size: number): MobileFileRoute | null {
  const type = mobileFileType(name);
  if (!type) return null;
  return type.route === "json" && size >= MAX_JSON_RICH_PREVIEW_BYTES ? "text" : type.route;
}

export function externalMimeCandidates(name: string): string[] {
  const type = mobileFileType(name);
  if (!type || type.route !== "external") return [];
  return type.textFallback && type.mimeType !== "text/plain"
    ? [type.mimeType, "text/plain"]
    : [type.mimeType];
}
