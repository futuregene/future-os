import { readFile } from "node:fs/promises";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
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
  case_searcher: {
    description: "Search similar rare disease cases by patient phenotypes.",
    args: {
      patient_phenotypes: "string[] (required)",
      topk: "int",
      exclude_case_id: "string|null",
    },
    example: '{"patient_phenotypes": ["joint hypermobility", "lens dislocation"], "topk": 5}',
  },
  disease_searcher: {
    description: "Search disease information by disease identifier (MONDO, OMIM, ORPHA, etc.).",
    args: {
      id: "string (required)",
      domain: "string (default: all)",
      only_show_hpo_with_frequent_or_more: "bool",
    },
    example: '{"id": "MONDO:0009061", "domain": "all"}',
  },
  normalize_disease: {
    description: "Normalize a disease name into standard disease identifiers (MONDO, OMIM, ORPHA).",
    args: { disease: "string (required)" },
    example: '{"disease": "cystic fibrosis"}',
  },
  gene_getter: {
    description: "Get gene information by gene identifier or symbol.",
    args: { gene_id_or_symbol: "string (required)" },
    example: '{"gene_id_or_symbol": "FBN1"}',
  },
  extract_phenotype: {
    description: "Extract patient phenotypes (HPO terms) from free-text clinical descriptions.",
    args: {
      patient_info: "string (required)",
      is_fetal: "bool",
      extract_family_history: "bool",
    },
    example: '{"patient_info": "Patient presents with joint hypermobility and arachnodactyly."}',
  },
  get_phenotype_by_hpo_id: {
    description: "Get detailed phenotype information by HPO identifier.",
    args: { hpo_id: "string (required)" },
    example: '{"hpo_id": "HP:0001166"}',
  },
  search_paper: {
    description: "Search academic papers and extract requested information.",
    args: {
      information_to_extract: "string",
    },
    example: '{"information_to_extract": "inheritance pattern and typical age of onset"}',
  },
  phenotype_analyzer: {
    description: "Analyze phenotype evidence for rare disease interpretation. Produces differential diagnosis with scored disease matches.",
    args: {
      hpo_list: "string[]",
      domain: "string[]",
      num_differential_diseases: "int",
    },
    example: '{"hpo_list": ["HP:0001382", "HP:0001166", "HP:0000486"], "num_differential_diseases": 10}',
  },
  think: {
    description: "Run structured thinking for complex multi-step rare disease analysis.",
    args: {
      thought: "string (required)",
      thoughtNumber: "int (required)",
      nextThoughtNeeded: "bool",
    },
    example: '{"thought": "Considering phenotype matches...", "thoughtNumber": 1, "nextThoughtNeeded": true}',
  },
  variant_getter: {
    description: "Get variant information by variant identifier.",
    args: {
      variant_id: "string (required)",
      assembly: "string (default: hg38)",
    },
    example: '{"variant_id": "15-48702977-G-A", "assembly": "hg38"}',
  },
  variant_searcher: {
    description: "Search variants with filter parameters.",
    args: {
      gene: "string",
      consequence: "string",
      frequency_max: "number",
      cadd_score_min: "number",
    },
    example: '{"gene": "FBN1", "consequence": "missense", "frequency_max": 0.01}',
  },
  get_paper: {
    description: "Get full paper content by identifier (PMID, DOI). Returns structured Paper object with metadata (title, authors, journal, year, DOI) and complete body_text.",
    args: {
      paper_id: "string (required)",
      max_k: "int",
    },
    example: '{"paper_id": "PMID:12345678"}',
  },
  get_page: {
    description: "Fetch and extract content from a web page by URL.",
    args: {
      url: "string (required)",
      max_k: "int",
    },
    example: '{"url": "https://pubmed.ncbi.nlm.nih.gov/12345678/"}',
  },
  search_page: {
    description: "Search web pages.",
    args: {
      query: "string (required)",
      limit: "int",
    },
    example: '{"query": "BRCA1 variant classification guidelines 2025"}',
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
    description: "Edit an existing image using text instructions.",
    args: {
      prompt: "string (required)",
      image_b64: "string (required, base64-encoded image)",
      mask_b64: "string (optional, base64-encoded mask)",
      size: 'string (default: "1024x1024")',
      quality: 'string (default: "medium")',
    },
    example: '{"prompt": "Convert to watercolor painting", "image_b64": "<base64>"}',
  },
  read_image: {
    description: "Read and analyze an image. Provide a base64-encoded image and a question — supports OCR, object recognition, and visual Q&A.",
    args: {
      image_b64: "string (required, base64-encoded image)",
      question: "string (required, e.g. 'Extract text' or 'Describe this image')",
      mime_type: 'string (default: "image/png")',
      max_tokens: "integer (default: 2000)",
    },
    example: '{"image_b64": "<base64>", "question": "What text is in this image?"}',
  },
  parse_doc: {
    description: "Parse PDF and Word documents into markdown. Upload a base64-encoded document and get structured markdown with text, tables, and formulas preserved.",
    args: {
      doc_b64: "string (required, base64-encoded document content)",
    },
    example: '{"doc_b64": "<base64>"}',
  },
  web_search: {
    description: "Search the web. Returns titles, links, and snippets.",
    args: {
      query: "string (required)",
      count: "integer (default: 10, max: 20)",
      offset: "integer (default: 0)",
    },
    example: '{"query": "BRCA1 variant classification guidelines 2025", "count": 5, "offset": 0}',
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
    if (!isRecord(future)) throw new Error(`No "${FUTURE_PROVIDER}" provider in auth.json`);

    const key = typeof (future as Record<string, unknown>).key === "string"
      ? (future as Record<string, unknown>).key as string
      : undefined;
    if (!key) throw new Error(`No API key for "${FUTURE_PROVIDER}" in auth.json`);
    return key;
  } catch (err) {
    const testKey = process.env["FUTURE_API_TEST_KEY"];
    if (testKey) return testKey;
    if (isNodeError(err) && err.code === "ENOENT") {
      throw new Error(`No API key. Run "future auth login" first, or set FUTURE_API_KEY.`);
    }
    throw err;
  }
}

// ── Tool operations ──────────────────────────────────────────────────────────

async function listRemoteTools(apiKey: string): Promise<Array<{ name: string; description: string }>> {
  const sessionId = await initializeSession(apiKey);
  const { body } = await mcpPost(mcpUrl(), "tools/list", {}, apiKey, sessionId, 2);

  if (body.error) throw new Error(`tools/list failed: ${JSON.stringify(body.error)}`);
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

async function callRemoteTool(apiKey: string, name: string, args: Record<string, unknown>): Promise<CallToolResponse> {
  const sessionId = await initializeSession(apiKey);
  const { body } = await mcpPost(mcpUrl(), "tools/call", {
    name,
    arguments: args,
  }, apiKey, sessionId, 2);

  if (body.error) throw new Error(`tools/call failed: ${JSON.stringify(body.error)}`);

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

/** Extract b64_json from structured_content.images[N].b64_json */
function extractImageB64(structured: Record<string, unknown> | null): string | null {
  if (!structured) return null;
  const images = structured["images"];
  if (!Array.isArray(images) || images.length === 0) return null;
  const first = images[0];
  if (!isRecord(first)) return null;
  const b64 = first["b64_json"];
  return typeof b64 === "string" ? b64 : null;
}

// ── Public command ───────────────────────────────────────────────────────────

export type ToolsCommand = "list" | "call";

export function isToolsCommand(command: string): command is ToolsCommand {
  return command === "list" || command === "call";
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
      console.error("Usage: future tools call <tool_name> [--args '<json>' | --stdin] [--output <path>]");
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

    if (stdinFlag) {
      // Read from stdin
      const chunks: Buffer[] = [];
      for await (const chunk of process.stdin) {
        chunks.push(chunk as Buffer);
      }
      toolArgs = JSON.parse(Buffer.concat(chunks).toString());
    } else if (argsIdx !== -1 && argsIdx + 1 < args.length) {
      toolArgs = JSON.parse(args[argsIdx + 1]);
    }

    // Resolve image_path / doc_path → base64 before sending to API
    toolArgs = await resolveLocalPaths(toolArgs);

    if (isBrowserTool(toolName)) {
      try {
        const result = await callBrowserTool(toolName, toolArgs);
        const output = result.structuredContent && Object.keys(result.structuredContent).length > 0
          ? JSON.stringify(result.structuredContent, null, 2)
          : result.text ?? "";
        await writeStdout(`${output}\n`);
        process.exit(0);
      } catch (error) {
        await writeStderr(`${error instanceof Error ? error.message : String(error)}\n`);
        process.exit(1);
      }
    }

    const apiKey = await loadApiKey();
    const result = await callRemoteTool(apiKey, toolName, toolArgs);

    // Output structured content as JSON when available (primary data for agent consumption).
    // Fall back to text content otherwise.
    if (result.structuredContent && Object.keys(result.structuredContent).length > 0) {
      console.log(JSON.stringify(result.structuredContent, null, 2));
    } else {
      console.log(result.text);
    }

    // Handle image output
    if (outputPath) {
      const b64 = extractImageB64(result.structuredContent);
      if (b64) {
        const buf = Buffer.from(b64, "base64");
        await writeFile(outputPath, buf);
        console.log(`\nImage saved to: ${outputPath}`);
      } else {
        console.error("\nWarning: --output specified but no b64_json found in response.");
        process.exitCode = 1;
      }
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
