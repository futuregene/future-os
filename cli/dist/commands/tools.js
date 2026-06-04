import { readFile } from "node:fs/promises";
import { request as httpRequest } from "node:http";
import { join } from "node:path";
import { homedir } from "node:os";
import { getRecord, isRecord, isNodeError } from "../utils/object.js";
// ── Constants ────────────────────────────────────────────────────────────────
const AUTH_FILE = join(homedir(), ".future", "agent", "auth.json");
const FUTURE_PROVIDER = "future";
const DEFAULT_MCP_URL = "http://localhost:7003/mcp";
export const TOOL_CATALOG = {
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
    knowledge_searcher: {
        description: "Search rare disease knowledge sources and extract requested information.",
        args: {
            information_to_extract: "string",
            domains: 'string[] (default: ["all"])',
        },
        example: '{"information_to_extract": "inheritance pattern and typical age of onset", "domains": ["all"]}',
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
        description: "Search variants using raremcp variant search with filter parameters.",
        args: {
            gene: "string",
            consequence: "string",
            frequency_max: "number",
            cadd_score_min: "number",
        },
        example: '{"gene": "FBN1", "consequence": "missense", "frequency_max": 0.01}',
    },
    get_paper: {
        description: "Get paper content from raremcp by paper identifier (PMID, DOI, etc.).",
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
        description: "Search web pages through raremcp.",
        args: {
            query: "string (required)",
            limit: "int",
        },
        example: '{"query": "BRCA1 variant classification guidelines 2025"}',
    },
};
// ── Auth ─────────────────────────────────────────────────────────────────────
export async function loadApiKey() {
    const envKey = process.env["FUTURE_API_KEY"];
    if (envKey)
        return envKey;
    try {
        const raw = await readFile(AUTH_FILE, "utf8");
        const auth = JSON.parse(raw);
        if (!isRecord(auth))
            throw new Error("auth.json must be a JSON object");
        const future = auth[FUTURE_PROVIDER];
        if (!isRecord(future))
            throw new Error(`No "${FUTURE_PROVIDER}" provider in auth.json`);
        const key = typeof future.key === "string"
            ? future.key
            : undefined;
        if (!key)
            throw new Error(`No API key for "${FUTURE_PROVIDER}" in auth.json`);
        return key;
    }
    catch (err) {
        const testKey = process.env["FUTURE_API_TEST_KEY"];
        if (testKey)
            return testKey;
        if (isNodeError(err) && err.code === "ENOENT") {
            throw new Error(`No API key. Run "future auth login" first, or set FUTURE_API_KEY.`);
        }
        throw err;
    }
}
// ── MCP protocol ─────────────────────────────────────────────────────────────
function mcpPost(url, method, params, apiKey, sessionId, id) {
    const body = {
        jsonrpc: "2.0",
        method,
        params,
    };
    if (id !== undefined)
        body.id = id;
    const payload = JSON.stringify(body);
    const urlObj = new URL(url);
    return new Promise((resolve, reject) => {
        const req = httpRequest({
            hostname: urlObj.hostname,
            port: urlObj.port || 80,
            path: urlObj.pathname + urlObj.search,
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                "Accept": "application/json, text/event-stream",
                "Authorization": `Bearer ${apiKey}`,
                ...(sessionId ? { "Mcp-Session-Id": sessionId } : {}),
                "Content-Length": Buffer.byteLength(payload).toString(),
            },
            agent: false,
        }, (res) => {
            const sid = res.headers["mcp-session-id"];
            let data = "";
            res.on("data", (chunk) => { data += chunk.toString(); });
            res.on("end", () => {
                // Parse SSE
                for (const line of data.split("\n")) {
                    if (line.startsWith("data:")) {
                        const payload = line.slice(5).trim();
                        if (payload) {
                            try {
                                resolve({ body: JSON.parse(payload), sessionId: sid ?? null });
                                return;
                            }
                            catch {
                                reject(new Error(`Invalid JSON in SSE: ${payload}`));
                                return;
                            }
                        }
                    }
                }
                resolve({ body: {}, sessionId: sid ?? null });
            });
        });
        req.on("error", reject);
        req.write(payload);
        req.end();
    });
}
async function mcpNotify(url, method, params, apiKey, sessionId) {
    const payload = JSON.stringify({ jsonrpc: "2.0", method, params });
    const urlObj = new URL(url);
    return new Promise((resolve) => {
        const req = httpRequest({
            hostname: urlObj.hostname,
            port: urlObj.port || 80,
            path: urlObj.pathname + urlObj.search,
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                Accept: "application/json, text/event-stream",
                Authorization: `Bearer ${apiKey}`,
                "Mcp-Session-Id": sessionId,
                "Content-Length": Buffer.byteLength(payload).toString(),
            },
            agent: false,
        }, () => resolve());
        req.on("error", () => resolve());
        req.write(payload);
        req.end();
    });
}
async function initializeSession(apiKey) {
    const mcpUrl = process.env["FUTURE_MCP_URL"] || DEFAULT_MCP_URL;
    const { body, sessionId } = await mcpPost(mcpUrl, "initialize", {
        protocolVersion: "2024-11-05",
        capabilities: {},
        clientInfo: { name: "future", version: "1.0" },
    }, apiKey, undefined, 1);
    if (body.error)
        throw new Error(`MCP initialize failed: ${JSON.stringify(body.error)}`);
    if (!sessionId)
        throw new Error("No session ID received from MCP server");
    await mcpNotify(mcpUrl, "notifications/initialized", {}, apiKey, sessionId);
    return sessionId;
}
// ── Tool operations ──────────────────────────────────────────────────────────
async function listTools(apiKey) {
    const mcpUrl = process.env["FUTURE_MCP_URL"] || DEFAULT_MCP_URL;
    const sessionId = await initializeSession(apiKey);
    const { body } = await mcpPost(mcpUrl, "tools/list", {}, apiKey, sessionId, 2);
    if (body.error)
        throw new Error(`tools/list failed: ${JSON.stringify(body.error)}`);
    const result = getRecord(body.result);
    const tools = Array.isArray(result?.tools) ? result.tools : [];
    return tools.filter(isRecord).map((t) => {
        const record = t;
        return {
            name: typeof record.name === "string" ? record.name : "unknown",
            description: typeof record.description === "string" ? record.description : "",
        };
    });
}
async function callTool(apiKey, name, args) {
    const mcpUrl = process.env["FUTURE_MCP_URL"] || DEFAULT_MCP_URL;
    const sessionId = await initializeSession(apiKey);
    const { body } = await mcpPost(mcpUrl, "tools/call", {
        name,
        arguments: args,
    }, apiKey, sessionId, 2);
    if (body.error)
        throw new Error(`tools/call failed: ${JSON.stringify(body.error)}`);
    const result = getRecord(body.result);
    const content = Array.isArray(result?.content) ? result.content : [];
    const texts = content
        .filter(isRecord)
        .map((block) => {
        const b = block;
        if (b.type === "text")
            return String(b.text ?? "");
        if (b.type === "resource")
            return JSON.stringify(b.resource, null, 2);
        return JSON.stringify(b, null, 2);
    });
    return texts.join("\n");
}
export function isToolsCommand(command) {
    return command === "list" || command === "call";
}
export async function tools(command, args) {
    const apiKey = await loadApiKey();
    if (command === "list") {
        const jsonFlag = args.includes("--json");
        const tools = await listTools(apiKey);
        if (jsonFlag) {
            console.log(JSON.stringify(tools, null, 2));
        }
        else {
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
            console.error("Usage: future tools call <tool_name> [--args '<json>' | --stdin]");
            process.exitCode = 1;
            return;
        }
        let toolArgs = {};
        const argsIdx = args.indexOf("--args");
        const stdinFlag = args.includes("--stdin");
        if (stdinFlag) {
            // Read from stdin
            const chunks = [];
            for await (const chunk of process.stdin) {
                chunks.push(chunk);
            }
            toolArgs = JSON.parse(Buffer.concat(chunks).toString());
        }
        else if (argsIdx !== -1 && argsIdx + 1 < args.length) {
            toolArgs = JSON.parse(args[argsIdx + 1]);
        }
        const result = await callTool(apiKey, toolName, toolArgs);
        console.log(result);
        return;
    }
}
