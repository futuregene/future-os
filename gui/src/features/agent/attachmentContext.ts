import type { MessageAttachment } from "./agentThreadTypes";
import * as pdfjs from "pdfjs-dist";
import { readFileBase64, readTextFilePreview } from "../../integrations/storage/files";
import { READ_SOURCE_MAX_BYTES } from "./attachments";

pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url,
).toString();

const INLINE_MAX_BYTES_PER_FILE = 30 * 1024;
const INLINE_MAX_LINES_PER_FILE = 2000;
const INLINE_MAX_TOTAL_BYTES = 60 * 1024;
/**
 * Upper bound on PDF pages scanned for text. A large scanned PDF yields empty
 * text per page, so the byte cap never trips — without this it would walk every
 * page (thousands) before giving up.
 */
const MAX_PDF_PAGES = 100;

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
