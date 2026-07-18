import { readFile } from "node:fs/promises";
import { writeFile } from "node:fs/promises";
import { mkdir } from "node:fs/promises";
import { closeSync, writeSync } from "node:fs";
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
  inputRequired?: boolean;    // tool needs --input <file>
  maskSupported?: boolean;    // tool also accepts --mask <file>
  outputSupported?: boolean;  // tool can save output to --output <path>
}

export const TOOL_CATALOG: Record<string, ToolEntry> = {
  ...BROWSER_TOOL_CATALOG,
  search_paper: {
    description: "Search academic papers and extract requested information.",
    args: {
      queries: "search terms, one per query (required)",
      information_to_extract: "what information to extract from the results (optional)",
      max_results_per_query: "max papers to return per query, 1-20 (optional, default: 10)",
    },
    example: '{"queries": ["CRISPR gene editing overview", "CRISPR applications 2025"], "information_to_extract": "key methods and recent advances", "max_results_per_query": 8}',
  },
  get_paper: {
    description: "Get full paper content by identifier (PMID, DOI). Returns metadata (title, authors, journal, year, DOI) and complete body_text.",
    args: {
      paper_id: 'paper identifier like "PMID:12345678" or "DOI:10.xxx/..." (required)',
      max_k: "max result chunks to return (optional)",
    },
    example: '{"paper_id": "PMID:12345678", "max_k": 3}',
  },
  image_gen: {
    description: "Generate images from a text prompt.",
    outputSupported: true,
    args: {
      prompt: "description of the image to generate (required)",
      size: 'output dimensions, e.g. "1024x1024", "1792x1024" (optional, default: 1024x1024)',
      quality: 'image quality: "standard" or "hd" (optional, default: standard)',
      n: "number of images to generate, 1–10 (optional, default: 1)",
      output_format: 'file format: "png", "jpg", or "webp" (optional, default: png)',
    },
    example: '{"prompt": "A red fox in an autumn forest, golden hour", "size": "1024x1024", "n": 1}',
  },
  image_edit: {
    description: "Edit an existing image using a text prompt. Requires --input <path> for the source image. Optional --mask <path> to limit edits to a region.",
    outputSupported: true,
    args: {
      prompt: "description of the desired edits (required)",
      size: 'output dimensions, e.g. "1024x1024" (optional)',
      quality: '"standard" or "hd" (optional)',
    },
    example: '{"prompt": "Convert to watercolor painting"}',
    inputRequired: true,
    maskSupported: true,
  },
  read_image: {
    description: "Analyze an image: OCR text extraction, object recognition, visual Q&A. Requires --input <path> for the image file.",
    args: {
      question: 'what to ask about the image, e.g. "What text is in this image?" or "Describe this image" (required)',
      mime_type: 'image MIME type (optional, default: image/png)',
      max_tokens: "max tokens in the response (optional, default: 2000)",
    },
    example: '{"question": "What text is in this image?"}',
    inputRequired: true,
  },
  parse_doc: {
    description: "Parse a PDF or Word document into markdown, preserving text, tables, and formulas. Requires --input <path> for the document.",
    args: {
      file_type: 'document type: "pdf" or "docx" (optional, default: pdf)',
    },
    example: '{"file_type": "pdf"}',
    inputRequired: true,
  },
  web_search: {
    description: "Search the web. Returns result titles, URLs, and snippets.",
    args: {
      query: "the search query string (required)",
      count: "number of results to return, max 50 (optional, default: 10)",
    },
    example: '{"query": "BRCA1 variant classification guidelines 2025", "count": 5}',
  },
  fetch_url: {
    description: "Fetch and extract the main content from a web page. Returns page title and clean text.",
    args: {
      url: "the full URL to fetch, e.g. https://example.com/article (required)",
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
      throw new Error(`Not logged in. Run "future auth login" first, or set the FUTURE_API_KEY environment variable.`);
    }

    const key = typeof (future as Record<string, unknown>).key === "string"
      ? (future as Record<string, unknown>).key as string
      : undefined;
    if (!key) {
      throw new Error(`Not logged in. Run "future auth login" first, or set the FUTURE_API_KEY environment variable.`);
    }
    return key;
  } catch (err) {
    const testKey = process.env["FUTURE_API_TEST_KEY"];
    if (testKey) return testKey;
    if (isNodeError(err) && err.code === "ENOENT") {
      throw new Error(`Not logged in. Run "future auth login" first, or set the FUTURE_API_KEY environment variable.`);
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
    throw new Error(`code=${code}, message=${message}`);
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

export type ToolsCommand = "list" | "call" | "describe";

export function isToolsCommand(command: string): command is ToolsCommand {
  return command === "list" || command === "call" || command === "describe";
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


// ── Public command entry ────────────────────────────────────────────────────

export async function tools(command: ToolsCommand, args: string[]): Promise<void> {
  if (command === "list") {
    const jsonFlag = args.includes("--json");
    const allTools: Array<{ name: string; description: string; needsInput?: boolean }> = [];

    // Local catalog (browser)
    for (const [name, entry] of Object.entries(BROWSER_TOOL_CATALOG)) {
      allTools.push({ name, description: entry.description });
    }

    // Remote tools from API, prefer local catalog descriptions
    try {
      const apiKey = await loadApiKey();
      const remote = await listRemoteTools(apiKey);
      for (const rt of remote) {
        const local = TOOL_CATALOG[rt.name];
        allTools.push({
          name: rt.name,
          description: local?.description ?? rt.description,
          needsInput: local?.inputRequired,
        });
      }
    } catch (error) {
      if (!jsonFlag) {
        console.error(
          `Remote tools unavailable: ${error instanceof Error ? error.message : String(error)}`,
        );
        console.error("Showing local tools only.\n");
      }
    }

    if (jsonFlag) {
      console.log(JSON.stringify(allTools, null, 2));
    } else {
      const maxName = Math.max(...allTools.map(t => t.name.length), 12);
      for (const t of allTools) {
        const desc = t.description.length > 90 ? t.description.slice(0, 89) + "…" : t.description;
        const hint = t.needsInput ? " [needs --input]" : "";
        console.log(`  ${t.name.padEnd(maxName + 2)} ${desc}${hint}`);
      }
      console.log(`\n${allTools.length} tools available.  Use "future tools describe <name>" for details.`);
    }
    return;
  }

  if (command === "describe") {
    if (args[0] === "--help" || args[0] === "-h") {
      console.log("Usage: future tools describe <tool_name>\n\nShow arguments, flags, and usage example for a tool.");
      return;
    }
    const toolName = args[0];
    if (!toolName) {
      console.error("Usage: future tools describe <tool_name>");
      process.exitCode = 1;
      return;
    }
    const entry = TOOL_CATALOG[toolName];
    if (!entry) {
      // Fallback: try remote tool
      try {
        const apiKey = await loadApiKey();
        const remote = await listRemoteTools(apiKey);
        const found = remote.find(t => t.name === toolName);
        if (found) {
          console.log(`  ${found.name}`);
          console.log(`  ${found.description}`);
          console.log("");
          console.log("  Remote tool — use --args with JSON to call it:");
          console.log(`  future tools call ${found.name} --args '{"param": "value"}'`);
          return;
        }
      } catch { /* ignore */ }
      console.error(`Tool not found: ${toolName}`);
      process.exit(1);
      return;
    }
    console.log(`  ${toolName}`);
    console.log(`  ${entry.description}`);

    // Flags (common to all tools)
    console.log("");
    console.log("  Flags:");
    if (entry.inputRequired) {
      console.log("    --input <path>     Input file");
      if (entry.maskSupported) console.log("    --mask <path>      Optional mask image");
    }
    if (entry.outputSupported) console.log("    --output <path>    Save output to file");
    console.log("    --timeout <secs>   HTTP timeout (default: 60s)");

    // Arguments (tool-specific)
    if (Object.keys(entry.args).length > 0) {
      console.log("");
      console.log("  Arguments (--key value):");
      for (const [name, type] of Object.entries(entry.args)) {
        console.log(`    --${name.padEnd(24)} ${type}`);
      }
    }

    // Example
    const exampleFlags = (() => {
      try {
        const ex = JSON.parse(entry.example) as Record<string, unknown>;
        return Object.keys(ex).map(k => {
          const v = ex[k];
          if (Array.isArray(v)) return `--${k} '${JSON.stringify(v)}'`;
          if (typeof v === "string") return `--${k} "${v}"`;
          return `--${k} ${JSON.stringify(v)}`;
        }).join(" ");
      } catch { return ""; }
    })();
    const inputPart = entry.inputRequired ? "--input <file> " : "";
    console.log("");
    console.log("  Example:");
    console.log(`  future tools call ${toolName} ${inputPart}${exampleFlags}`);
    return;
  }

  if (command === "call") {
    if (args[0] === "--help" || args[0] === "-h") {
      console.log(`Usage: future tools call <tool_name> [--key value...]

Call a tool by name. Use "future tools describe <tool_name>" to see
required arguments, flags, and examples for each tool.`);
      return;
    }
    const toolName = args[0];
    if (!toolName || toolName.startsWith("--")) {
      console.error("Usage: future tools call <tool_name> [--key value...] [--input <path>] [--output <path>] [--timeout <secs>] [--raw]");
      process.exitCode = 1;
      return;
    }

    let toolArgs: Record<string, unknown> = {};
    const stdinFlag = args.includes("--stdin");
    const outputIdx = args.indexOf("--output");
    const outputPath = outputIdx !== -1 && outputIdx + 1 < args.length
      ? args[outputIdx + 1]
      : null;
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
        ? 600_000
        : undefined;

    const rawFlag = args.includes("--raw");

    if (stdinFlag) {
      const chunks: Buffer[] = [];
      for await (const chunk of process.stdin) {
        chunks.push(chunk as Buffer);
      }
      toolArgs = parseToolArgs(Buffer.concat(chunks).toString());
    }

    // Tool arguments: --key value.
    const knownFlags = new Set([
      "--stdin", "--input", "--mask", "--output", "--timeout", "--raw",
    ]);
    for (let i = 1; i < args.length - 1; i++) {
      const arg = args[i];
      if (arg.startsWith("--") && !knownFlags.has(arg)) {
        const val = args[i + 1];
        if (!val.startsWith("--")) {
          toolArgs[arg.slice(2)] = parseCmdValue(val);
          i++;
        }
      }
    }

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

    // Pre-check: for known tools, validate required args and value ranges
    const catalogEntry = TOOL_CATALOG[toolName];
    if (catalogEntry) {
      const missing: string[] = [];
      for (const [name, desc] of Object.entries(catalogEntry.args)) {
        if (desc.includes("required") && !(name in toolArgs)) {
          missing.push(`--${name}`);
        }
      }
      if (missing.length > 0) {
        console.error(`Error: ${toolName} requires: ${missing.join(", ")}`);
        console.error(`Use "future tools describe ${toolName}" for details.`);
        process.exit(1);
      }

      // Validate numeric ranges for known parameters
      const intRange = (key: string, min: number, max: number) => {
        if (key in toolArgs) {
          const v = toolArgs[key];
          if (typeof v !== "number" || !Number.isInteger(v) || v < min || v > max) {
            console.error(`Error: --${key} must be an integer between ${min} and ${max}, got: ${JSON.stringify(v)}`);
            process.exit(1);
          }
        }
      };
      const intMin = (key: string, min: number) => {
        if (key in toolArgs) {
          const v = toolArgs[key];
          if (typeof v !== "number" || !Number.isInteger(v) || v < min) {
            console.error(`Error: --${key} must be a positive integer, got: ${JSON.stringify(v)}`);
            process.exit(1);
          }
        }
      };

      // Validate search_paper queries
      if (toolName === "search_paper" && "queries" in toolArgs) {
        const q = toolArgs["queries"];
        if (!Array.isArray(q) || q.length === 0 || q.some((s: unknown) => typeof s !== "string" || s.trim() === "")) {
          console.error(`Error: --queries must be a non-empty array of non-empty strings`);
          process.exit(1);
        }
      }

      intRange("n", 1, 10);
      intRange("count", 1, 50);
      intMin("max_k", 1);
      intMin("max_tokens", 1);

      // Normalize file_type to lowercase
      if (typeof toolArgs["file_type"] === "string") {
        const ft = (toolArgs["file_type"] as string).toLowerCase();
        if (ft !== "pdf" && ft !== "docx") {
          console.error(`Error: --file_type must be "pdf" or "docx", got: "${toolArgs["file_type"]}"`);
          process.exit(1);
        }
        toolArgs["file_type"] = ft;
      }
    }

    // Validate --timeout (common flag)
    if (timeoutSec < 0) {
      console.error(`Error: --timeout must be >= 1 second, got: ${timeoutSec}`);
      process.exit(1);
    }

    if (isBrowserTool(toolName)) {
      let output: string;
      let exitCode = 0;
      try {
        const result = await callBrowserTool(toolName, toolArgs);
        output = result.structuredContent && Object.keys(result.structuredContent).length > 0
          ? JSON.stringify(result.structuredContent, null, 2)
          : result.text ?? "";
      } catch (error) {
        exitCode = 1;
        output = error instanceof Error ? error.message
          : (typeof error === "object" && error !== null && "message" in error)
            ? String((error as Record<string, unknown>).message)
            : String(error);
      }

      // Browser commands are short-lived subprocesses of the Rust shell tool.
      // Flush synchronously, then close inherited pipes on Windows so the
      // agent can observe completion even if the JS runtime still has handles.
      writeSync(exitCode === 0 ? 1 : 2, `${output}\n`);
      if (process.platform === "win32") {
        try { closeSync(1); } catch { /* already closed */ }
        try { closeSync(2); } catch { /* already closed */ }
      }
      process.exit(exitCode);
    }

    const apiKey = await loadApiKey();
    let result: CallToolResponse;
    try {
      result = await callRemoteTool(apiKey, toolName, toolArgs, timeoutMs);
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      // Give context: which tool, and how to see its arguments
      console.error(`Error calling ${toolName}: ${msg}`);
      console.error(`Use "future tools describe ${toolName}" to see required arguments.`);
      process.exit(1);
    }

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
