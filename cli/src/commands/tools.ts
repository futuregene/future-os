import { readFile } from "node:fs/promises";
import { writeFile } from "node:fs/promises";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";
import { resolve as pathResolve, dirname as pathDirname } from "node:path";
import { homedir } from "node:os";

import { getRecord, isRecord, isNodeError } from "../utils/object.js";
import { mcpPost, mcpUrl, initializeSession } from "./mcp.js";
import { BROWSER_TOOL_CATALOG, callBrowserTool, isBrowserTool } from "./browser-tools.js";

// ── Constants ────────────────────────────────────────────────────────────────

const AUTH_FILE = join(homedir(), ".future", "agent", "auth.json");
const FUTURE_PROVIDER = "future";

// ── Tool catalog (matches config/api.yaml, manually maintained) ──────────────

interface ToolEntry {
  description: string;
  args: Record<string, string>;
  example: string;
}

export const TOOL_CATALOG: Record<string, ToolEntry> = {
  ...BROWSER_TOOL_CATALOG,
  search_paper: {
    description: "Search academic papers and extract requested information.",
    args: {
      information_to_extract: "string",
    },
    example: '{"information_to_extract": "inheritance pattern and typical age of onset"}',
  },
  get_paper: {
    description: "Get full paper content by identifier (PMID, DOI). Returns structured Paper object with metadata (title, authors, journal, year, DOI) and complete body_text.",
    args: {
      paper_id: "string (required)",
      max_k: "int",
    },
    example: '{"paper_id": "PMID:12345678"}',
  },
  image_gen: {
    description: "Generate images from text prompts.",
    args: {
      prompt: "string (required)",
      size: 'string (default: "1024x1024")',
      quality: 'string (default: "medium")',
      n: "int (1–10)",
      output_format: 'string (default: "png")',
    },
    example: '{"prompt": "A photograph of a red fox in an autumn forest, golden hour", "size": "1024x1024"}',
  },
  image_edit: {
    description: "Edit an existing image using text instructions. Use --input <path> to provide the source image, --mask <path> for an optional mask.",
    args: {
      prompt: "string (required)",
      size: 'string (default: "1024x1024")',
      quality: 'string (default: "medium")',
    },
    example: '--input photo.png --args \'{"prompt": "Convert to watercolor painting"}\'',
  },
  read_image: {
    description: "Read and analyze an image. Use --input <path> to provide the image file — supports OCR, object recognition, and visual Q&A.",
    args: {
      question: "string (required, e.g. 'Extract text' or 'Describe this image')",
      mime_type: 'string (default: "image/png")',
      max_tokens: "integer (default: 2000)",
    },
    example: '--input photo.png --args \'{"question": "What text is in this image?"}\'',
  },
  parse_doc: {
    description: "Parse PDF and Word documents into markdown. Use --input <path> to provide the document — get structured markdown with text, tables, and formulas preserved.",
    args: {
      file_type: 'string (optional, "pdf" or "docx", default: "pdf")',
    },
    example: '--input report.pdf --args \'{"file_type": "pdf"}\'',
  },
  web_search: {
    description: "Search the web. Returns titles, links, and snippets.",
    args: {
      query: "string (required)",
      count: "integer (default: 10, max: 50)",
    },
    example: '{"query": "BRCA1 variant classification guidelines 2025", "count": 5}',
  },
  fetch_url: {
    description: "Fetch and extract content from a web page URL. Returns page title and compact content.",
    args: {
      url: "string (required)",
    },
    example: '{"url": "https://en.wikipedia.org/wiki/BRCA1"}',
  },
};

// ── Auth ─────────────────────────────────────────────────────────────────────

export async function loadApiKey(): Promise<string> {
  const envKey = process.env["FUTURE_API_KEY"];
  if (envKey) return envKey;

  try {
    const raw = await readFile(AUTH_FILE, "utf8");
    const auth = JSON.parse(raw) as unknown;
    if (!isRecord(auth)) throw new Error("auth.json must be a JSON object");

    const future = auth[FUTURE_PROVIDER];
    if (!isRecord(future)) {
      throw new Error(`Not logged in. Run "future auth login" first, or set FUTURE_API_KEY.`);
    }

    const key = typeof (future as Record<string, unknown>).key === "string"
      ? (future as Record<string, unknown>).key as string
      : undefined;
    if (!key) {
      throw new Error(`Not logged in. Run "future auth login" first, or set FUTURE_API_KEY.`);
    }
    return key;
  } catch (err) {
    const testKey = process.env["FUTURE_API_TEST_KEY"];
    if (testKey) return testKey;
    if (isNodeError(err) && err.code === "ENOENT") {
      throw new Error(`Not logged in. Run "future auth login" first, or set FUTURE_API_KEY.`);
    }
    throw err;
  }
}

// ── Tool operations ──────────────────────────────────────────────────────────

async function listRemoteTools(apiKey: string): Promise<Array<{ name: string; description: string }>> {
  const sessionId = await initializeSession(apiKey);
  const { body } = await mcpPost(await mcpUrl(), "tools/list", {}, apiKey, sessionId, 2);

  if (body.error) {
    const err = body.error as Record<string, unknown>;
    const code = typeof err.code === "number" ? String(err.code) : String(err.code ?? "unknown");
    const message = typeof err.message === "string" ? err.message : "unknown error";
    throw new Error(`tools/list failed: code=${code}, message=${message}`);
  }
  const result = getRecord(body.result);
  const tools = Array.isArray(result?.tools) ? result!.tools : [];
  return tools.filter(isRecord).map((t) => {
    const record = t as Record<string, unknown>;
    return {
      name: typeof record.name === "string" ? record.name : "unknown",
      description: typeof record.description === "string" ? record.description : "",
    };
  });
}

interface CallToolResponse {
  text: string;
  structuredContent: Record<string, unknown> | null;
}

async function callRemoteTool(apiKey: string, name: string, args: Record<string, unknown>, timeoutMs?: number): Promise<CallToolResponse> {
  const sessionId = await initializeSession(apiKey);
  const { body } = await mcpPost(await mcpUrl(), "tools/call", {
    name,
    arguments: args,
  }, apiKey, sessionId, 2, timeoutMs);

  if (body.error) {
    // Sanitize: only expose code and message — never leak upstream internals
    // (RequestId, HostId, nested data bodies, troubleshooting URLs, etc.).
    const err = body.error as Record<string, unknown>;
    const code = typeof err.code === "number" ? String(err.code) : String(err.code ?? "unknown");
    const message = typeof err.message === "string" ? err.message : "unknown error";
    throw new Error(`tools/call failed: code=${code}, message=${message}`);
  }

  const result = getRecord(body.result);
  const content = Array.isArray(result?.content) ? result!.content : [];
  const texts = content
    .filter(isRecord)
    .map((block) => {
      const b = block as Record<string, unknown>;
      if (b.type === "text") return String(b.text ?? "");
      if (b.type === "resource") return JSON.stringify(b.resource, null, 2);
      return JSON.stringify(b, null, 2);
    });

  return {
    text: texts.join("\n"),
    structuredContent: isRecord(result?.structuredContent)
      ? (result!.structuredContent as Record<string, unknown>)
      : null,
  };
}

// ── Tool-specific result formatters ─────────────────────────────────────────
// Each formatter extracts the useful content from MCP responses, filtering
// out JSON wrapper noise and usage metadata. Use --raw to get the original output.

/** Format an MCP tool result for LLM agent consumption.
 *  Returns clean, structured output that omits JSON wrappers and usage metadata.
 *  Image tools save files to disk and return the absolute path. */
async function formatToolResult(
  toolName: string,
  result: CallToolResponse,
  outputPath: string | null,
): Promise<string> {
  const sc = result.structuredContent;
  if (!sc) return result.text;

  switch (toolName) {
    case "search_paper": return formatSearchPaper(sc);
    case "get_paper":    return formatGetPaper(sc);
    case "web_search":   return formatWebSearch(sc);
    case "fetch_url":    return formatFetchUrl(sc);
    case "read_image":   return formatReadImage(sc);
    case "parse_doc":    return formatParseDoc(sc);
    case "image_gen":
    case "image_edit":   return formatImageResult(toolName, sc, outputPath);
    default:             return result.text || JSON.stringify(sc, null, 2);
  }
}

// ── search_paper ────────────────────────────────────────────────────────────

interface PaperItem {
  paper_id?: unknown; title?: unknown; authors?: unknown; journal?: unknown;
  volume?: unknown; pages?: unknown; publication_date?: unknown; year?: unknown;
  doi?: unknown; pubmed_id?: unknown; pmc_id?: unknown; arxiv_id?: unknown;
  url?: unknown; citation_count?: unknown; impact_factor?: unknown;
  ai_summary?: unknown; source?: unknown;
}

function formatSearchPaper(sc: Record<string, unknown>): string {
  const results = sc["results"];
  if (!Array.isArray(results) || results.length === 0) return "No papers found.";

  const parts: string[] = [];
  for (const qr of results) {
    if (!isRecord(qr)) continue;
    const query = String(qr["query"] ?? "");
    const papers: PaperItem[] = Array.isArray(qr["papers"]) ? qr["papers"] as PaperItem[] : [];
    if (papers.length === 0) continue;

    parts.push(`## Search Results: "${query}" (${papers.length} papers)\n`);
    for (let i = 0; i < papers.length; i++) {
      const p = papers[i];
      const title = str(p.title);
      const authors = str(p.authors);
      const journal = str(p.journal);
      const year = str(p.year);
      const doi = str(p.doi);
      const url = str(p.url);
      const aiSummary = str(p.ai_summary);

      parts.push(`### ${i + 1}. ${title || "Untitled"}`);
      if (authors) parts.push(`**Authors:** ${authors}`);
      if (journal || year) {
        parts.push(`**Journal:** ${[journal, year ? `(${year})` : ""].filter(Boolean).join(" ")}`);
      }
      if (doi) parts.push(`**DOI:** ${doi}`);
      if (url) parts.push(`**URL:** ${url}`);
      if (aiSummary) parts.push(`\n${aiSummary}`);
      parts.push("");
    }
  }
  return parts.join("\n").trim() || "No papers found.";
}

// ── get_paper ───────────────────────────────────────────────────────────────

function formatGetPaper(sc: Record<string, unknown>): string {
  const paper = sc["paper"];
  if (!isRecord(paper)) return "No paper found.";

  const meta = paper as Record<string, unknown>;
  const title = str(meta.title);
  const authors = str(meta.authors);
  const journal = str(meta.journal);
  const year = str(meta.year);
  const doi = str(meta.doi);
  const pubmedId = str(meta.pubmed_id);
  const url = str(meta.url);
  const bodyText = str(meta.body_text);

  const parts: string[] = [];
  parts.push(`# ${title || "Untitled"}`);
  if (authors) parts.push(`**Authors:** ${authors}`);
  if (journal || year) {
    parts.push(`**Journal:** ${[journal, year ? `(${year})` : ""].filter(Boolean).join(" ")}`);
  }
  if (doi) parts.push(`**DOI:** ${doi}${pubmedId ? ` | **PMID:** ${pubmedId}` : ""}`);
  if (url) parts.push(`**URL:** ${url}`);
  parts.push("");
  parts.push("---");
  parts.push("");
  parts.push(bodyText || "(No body text available)");

  return parts.join("\n");
}

// ── web_search ──────────────────────────────────────────────────────────────

function formatWebSearch(sc: Record<string, unknown>): string {
  const query = str(sc["query"]);
  const results = sc["results"];
  if (!Array.isArray(results) || results.length === 0) {
    return `## Search Results: "${query}"\n\nNo results found.`;
  }

  const parts: string[] = [];
  parts.push(`## Search Results: "${query}" (${results.length} results)\n`);
  for (let i = 0; i < results.length; i++) {
    const r = results[i];
    if (!isRecord(r)) continue;
    const title = str((r as Record<string, unknown>).title);
    const link = str((r as Record<string, unknown>).link);
    const snippet = str((r as Record<string, unknown>).snippet);

    parts.push(`${i + 1}. **${title || "Untitled"}**`);
    if (link) parts.push(`   ${link}`);
    if (snippet) parts.push(`   ${snippet}`);
    parts.push("");
  }
  return parts.join("\n").trim();
}

// ── fetch_url ───────────────────────────────────────────────────────────────

function formatFetchUrl(sc: Record<string, unknown>): string {
  const url = str(sc["url"]);
  const title = str(sc["title"]);
  const content = str(sc["content"]);

  const parts: string[] = [];
  if (title) parts.push(`# ${title}`);
  parts.push(`**URL:** ${url || "(unknown)"}`);
  parts.push("");
  parts.push(content || "(No content)");
  return parts.join("\n");
}

// ── read_image ──────────────────────────────────────────────────────────────

function formatReadImage(sc: Record<string, unknown>): string {
  // The answer text is the only useful output for an agent.
  const answer = str(sc["answer"]);
  return answer || "(No answer)";
}

// ── parse_doc ───────────────────────────────────────────────────────────────

function formatParseDoc(sc: Record<string, unknown>): string {
  // Full markdown content — the text preview may be truncated.
  const markdown = str(sc["markdown"]);
  return markdown || "(No content)";
}

// ── image_gen / image_edit ──────────────────────────────────────────────────

/** Default directory for generated/edited images. */
const IMAGE_OUTPUT_DIR = join(homedir(), ".future", "agent", "images");

async function formatImageResult(
  toolName: string,
  sc: Record<string, unknown>,
  outputPath: string | null,
): Promise<string> {
  const images = sc["images"];
  const prompt = str(sc["prompt"]);
  const size = str(sc["size"]) || "unknown";
  const quality = str(sc["quality"]) || "unknown";
  const fmt = str(sc["format"]) || "png";

  const verb = toolName === "image_edit" ? "Image edited" : "Image generated";
  const parts: string[] = [];
  parts.push(`[${verb}: ${size} ${quality} ${fmt}]`);
  if (prompt) parts.push(`Prompt: ${prompt}`);

  const imageList: Array<{ b64: string; fmt: string }> = [];
  if (Array.isArray(images)) {
    for (const img of images) {
      if (!isRecord(img)) continue;
      const b64 = img["b64_json"];
      const imgFmt = str(img["format"]) || fmt;
      if (typeof b64 === "string") imageList.push({ b64, fmt: imgFmt });
    }
  }
  if (imageList.length === 0) return parts.join("\n");

  // Generate output paths
  const paths: string[] = [];
  for (let i = 0; i < imageList.length; i++) {
    const img = imageList[i];
    const ext = img.fmt === "jpeg" ? "jpg" : img.fmt;
    let filePath: string;

    if (outputPath) {
      // Single image with explicit --output: use it directly.
      // Multiple images: insert a suffix before the extension.
      if (imageList.length === 1) {
        filePath = pathResolve(outputPath);
      } else {
        const dot = outputPath.lastIndexOf(".");
        const base = dot > 0 ? outputPath.slice(0, dot) : outputPath;
        const suffix = dot > 0 ? outputPath.slice(dot) : `.${ext}`;
        filePath = pathResolve(`${base}_${i + 1}${suffix}`);
      }
    } else {
      const ts = Date.now();
      const suffix = imageList.length > 1 ? `_${i + 1}` : "";
      const filename = `future-image-${ts}${suffix}.${ext}`;
      filePath = join(IMAGE_OUTPUT_DIR, filename);
    }

    await fsMkdirForPath(filePath);
    await writeFile(filePath, Buffer.from(img.b64, "base64"));
    paths.push(filePath);
  }

  parts.push("");
  for (let i = 0; i < paths.length; i++) {
    if (imageList.length > 1) parts.push(`Image ${i + 1}: ${paths[i]}`);
    else parts.push(`Saved: ${paths[i]}`);
  }

  return parts.join("\n");
}

async function fsMkdirForPath(filePath: string): Promise<void> {
  const dir = pathDirname(filePath);
  try { await mkdir(dir, { recursive: true }); } catch { /* ok */ }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

// ── Public command ───────────────────────────────────────────────────────────

export type ToolsCommand = "list" | "call";

export function isToolsCommand(command: string): command is ToolsCommand {
  return command === "list" || command === "call";
}

export function parseToolArgs(raw: string): Record<string, unknown> {
  const candidates = toolArgCandidates(raw);
  let lastError: unknown;

  for (const candidate of candidates) {
    try {
      let value: unknown = JSON.parse(candidate);
      // Windows process creation can preserve an extra encoded JSON layer.
      if (typeof value === "string") value = JSON.parse(value);
      if (isRecord(value)) return value;
    } catch (error) {
      lastError = error;
    }
  }

  // cmd.exe can consume every double quote before Node receives argv, leaving
  // `{prompt:puppy,size:1024x1024}`. Recover this common flat-object form.
  const relaxed = parseCmdObject(stripOuterQuotes(raw));
  if (relaxed) return relaxed;

  throw new Error(
    `--args must be a JSON object, e.g. '{"prompt":"..."}' (${lastError instanceof Error ? lastError.message : "invalid JSON"})`,
  );
}

/** Produce conservative variants for quoting changed by cmd.exe, PowerShell,
 * or a parent process using Windows command-line escaping. */
function toolArgCandidates(raw: string): string[] {
  const stripped = stripOuterQuotes(raw);
  return [...new Set([
    raw.trim(),
    stripped,
    stripped.replace(/\\"/g, '"').replace(/\\'/g, "'"),
  ])];
}

/** Parse the simple key/value object produced when cmd.exe strips JSON quotes.
 * Values stay strings unless they are unambiguously JSON primitives.
 * Supports nested objects/arrays via recursive parsing. */
function parseCmdObject(raw: string): Record<string, unknown> | null {
  const text = raw.trim();
  if (!text.startsWith("{") || !text.endsWith("}")) return null;

  const result: Record<string, unknown> = {};
  const body = text.slice(1, -1).trim();
  if (!body) return result;

  // Split only on top-level commas — commas nested inside braces/brackets
  // are part of a value and must not be treated as field separators.
  for (const field of splitTopLevel(body, ",")) {
    const colon = field.indexOf(":");
    if (colon <= 0) return null;
    const key = field.slice(0, colon).trim().replace(/^['"]|['"]$/g, "");
    const rawValue = field.slice(colon + 1).trim();
    if (!key) return null;
    result[key] = parseCmdValue(rawValue);
  }
  return result;
}

/** Split text on separator only at brace/bracket depth 0.
 *  Commas inside nested {…} or […] are left intact. */
function splitTopLevel(text: string, separator: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === "{" || ch === "[") depth++;
    else if (ch === "}" || ch === "]") depth--;
    else if (ch === separator && depth === 0) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(text.slice(start));
  return parts;
}

/** Parse a single value from cmd.exe-mangled JSON.
 *  Primitive detection first, then recursive object/array parsing. */
function parseCmdValue(raw: string): unknown {
  const text = raw.trim().replace(/^['"]|['"]$/g, "");
  if (text === "true") return true;
  if (text === "false") return false;
  if (text === "null") return null;
  if (/^-?\d+(?:\.\d+)?$/.test(text)) return Number(text);

  // Recursively parse nested objects
  if (text.startsWith("{") && text.endsWith("}")) {
    const nested = parseCmdObject(text);
    if (nested) return nested;
  }
  // Recursively parse nested arrays
  if (text.startsWith("[") && text.endsWith("]")) {
    const inner = text.slice(1, -1).trim();
    if (!inner) return [];
    const items = splitTopLevel(inner, ",");
    return items.map((item) => parseCmdValue(item.trim()));
  }

  return text;
}

function stripOuterQuotes(input: string): string {
  const trimmed = input.trim();
  if (trimmed.length >= 2) {
    const first = trimmed[0];
    const last = trimmed[trimmed.length - 1];
    if ((first === '"' && last === '"') || (first === "'" && last === "'")) {
      return trimmed.slice(1, -1);
    }
  }
  return trimmed;
}

// ── Path-to-base64 resolution ──────────────────────────────────────────────

/** Resolve image_path / doc_path fields to base64 before sending to API.
 *  This allows users to pass file paths instead of giant base64 strings. */
async function resolveLocalPaths(args: Record<string, unknown>): Promise<Record<string, unknown>> {
  const resolved = { ...args };

  // read_image / image_edit: support image_path and mask_path
  for (const key of ["image_path", "doc_path", "mask_path"]) {
    const val = resolved[key];
    if (typeof val !== "string") continue;
    try {
      const buf = await readFile(val);
      const b64Key = key === "image_path" ? "image_b64"
        : key === "mask_path" ? "mask_b64"
        : "doc_b64";
      resolved[b64Key] = buf.toString("base64");
      // Keep the original path so API knows the filename too
    } catch {
      // File not found — leave as-is, let API report the error
    }
  }

  return resolved;
}

// ── Public command entry ────────────────────────────────────────────────────

export async function tools(command: ToolsCommand, args: string[]): Promise<void> {
  if (command === "list") {
    const jsonFlag = args.includes("--json");
    let tools: Array<{ name: string; description: string }> = Object.entries(BROWSER_TOOL_CATALOG)
      .map(([name, entry]) => ({ name, description: entry.description }));

    try {
      const apiKey = await loadApiKey();
      tools = [...tools, ...await listRemoteTools(apiKey)];
    } catch (error) {
      if (jsonFlag) {
        tools = tools.map((tool) => ({ ...tool }));
      } else {
        console.error(
          `Remote tools unavailable: ${error instanceof Error ? error.message : String(error)}`,
        );
        console.error("Showing local tools only.\n");
      }
    }

    if (jsonFlag) {
      console.log(JSON.stringify(tools, null, 2));
    } else {
      for (const t of tools) {
        const desc = t.description.slice(0, 80);
        console.log(`  ${t.name.padEnd(30)} ${desc}`);
      }
      console.log(`\n${tools.length} tools available.`);
    }
    return;
  }

  if (command === "call") {
    const toolName = args[0];
    if (!toolName) {
      console.error("Usage: future tools call <tool_name> [--args '<json>' | --stdin] [--input <path>] [--mask <path>] [--raw] [--output <path>] [--timeout <seconds>]");
      process.exitCode = 1;
      return;
    }

    let toolArgs: Record<string, unknown> = {};
    const argsIdx = args.indexOf("--args");
    const stdinFlag = args.includes("--stdin");
    const outputIdx = args.indexOf("--output");
    const outputPath = outputIdx !== -1 && outputIdx + 1 < args.length
      ? args[outputIdx + 1]
      : null;
    // --input and --mask: read local files and inject as base64 to MCP,
    // so base64 strings never appear in user-facing args or output.
    const inputIdx = args.indexOf("--input");
    const inputPath = inputIdx !== -1 && inputIdx + 1 < args.length && !args[inputIdx + 1].startsWith("--")
      ? args[inputIdx + 1]
      : null;
    const maskIdx = args.indexOf("--mask");
    const maskPath = maskIdx !== -1 && maskIdx + 1 < args.length && !args[maskIdx + 1].startsWith("--")
      ? args[maskIdx + 1]
      : null;
    const timeoutIdx = args.indexOf("--timeout");
    const timeoutSec = timeoutIdx !== -1 && timeoutIdx + 1 < args.length
      ? parseInt(args[timeoutIdx + 1], 10) || 0
      : 0;
    const timeoutMs = timeoutSec > 0
      ? timeoutSec * 1000
      : ["image_gen", "image_edit"].includes(toolName)
        ? 600_000   // image generation can take 2-10 minutes
        : undefined; // other tools use mcpPost's default 60s

    const rawFlag = args.includes("--raw");

    if (stdinFlag) {
      // Read from stdin
      const chunks: Buffer[] = [];
      for await (const chunk of process.stdin) {
        chunks.push(chunk as Buffer);
      }
      toolArgs = parseToolArgs(Buffer.concat(chunks).toString());
    } else if (argsIdx !== -1 && argsIdx + 1 < args.length) {
      // cmd.exe strips double quotes from JSON, which turns spaces inside
      // string values into argument boundaries. Rejoin adjacent fragments
      // that belong to the same JSON argument (stopping at the next --flag).
      let raw = args[argsIdx + 1];
      for (let i = argsIdx + 2; i < args.length && !args[i].startsWith("--"); i++) {
        raw += " " + args[i];
      }
      toolArgs = parseToolArgs(raw);
    }

    // Accept --<param> <value> sugar as an alternative to --args JSON.
    // The model sometimes generates individual flags (e.g. --query "..."
    // --count 8) instead of --args '{"query":"...","count":8}'.
    const knownFlags = new Set([
      "--args", "--stdin", "--input", "--mask",
      "--output", "--timeout", "--raw",
    ]);
    for (let i = 2; i < args.length - 1; i++) {
      const arg = args[i];
      if (arg.startsWith("--") && !knownFlags.has(arg)) {
        const val = args[i + 1];
        // Don't consume a value that looks like another flag
        if (!val.startsWith("--")) {
          const key = arg.slice(2); // strip "--" prefix
          toolArgs[key] = parseCmdValue(val);
          i++; // skip the value
        }
      }
    }

    // Resolve image_path / doc_path → base64 before sending to API
    toolArgs = await resolveLocalPaths(toolArgs);

    // Resolve --input / --mask flags to base64, tool-aware:
    //   image_edit, read_image → image_b64 / mask_b64
    //   parse_doc              → doc_b64
    if (inputPath) {
      try {
        const buf = await readFile(inputPath);
        const b64Key = toolName === "parse_doc" ? "doc_b64" : "image_b64";
        toolArgs[b64Key] = buf.toString("base64");
      } catch {
        console.error(`Error: cannot read input file: ${inputPath}`);
        process.exit(1);
      }
    }
    if (maskPath) {
      try {
        const buf = await readFile(maskPath);
        toolArgs["mask_b64"] = buf.toString("base64");
      } catch {
        console.error(`Error: cannot read mask file: ${maskPath}`);
        process.exit(1);
      }
    }

    if (isBrowserTool(toolName)) {
      try {
        const result = await callBrowserTool(toolName, toolArgs);
        const output = result.structuredContent && Object.keys(result.structuredContent).length > 0
          ? JSON.stringify(result.structuredContent, null, 2)
          : result.text ?? "";
        await writeStdout(`${output}\n`);
        process.exit(0);
      } catch (error) {
        const msg = error instanceof Error ? error.message
          : (typeof error === "object" && error !== null && "message" in error)
            ? String((error as Record<string, unknown>).message)
            : String(error);
        await writeStderr(`${msg}\n`);
        process.exit(1);
      }
    }

    const apiKey = await loadApiKey();
    const result = await callRemoteTool(apiKey, toolName, toolArgs, timeoutMs);

    // --raw: output the original MCP result directly (structured content as JSON, or text).
    // Otherwise: format the result for LLM agent consumption — strip JSON wrappers
    // and metadata, keeping only the useful content. Image tools save files to disk
    // and output the absolute path.
    if (rawFlag) {
      if (result.structuredContent && Object.keys(result.structuredContent).length > 0) {
        console.log(JSON.stringify(result.structuredContent, null, 2));
      } else {
        console.log(result.text);
      }
    } else {
      console.log(await formatToolResult(toolName, result, outputPath));
    }

    return;
  }
}

function writeStdout(text: string): Promise<void> {
  return new Promise((resolve) => process.stdout.write(text, () => resolve()));
}

function writeStderr(text: string): Promise<void> {
  return new Promise((resolve) => process.stderr.write(text, () => resolve()));
}
