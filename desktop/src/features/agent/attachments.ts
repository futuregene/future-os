import i18n from "../../i18n";
import { inspectAttachment, validateImageAttachment } from "../../integrations/storage/files";
import { formatBytes } from "../../lib/format";
import { pathBasename } from "../../lib/workspacePath";

/**
 * Images are size- and count-limited (vision models have token/dimension caps);
 * every other file type is unlimited — the agent reads local paths on demand
 * with its own tools, so we neither restrict nor pre-process them.
 */
export const MAX_IMAGES_PER_TURN = 4;

// SVG is intentionally excluded: it's text (XML), not a raster image the vision
// pipeline can decode/downscale. It's treated as a normal file so the model
// reads its source with its tools.
const IMAGE_EXTENSIONS = ["jpg", "jpeg", "png", "gif", "webp", "bmp"] as const;

/** Per-image byte cap. Non-image files carry no size limit. */
export const READ_SOURCE_MAX_BYTES = 25 * 1024 * 1024;

type AttachmentKind = "image" | "file";

const IMAGE_MIME_EXTENSIONS: Record<string, string> = {
  "image/bmp": "bmp",
  "image/gif": "gif",
  "image/jpeg": "jpg",
  "image/png": "png",
  "image/webp": "webp",
};

export function imageExtensionFromMime(mime: string) {
  return IMAGE_MIME_EXTENSIONS[mime.toLowerCase()] ?? null;
}

export function fileNameFromPath(path: string) {
  return pathBasename(path) || path;
}

/**
 * Split a filename into its stem and extension (".png", dot included) so the UI
 * can middle-truncate the stem while keeping the suffix visible. A dotfile or a
 * name without an extension returns the whole name as the stem.
 */
export function splitFileName(name: string): { stem: string; ext: string } {
  const dot = name.lastIndexOf(".");
  if (dot <= 0)
    return { stem: name, ext: "" };
  return { stem: name.slice(0, dot), ext: name.slice(dot) };
}

export function extOf(path: string) {
  const name = fileNameFromPath(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

export function isImageExtension(path: string) {
  return IMAGE_EXTENSIONS.includes(extOf(path) as (typeof IMAGE_EXTENSIONS)[number]);
}

/**
 * Classify a local attachment by path into `image` | `file`. Only the directory
 * check needs Rust (`inspect_attachment`) — the webview can't stat arbitrary
 * paths. Non-image files are never restricted by type or content: the agent
 * reads them with its own tools.
 */
export async function classifyAttachment(
  path: string,
): Promise<{ kind: AttachmentKind } | { kind: null; reason: string }> {
  let info: { isDir: boolean; size: number } | null = null;
  try {
    info = await inspectAttachment(path);
  }
  catch {
    return { kind: null, reason: i18n.t("agent:attachment.readFailed") };
  }
  if (info.isDir)
    return { kind: null, reason: i18n.t("agent:attachment.directoryUnsupported") };
  if (isImageExtension(path) && info.size > READ_SOURCE_MAX_BYTES) {
    return {
      kind: null,
      reason: i18n.t("agent:attachment.imageTooLarge", { max: formatBytes(READ_SOURCE_MAX_BYTES) }),
    };
  }
  if (isImageExtension(path)) {
    try {
      await validateImageAttachment(path);
    }
    catch {
      return {
        kind: null,
        reason: i18n.t("agent:attachment.imageUnreadable", { name: fileNameFromPath(path) }),
      };
    }
  }
  return { kind: isImageExtension(path) ? "image" : "file" };
}
