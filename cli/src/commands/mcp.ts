// Shared MCP protocol helpers used by tools.ts and skills.ts.
import { readFile } from "node:fs/promises";
import { AUTH_FILE, DEFAULT_API_URL, FUTURE_AUTH_PROVIDER } from "../constants.js";

async function resolveMcpUrl(): Promise<string> {
  let baseUrl = DEFAULT_API_URL;
  try {
    const raw = await readFile(AUTH_FILE, "utf8");
    const auth = JSON.parse(raw) as Record<string, unknown>;
    const future = auth[FUTURE_AUTH_PROVIDER];
    if (future && typeof future === "object") {
      const base_url = (future as Record<string, unknown>).base_url;
      if (typeof base_url === "string") baseUrl = base_url;
    }
  } catch {
    // auth.json not found or unreadable — use default
  }
  return `${baseUrl}/v1/mcp`;
}

export async function mcpUrl(): Promise<string> {
  return resolveMcpUrl();
}

export interface McpResponse {
  body: Record<string, unknown>;
  sessionId: string | null;
}

export async function mcpPost(
  url: string,
  method: string,
  params: Record<string, unknown>,
  apiKey: string,
  sessionId?: string,
  id?: number,
): Promise<McpResponse> {
  const body: Record<string, unknown> = {
    jsonrpc: "2.0",
    method,
    params,
  };
  if (id !== undefined) body.id = id;

  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    Accept: "application/json, text/event-stream",
    Authorization: `Bearer ${apiKey}`,
  };
  if (sessionId) headers["Mcp-Session-Id"] = sessionId;

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 60_000);

  try {
    const response = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
      signal: controller.signal,
    });

    if (!response.ok) {
      const text = await response.text().catch(() => "");
      throw new Error(
        `MCP request failed: HTTP ${response.status}${text ? " — " + text.slice(0, 200) : ""}`
      );
    }

    const sid = response.headers.get("mcp-session-id") ?? undefined;
    const data = await response.text();

    // Parse SSE stream: look for `data:` lines
    for (const line of data.split("\n")) {
      if (line.startsWith("data:")) {
        const p = line.slice(5).trim();
        if (p) {
          try {
            return { body: JSON.parse(p) as Record<string, unknown>, sessionId: sid ?? null };
          } catch {
            throw new Error(`Invalid JSON in SSE: ${p}`);
          }
        }
      }
    }
    return { body: {}, sessionId: sid ?? null };
  } finally {
    clearTimeout(timeout);
  }
}

export function mcpNotify(
  url: string,
  method: string,
  params: Record<string, unknown>,
  apiKey: string,
  sessionId: string,
): Promise<void> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    Accept: "application/json, text/event-stream",
    Authorization: `Bearer ${apiKey}`,
    "Mcp-Session-Id": sessionId,
  };

  return fetch(url, {
    method: "POST",
    headers,
    body: JSON.stringify({ jsonrpc: "2.0", method, params }),
  }).then(
    () => {},
    () => {}, // swallow errors, fire-and-forget
  );
}

export async function initializeSession(apiKey: string): Promise<string> {
  const url = await mcpUrl();
  const { body, sessionId } = await mcpPost(url, "initialize", {
    protocolVersion: "2024-11-05",
    capabilities: {},
    clientInfo: { name: "future", version: "1.0" },
  }, apiKey, undefined, 1);

  if (body.error) throw new Error(`MCP initialize failed: ${JSON.stringify(body.error)}`);
  if (!sessionId) throw new Error("No session ID received from MCP server");

  await mcpNotify(url, "notifications/initialized", {}, apiKey, sessionId);
  return sessionId;
}
