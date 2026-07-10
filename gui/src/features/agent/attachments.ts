import i18n from "../../i18n";
import { inspectAttachment } from "../../integrations/storage/files";
import { pathBasename } from "../../lib/workspacePath";

export const MAX_ATTACHMENTS_PER_TURN = 4;

const IMAGE_EXTENSIONS = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"] as const;

/** Text/source extensions accepted for inline extraction (must also pass the Rust binary sniff). */
const TEXT_EXTENSIONS = new Set([
  "txt",
  "md",
  "markdown",
  "json",
  "jsonl",
  "yaml",
  "yml",
  "toml",
  "xml",
  "csv",
  "tsv",
  "ini",
  "cfg",
  "conf",
  "log",
  "sql",
  "sh",
  "bash",
  "zsh",
  "py",
  "rs",
  "ts",
  "tsx",
  "js",
  "jsx",
  "mjs",
  "cjs",
  "go",
  "java",
  "c",
  "h",
  "cpp",
  "hpp",
  "cc",
  "cs",
  "rb",
  "php",
  "swift",
  "kt",
  "scala",
  "r",
  "lua",
  "pl",
  "dart",
  "vue",
  "svelte",
  "css",
  "scss",
  "less",
  "html",
  "htm",
]);

/** Extension-less filenames treated as text. */
const TEXT_BASENAMES = new Set(["Dockerfile", "Makefile"]);

/** Extensions offered in the file picker (images + pdf + text/source). */
export const PICKER_EXTENSIONS = [...IMAGE_EXTENSIONS, "pdf", ...TEXT_EXTENSIONS] as string[];

/**
 * File-picker extensions for a given model: images are dropped when the active
 * model can't accept image input, so the picker won't even offer them.
 */
export function pickerExtensions(allowImages: boolean): string[] {
  return allowImages ? PICKER_EXTENSIONS : ["pdf", ...TEXT_EXTENSIONS];
}

/**
 * Fast, path-only check used during drag-over to decide accept vs. reject
 * before the file is dropped — the OS drag flow only hands us paths, not
 * content. Extension-based only (mirrors the picker filter, images gated by
 * `allowImages`); a text-extension file that later proves binary is still
 * caught by `classifyAttachment` on drop.
 */
export function isDraggableAttachment(path: string, allowImages: boolean): boolean {
  const ext = extOf(path);
  if (ext === "pdf" || TEXT_EXTENSIONS.has(ext) || TEXT_BASENAMES.has(fileNameFromPath(path)))
    return true;
  return allowImages && IMAGE_EXTENSIONS.includes(ext as (typeof IMAGE_EXTENSIONS)[number]);
}

/** Per-file byte cap shared by attachments and artifact uploads. */
export const READ_SOURCE_MAX_BYTES = 25 * 1024 * 1024;

type AttachmentKind = "image" | "pdf" | "text";

const IMAGE_MIME_EXTENSIONS: Record<string, string> = {
  "image/bmp": "bmp",
  "image/gif": "gif",
  "image/jpeg": "jpg",
  "image/png": "png",
  "image/svg+xml": "svg",
  "image/webp": "webp",
};

export function imageExtensionFromMime(mime: string) {
  return IMAGE_MIME_EXTENSIONS[mime.toLowerCase()] ?? null;
}

export function fileNameFromPath(path: string) {
  return pathBasename(path) || path;
}

export function extOf(path: string) {
  const name = fileNameFromPath(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/**
 * Classify a local attachment by path. Directory + binary detection happens in
 * Rust (`inspect_attachment`) since the webview can't read arbitrary paths.
 */
export async function classifyAttachment(
  path: string,
): Promise<{ kind: AttachmentKind } | { kind: null; reason: string }> {
  const ext = extOf(path);
  const base = fileNameFromPath(path);
  let info: { isDir: boolean; size: number; isBinary: boolean } | null = null;
  try {
    info = await inspectAttachment(path);
  }
  catch {
    return { kind: null, reason: i18n.t("agent:attachment.readFailed") };
  }
  if (info.isDir)
    return { kind: null, reason: i18n.t("agent:attachment.directoryUnsupported") };

  if (IMAGE_EXTENSIONS.includes(ext as (typeof IMAGE_EXTENSIONS)[number]))
    return { kind: "image" };
  if (ext === "pdf")
    return { kind: "pdf" };
  if ((TEXT_EXTENSIONS.has(ext) || TEXT_BASENAMES.has(base)) && !info.isBinary)
    return { kind: "text" };
  return { kind: null, reason: info.isBinary ? i18n.t("agent:attachment.binaryUnsupported") : i18n.t("agent:attachment.typeUnsupported") };
}
