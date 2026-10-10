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
//
// `route: "text"` means the phone reads the file in-app as plain text; the
// desktop host serves exactly these suffixes as preview-capable text types
// (`remote_host/files.rs`), so the phone never asks for a preview it would
// refuse.
const MOBILE_FILE_TYPES: Readonly<Record<string, MobileFileType>> = {
  ".tar.gz": { mimeType: "application/gzip", route: "external" },
  ".markdown": { mimeType: "text/markdown", route: "markdown" },
  ".numbers": { mimeType: "application/vnd.apple.numbers", route: "external" },
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
  ".htm": { mimeType: "text/html", route: "external" },
  ".tex": { mimeType: "application/x-tex", route: "external", textFallback: true },
  ".bib": { mimeType: "application/x-bibtex", route: "external", textFallback: true },
  ".ris": {
    mimeType: "application/x-research-info-systems",
    route: "external",
    textFallback: true,
  },
  ".py": { mimeType: "text/x-python", route: "text", textFallback: true },
  ".r": { mimeType: "text/x-r-source", route: "text", textFallback: true },
  ".jl": { mimeType: "text/x-julia", route: "text", textFallback: true },
  ".m": { mimeType: "text/x-matlab", route: "text", textFallback: true },
  ".sql": { mimeType: "application/sql", route: "text", textFallback: true },
  ".sh": { mimeType: "application/x-sh", route: "text", textFallback: true },
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

  // Text data formats read in-app instead of being handed to another app:
  // columns and indentation only line up in the monospace reader. Each keeps
  // its precise MIME + `textFallback` so the preview menu can still push it to
  // an installed editor.
  ".csv": { mimeType: "text/csv", route: "text", textFallback: true },
  ".tsv": { mimeType: "text/tab-separated-values", route: "text", textFallback: true },
  ".yaml": { mimeType: "application/yaml", route: "text", textFallback: true },
  ".yml": { mimeType: "application/yaml", route: "text", textFallback: true },
  ".xml": { mimeType: "application/xml", route: "text", textFallback: true },
  ".jsonl": { mimeType: "application/jsonl", route: "text", textFallback: true },
  ".ndjson": { mimeType: "application/x-ndjson", route: "text", textFallback: true },
  // A notebook is one JSON document, so it gets the rich JSON reader (and the
  // plain-text fallback that reader already has above its size ceiling).
  ".ipynb": { mimeType: "application/json", route: "json", textFallback: true },

  // Code and configuration files read in-app as text. The preview renders them
  // with the grammar for their suffix (`components/codeHighlight.ts`).
  ".asm": { mimeType: "text/plain", route: "text" },
  ".c": { mimeType: "text/plain", route: "text" },
  ".cc": { mimeType: "text/plain", route: "text" },
  ".clj": { mimeType: "text/plain", route: "text" },
  ".cljs": { mimeType: "text/plain", route: "text" },
  ".cpp": { mimeType: "text/plain", route: "text" },
  ".cs": { mimeType: "text/plain", route: "text" },
  ".cxx": { mimeType: "text/plain", route: "text" },
  ".dart": { mimeType: "text/plain", route: "text" },
  ".el": { mimeType: "text/plain", route: "text" },
  ".erl": { mimeType: "text/plain", route: "text" },
  ".ex": { mimeType: "text/plain", route: "text" },
  ".exs": { mimeType: "text/plain", route: "text" },
  ".f90": { mimeType: "text/plain", route: "text" },
  ".f95": { mimeType: "text/plain", route: "text" },
  ".go": { mimeType: "text/plain", route: "text" },
  ".groovy": { mimeType: "text/plain", route: "text" },
  ".h": { mimeType: "text/plain", route: "text" },
  ".hh": { mimeType: "text/plain", route: "text" },
  ".hpp": { mimeType: "text/plain", route: "text" },
  ".hs": { mimeType: "text/plain", route: "text" },
  ".hxx": { mimeType: "text/plain", route: "text" },
  ".java": { mimeType: "text/plain", route: "text" },
  ".kt": { mimeType: "text/plain", route: "text" },
  ".kts": { mimeType: "text/plain", route: "text" },
  ".lisp": { mimeType: "text/plain", route: "text" },
  ".lua": { mimeType: "text/plain", route: "text" },
  ".nim": { mimeType: "text/plain", route: "text" },
  ".pas": { mimeType: "text/plain", route: "text" },
  ".php": { mimeType: "text/plain", route: "text" },
  ".pl": { mimeType: "text/plain", route: "text" },
  ".pm": { mimeType: "text/plain", route: "text" },
  ".pyi": { mimeType: "text/plain", route: "text" },
  ".rb": { mimeType: "text/plain", route: "text" },
  ".rs": { mimeType: "text/plain", route: "text" },
  ".scala": { mimeType: "text/plain", route: "text" },
  ".scm": { mimeType: "text/plain", route: "text" },
  ".sol": { mimeType: "text/plain", route: "text" },
  ".swift": { mimeType: "text/plain", route: "text" },
  ".vb": { mimeType: "text/plain", route: "text" },
  ".zig": { mimeType: "text/plain", route: "text" },
  ".js": { mimeType: "text/plain", route: "text" },
  ".jsx": { mimeType: "text/plain", route: "text" },
  ".mjs": { mimeType: "text/plain", route: "text" },
  ".cjs": { mimeType: "text/plain", route: "text" },
  ".ts": { mimeType: "text/plain", route: "text" },
  ".tsx": { mimeType: "text/plain", route: "text" },
  ".vue": { mimeType: "text/plain", route: "text" },
  ".svelte": { mimeType: "text/plain", route: "text" },
  ".css": { mimeType: "text/plain", route: "text" },
  ".less": { mimeType: "text/plain", route: "text" },
  ".sass": { mimeType: "text/plain", route: "text" },
  ".scss": { mimeType: "text/plain", route: "text" },
  ".bash": { mimeType: "text/plain", route: "text" },
  ".bat": { mimeType: "text/plain", route: "text" },
  ".cmd": { mimeType: "text/plain", route: "text" },
  ".csh": { mimeType: "text/plain", route: "text" },
  ".fish": { mimeType: "text/plain", route: "text" },
  ".ps1": { mimeType: "text/plain", route: "text" },
  ".zsh": { mimeType: "text/plain", route: "text" },
  ".cfg": { mimeType: "text/plain", route: "text" },
  ".conf": { mimeType: "text/plain", route: "text" },
  ".env": { mimeType: "text/plain", route: "text" },
  ".gradle": { mimeType: "text/plain", route: "text" },
  ".ini": { mimeType: "text/plain", route: "text" },
  ".properties": { mimeType: "text/plain", route: "text" },
  ".tf": { mimeType: "text/plain", route: "text" },
  ".toml": { mimeType: "text/plain", route: "text" },
  ".mk": { mimeType: "text/plain", route: "text" },
  ".cmake": { mimeType: "text/plain", route: "text" },
  ".lock": { mimeType: "text/plain", route: "text" },
  ".plist": { mimeType: "text/plain", route: "text" },
  ".xcconfig": { mimeType: "text/plain", route: "text" },
  ".csproj": { mimeType: "text/plain", route: "text" },
  ".sln": { mimeType: "text/plain", route: "text" },
  ".proto": { mimeType: "text/plain", route: "text" },
  ".diff": { mimeType: "text/plain", route: "text" },
  ".patch": { mimeType: "text/plain", route: "text" },
  ".graphql": { mimeType: "text/plain", route: "text" },
  ".edn": { mimeType: "text/plain", route: "text" },
  ".elm": { mimeType: "text/plain", route: "text" },
  ".xhtml": { mimeType: "text/plain", route: "text" },
  ".mm": { mimeType: "text/plain", route: "text" },
  ".f": { mimeType: "text/plain", route: "text" },
  ".hcl": { mimeType: "text/plain", route: "text" },
  ".tfvars": { mimeType: "text/plain", route: "text" },
  ".mts": { mimeType: "text/plain", route: "text" },
  ".cts": { mimeType: "text/plain", route: "text" },
};

/** Shared entry for the suffix-less names below. */
const PLAIN_TEXT: MobileFileType = { mimeType: "text/plain", route: "text" };

// Suffix-less files are the norm for build entry points, lockfiles and
// editor / shell / version-manager dotfiles, so a suffix table can never see
// them (`Makefile`, `Dockerfile`, `.gitignore`, `Cargo.lock`). They match on the
// whole base name instead. The desktop host (`remote_host/files.rs`) carries the
// same names, and the desktop overlay's own list (`previewKind.ts`) is wider
// because it has no transfer budget.
const MOBILE_FILE_NAMES: Readonly<Record<string, MobileFileType>> = {
  // Build and project entry points.
  "brewfile": PLAIN_TEXT,
  "build.bazel": PLAIN_TEXT,
  "caddyfile": PLAIN_TEXT,
  "gemfile": PLAIN_TEXT,
  "justfile": PLAIN_TEXT,
  "meson.build": PLAIN_TEXT,
  "podfile": PLAIN_TEXT,
  "procfile": PLAIN_TEXT,
  "rakefile": PLAIN_TEXT,
  "vagrantfile": PLAIN_TEXT,
  "workspace": PLAIN_TEXT,
  // Dependency manifests (`.lock` files are covered by their suffix).
  "go.mod": PLAIN_TEXT,
  "go.sum": PLAIN_TEXT,
  // Editor, shell and version-manager dotfiles.
  ".bazelrc": PLAIN_TEXT,
  ".babelrc": PLAIN_TEXT,
  ".bashrc": PLAIN_TEXT,
  ".condarc": PLAIN_TEXT,
  ".dockerignore": PLAIN_TEXT,
  ".editorconfig": PLAIN_TEXT,
  ".envrc": PLAIN_TEXT,
  ".eslintrc": PLAIN_TEXT,
  ".gitattributes": PLAIN_TEXT,
  ".gitignore": PLAIN_TEXT,
  ".gitmodules": PLAIN_TEXT,
  ".npmrc": PLAIN_TEXT,
  ".nvmrc": PLAIN_TEXT,
  ".prettierrc": PLAIN_TEXT,
  ".profile": PLAIN_TEXT,
  ".rprofile": PLAIN_TEXT,
  ".vimrc": PLAIN_TEXT,
  ".yamllint": PLAIN_TEXT,
  ".zshrc": PLAIN_TEXT,
  // Licence and attribution files that ship without an extension; the `.md` /
  // `.txt` versions are already covered by their suffix.
  "authors": PLAIN_TEXT,
  "changelog": PLAIN_TEXT,
  "codeowners": PLAIN_TEXT,
  "license": PLAIN_TEXT,
  "notice": PLAIN_TEXT,
  "readme": PLAIN_TEXT,
};

// Names that carry a qualifier of their own (`Dockerfile.dev`, `Makefile.am`,
// `.env.local`): the prefix alone, or the prefix followed by `.`.
const MOBILE_FILE_NAME_PREFIXES: readonly string[] = [
  "makefile",
  "gnumakefile",
  "dockerfile",
  "containerfile",
  "jenkinsfile",
  ".env",
];

const SUFFIXES = Object.keys(MOBILE_FILE_TYPES).sort((left, right) => right.length - left.length);

/** A path separator on either platform: the name a file was listed under may be
 * a full desktop path, and only its last segment decides the type. */
const PATH_SEPARATORS = /[\\/]/;

function baseName(normalized: string): string {
  return normalized.split(PATH_SEPARATORS).pop() ?? normalized;
}

export function mobileFileType(name: string): MobileFileType | null {
  const normalized = baseName(name.trim().toLowerCase());
  const named = MOBILE_FILE_NAMES[normalized];
  if (named) return named;
  if (MOBILE_FILE_NAME_PREFIXES.some(prefix =>
    normalized === prefix || normalized.startsWith(`${prefix}.`))) return PLAIN_TEXT;
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
  // Previewable files can also be explicitly opened in another application.
  if (!type) return [];
  return type.textFallback && type.mimeType !== "text/plain"
    ? [type.mimeType, "text/plain"]
    : [type.mimeType];
}
