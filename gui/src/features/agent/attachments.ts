import type { MessageAttachment } from "./agentThreadTypes";
import * as pdfjs from "pdfjs-dist";
import i18n from "../../i18n";
import { inspectAttachment, readFileBase64, readTextFilePreview, writeThumbnail } from "../../integrations/storage/files";
import { pathBasename } from "../../lib/workspacePath";

pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url,
).toString();

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

const INLINE_MAX_BYTES_PER_FILE = 30 * 1024;
const INLINE_MAX_LINES_PER_FILE = 2000;
const INLINE_MAX_TOTAL_BYTES = 60 * 1024;
/**
 * Upper bound on PDF pages scanned for text. A large scanned PDF yields empty
 * text per page, so the byte cap never trips — without this it would walk every
 * page (thousands) before giving up.
 */
const MAX_PDF_PAGES = 100;
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

const EXT_IMAGE_MIME: Record<string, string> = {
  bmp: "image/bmp",
  gif: "image/gif",
  jpeg: "image/jpeg",
  jpg: "image/jpeg",
  png: "image/png",
  svg: "image/svg+xml",
  webp: "image/webp",
};

export function imageExtensionFromMime(mime: string) {
  return IMAGE_MIME_EXTENSIONS[mime.toLowerCase()] ?? null;
}

interface StoredMixedMessage {
  attachments?: MessageAttachment[];
  inlineContext?: string;
  text?: string;
  type?: string;
}

export function fileNameFromPath(path: string) {
  return pathBasename(path) || path;
}

function extOf(path: string) {
  const name = fileNameFromPath(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

function isImagePath(path: string) {
  return /\.(?:jpe?g|png|gif|webp|bmp|svg)$/i.test(path);
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

function byteLength(text: string) {
  return new TextEncoder().encode(text).length;
}

/**
 * Truncate `text` to at most `maxBytes` UTF-8 bytes **on a character boundary**,
 * so a multibyte sequence (CJK, emoji) is never sliced mid-way — which would
 * decode to a `U+FFFD` replacement char injected into the model prompt.
 * Backs up from the byte limit past any UTF-8 continuation bytes (`0b10xxxxxx`).
 */
function truncateToBytes(text: string, maxBytes: number): { text: string; truncated: boolean } {
  const bytes = new TextEncoder().encode(text);
  if (bytes.length <= maxBytes)
    return { text, truncated: false };
  let end = maxBytes;
  while (end > 0 && ((bytes[end] ?? 0) & 0xC0) === 0x80)
    end--;
  return { text: new TextDecoder().decode(bytes.subarray(0, end)), truncated: true };
}

function capText(text: string): { text: string; truncated: boolean } {
  let truncated = false;
  let lines = text.split("\n");
  if (lines.length > INLINE_MAX_LINES_PER_FILE) {
    lines = lines.slice(0, INLINE_MAX_LINES_PER_FILE);
    truncated = true;
  }
  const capped = truncateToBytes(lines.join("\n"), INLINE_MAX_BYTES_PER_FILE);
  return { text: capped.text, truncated: truncated || capped.truncated };
}

function base64ToBytes(base64: string) {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++)
    bytes[i] = binary.charCodeAt(i);
  return bytes;
}

async function extractPdfText(path: string) {
  const base64 = await readFileBase64({ maxBytes: READ_SOURCE_MAX_BYTES, path });
  const loadingTask = pdfjs.getDocument({ data: base64ToBytes(base64) });
  try {
    const pdf = await loadingTask.promise;
    const parts: string[] = [];
    let bytes = 0;
    const lastPage = Math.min(pdf.numPages, MAX_PDF_PAGES);
    for (let page = 1; page <= lastPage; page++) {
      const content = await pdf.getPage(page).then(p => p.getTextContent());
      const text = content.items
        .map(item => ("str" in item ? item.str : ""))
        .join(" ");
      parts.push(text);
      bytes += byteLength(text);
      if (bytes > INLINE_MAX_BYTES_PER_FILE * 2)
        break;
    }
    return parts.join("\n");
  }
  finally {
    await loadingTask.destroy();
  }
}

async function extractTextFile(path: string) {
  const result = await readTextFilePreview({ maxBytes: INLINE_MAX_BYTES_PER_FILE, path });
  return result.content;
}

/**
 * Extract text/PDF attachment contents and build the model-facing inline block.
 * Returns "" when there is nothing to inline. Caps: 30KB/2000 lines per file,
 * 60KB total. This goes into `promptContent` only — never the visible bubble.
 */
export async function buildInlineAttachmentContext(attachments: MessageAttachment[]) {
  const targets = attachments.filter(a => a.kind === "pdf" || a.kind === "text");
  if (targets.length === 0)
    return "";

  let total = 0;
  const blocks: string[] = [];
  for (const attachment of targets) {
    // The on-disk path (inside the thread's working directory) so the model can
    // read the actual file when it needs more than the inlined text — e.g. a
    // scanned PDF with no extractable text, or content past the truncation cap.
    const header = (suffix: string) =>
      `===== ${attachment.name}${suffix} =====\n文件路径：${attachment.path}`;
    if (total >= INLINE_MAX_TOTAL_BYTES) {
      blocks.push(`${header("")}\n[已省略：超出附件内联总量上限，如需完整内容请读取上述文件路径]`);
      continue;
    }
    try {
      const raw = attachment.kind === "pdf"
        ? await extractPdfText(attachment.path)
        : await extractTextFile(attachment.path);
      if (attachment.kind === "pdf" && !raw.trim()) {
        blocks.push(`${header(" (PDF)")}\n[该 PDF 无可提取文本，可能是扫描件，如需处理请读取上述文件路径]`);
        continue;
      }
      const capped = capText(raw);
      const remaining = INLINE_MAX_TOTAL_BYTES - total;
      const fitted = truncateToBytes(capped.text, remaining);
      const text = fitted.text;
      const truncated = capped.truncated || fitted.truncated;
      total += byteLength(text);
      const tag = attachment.kind === "pdf" ? "PDF" : "文本";
      blocks.push(`${header(` (${tag}${truncated ? "，已截断" : ""})`)}\n${text}`);
    }
    catch {
      blocks.push(`${header("")}\n[读取失败，可尝试直接读取上述文件路径]`);
    }
  }

  if (blocks.length === 0)
    return "";
  return `\n\n附带文件内容（已为你读取，仅作上下文；文件已保存在下列路径，位于当前工作目录内，需要时可直接读取）：\n\n${blocks.join("\n\n")}`;
}

function loadImage(src: string) {
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("image decode failed"));
    image.src = src;
  });
}

/**
 * Downscale an image to ~96px and persist a JPEG thumbnail under the thread's
 * persistent image dir (`~/.future/app/images/<threadId>/thumb`). The backend
 * assigns a unique filename, so no client-side key is needed.
 */
export async function generateImageThumbnail(path: string, threadId: string): Promise<string | null> {
  try {
    const ext = extOf(path);
    const base64 = await readFileBase64({ maxBytes: READ_SOURCE_MAX_BYTES, path });
    const mime = EXT_IMAGE_MIME[ext] ?? "image/png";
    const image = await loadImage(`data:${mime};base64,${base64}`);
    const max = 96;
    const scale = Math.min(1, max / Math.max(image.width || 1, image.height || 1));
    const width = Math.max(1, Math.round((image.width || max) * scale));
    const height = Math.max(1, Math.round((image.height || max) * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx)
      return null;
    ctx.drawImage(image, 0, 0, width, height);
    const jpeg = canvas.toDataURL("image/jpeg", 0.6).split(",")[1] ?? "";
    if (!jpeg)
      return null;
    return await writeThumbnail({ base64Jpeg: jpeg, threadId });
  }
  catch {
    return null;
  }
}

export function parseMessageContent(content: string, contentType?: string) {
  if (contentType !== "mixed") {
    return { attachments: [] as MessageAttachment[], inlineContext: "", text: content };
  }

  try {
    const parsed = JSON.parse(content) as StoredMixedMessage;
    if (parsed.type !== "user_message")
      return { attachments: [] as MessageAttachment[], inlineContext: "", text: content };

    return {
      attachments: Array.isArray(parsed.attachments) ? parsed.attachments : [],
      inlineContext: parsed.inlineContext ?? "",
      text: parsed.text ?? "",
    };
  }
  catch {
    return { attachments: [] as MessageAttachment[], inlineContext: "", text: content };
  }
}

export function stringifyMessageContent(
  text: string,
  attachments: MessageAttachment[],
  inlineContext?: string,
) {
  return JSON.stringify({
    type: "user_message",
    text,
    attachments,
    inlineContext: inlineContext || undefined,
  });
}

export function imageAttachmentPaths(attachments: MessageAttachment[]) {
  return attachments
    .filter(attachment => attachment.kind === "image" || isImagePath(attachment.path))
    .map(attachment => attachment.path);
}
