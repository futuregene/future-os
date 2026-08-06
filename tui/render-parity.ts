/**
 * Render-parity driver (TypeScript side).
 *
 * Reads the shared corpus (`tui/rust/tests/parity-corpus.json`) and renders
 * every case with the TypeScript implementation — MarkdownRenderer,
 * ChatArea (including the streaming prefix-cache path) and the
 * terminal-image helpers — printing one line per case:
 *
 *     <kind>|<name>|<base64(JSON.stringify(result))>
 *
 * The Rust twin (`tui/rust/examples/render_parity.rs`) does the same; the
 * harness (`tui/rust/tests/diff-ts-rust.sh`) byte-compares the two outputs.
 * JSON.stringify + Buffer base64 gives a canonical byte encoding that the
 * Rust side reproduces exactly (serde_json escapes match JS, standard
 * base64 alphabet with padding).
 *
 * Usage: bun render-parity.ts <corpus.json>
 */

import { readFileSync } from "node:fs";
import { MarkdownRenderer } from "./src/components/markdown.js";
import { ChatArea } from "./src/components/chat-area.js";
import * as timg from "./src/terminal-image.js";
import { fg, dim, italic } from "./src/theme.js";

const corpus = JSON.parse(readFileSync(process.argv[2], "utf8"));
const out: string[] = [];

function emit(kind: string, name: string, value: unknown): void {
  out.push(`${kind}|${name}|${Buffer.from(JSON.stringify(value), "utf8").toString("base64")}`);
}

// ─── Markdown ─────────────────────────────────────────────────────────────

// The mdThinking theme (chat-area.ts): every accent color mapped to the
// thinking gray (244); bold/italic/underline stay attribute-only.
const thinkingTheme = {
  heading: (t: string) => fg(244, t),
  link: (t: string) => fg(244, t),
  linkUrl: (t: string) => fg(244, t),
  code: (t: string) => fg(244, t),
  codeBlock: (t: string) => fg(244, dim(t)),
  codeBlockBorder: (t: string) => fg(244, dim(t)),
  quote: (t: string) => fg(244, italic(t)),
  quoteBorder: (t: string) => fg(244, t),
  hr: (t: string) => fg(244, t),
  listBullet: (t: string) => fg(244, t),
  strikethrough: (t: string) => fg(244, t),
};

function fgFn(spec?: string): ((t: string) => string) | undefined {
  if (!spec) return undefined;
  const n = Number(spec.replace("fg", ""));
  return (t: string) => fg(n, t);
}

for (const c of corpus.markdown ?? []) {
  const theme = c.theme === "thinking" ? thinkingTheme : undefined;
  const style = c.opts?.defaultStyle
    ? { bold: c.opts.defaultStyle.bold, italic: c.opts.defaultStyle.italic, color: fgFn(c.opts.defaultStyle.color) }
    : undefined;
  const md = new MarkdownRenderer(theme, style);
  if (c.opts?.paddingX !== undefined) {
    md.setPadding(c.opts.paddingX, c.opts.paddingY);
  }
  emit("markdown", c.name, md.render(c.text, c.width));
}

// ─── ChatArea ─────────────────────────────────────────────────────────────

for (const c of corpus.chat ?? []) {
  const chat = new ChatArea(c.width);
  chat.render(c.width);
  for (const m of c.msgs) {
    chat.addMessage(m);
  }
  if (c.thinkingHidden) chat.setThinkingHidden(true);
  if (c.viewportHeight !== undefined) chat.setViewportHeight(c.viewportHeight);
  const lines = c.viewportHeight !== undefined ? chat.render(c.width) : chat.renderAll(c.width);
  emit("chat", c.name, lines);
}

// ─── ChatArea streaming (prefix-cache path, one frame per delta) ──────────

for (const c of corpus.chatStream ?? []) {
  const chat = new ChatArea(c.width);
  chat.render(c.width);
  chat.addMessage({ id: "m", role: "assistant", content: "", pending: true });
  const frames: string[][] = [];
  for (const delta of c.deltas) {
    chat.appendToLastMessage(delta);
    frames.push(chat.renderAll(c.width));
  }
  emit("chatStream", c.name, frames);
}

// ─── terminal-image ───────────────────────────────────────────────────────

function dims(d: { widthPx: number; heightPx: number }) {
  return { widthPx: d.widthPx, heightPx: d.heightPx };
}

for (const c of corpus.image ?? []) {
  let result: unknown;
  switch (c.fn) {
    case "encodeKitty":
      result = timg.encodeKitty(c.args[0], c.args[1] ?? {});
      break;
    case "deleteKittyImage":
      result = timg.deleteKittyImage(c.args[0]);
      break;
    case "deleteAllKittyImages":
      result = timg.deleteAllKittyImages();
      break;
    case "encodeITerm2":
      result = timg.encodeITerm2(c.args[0], c.args[1] ?? {});
      break;
    case "calculateImageRows":
      result = timg.calculateImageRows(dims(c.args[0]), c.args[1], { widthPx: c.args[2].widthPx, heightPx: c.args[2].heightPx });
      break;
    case "getPngDimensions":
      result = timg.getPngDimensions(c.args[0]);
      break;
    case "getJpegDimensions":
      result = timg.getJpegDimensions(c.args[0]);
      break;
    case "getGifDimensions":
      result = timg.getGifDimensions(c.args[0]);
      break;
    case "getWebpDimensions":
      result = timg.getWebpDimensions(c.args[0]);
      break;
    case "getImageDimensions":
      result = timg.getImageDimensions(c.args[0], c.args[1]);
      break;
    case "isImageLine":
      result = timg.isImageLine(c.args[0]);
      break;
    case "extractKittyImageIds":
      result = timg.extractKittyImageIds(c.args[0]);
      break;
    case "collectKittyImageIds": {
      const set = timg.collectKittyImageIds(c.args[0]);
      result = [...set]; // insertion order; corpus lines are ascending
      break;
    }
    case "deleteKittyImages":
      result = timg.deleteKittyImages(c.args[0]);
      break;
    case "hyperlink":
      result = timg.hyperlink(c.args[0], c.args[1]);
      break;
    case "imageFallback":
      result = timg.imageFallback(c.args[0], c.args[1] ? dims(c.args[1]) : undefined, c.args[2]);
      break;
    case "renderImage":
      timg.setCapabilities(c.args[0]);
      result = timg.renderImage(c.args[1], dims(c.args[2]), c.args[3] ?? {});
      break;
    default:
      throw new Error(`unknown image fn: ${c.fn}`);
  }
  emit("image", c.name, result);
}

process.stdout.write(out.join("\n") + "\n");
