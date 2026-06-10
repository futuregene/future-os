// Shared MCP protocol helpers used by tools.ts and skills.ts.
import { request as httpRequest } from "node:http";
import { DEFAULT_API_URL } from "../constants.js";

function resolveMcpUrl(): string {
  if (process.env["FUTURE_MCP_URL"]) return process.env["FUTURE_MCP_URL"];
  const apiBase = process.env["FUTURE_API_BASE"] ?? DEFAULT_API_URL;
  return `${apiBase}/mcp`;
}

export function mcpUrl(): string {
  return resolveMcpUrl();
}

export interface McpResponse {
  body: Record<string, unknown>;
  sessionId: string | null;
}

export function mcpPost(
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

  const payload = JSON.stringify(body);
  const urlObj = new URL(url);

  return new Promise((resolve, reject) => {
    const req = httpRequest(
      {
        hostname: urlObj.hostname,
        port: urlObj.port || 80,
        path: urlObj.pathname + urlObj.search,
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "application/json, text/event-stream",
          Authorization: `Bearer ${apiKey}`,
          ...(sessionId ? { "Mcp-Session-Id": sessionId } : {}),
          "Content-Length": Buffer.byteLength(payload).toString(),
        },
        agent: false,
      },
      (res) => {
        const sid = res.headers["mcp-session-id"] as string | undefined;
        let data = "";
        res.on("data", (chunk: Buffer) => { data += chunk.toString(); });
        res.on("end", () => {
          for (const line of data.split("\n")) {
            if (line.startsWith("data:")) {
              const p = line.slice(5).trim();
              if (p) {
                try {
                  resolve({ body: JSON.parse(p) as Record<string, unknown>, sessionId: sid ?? null });
                  return;
                } catch {
                  reject(new Error(`Invalid JSON in SSE: ${p}`));
                  return;
                }
              }
            }
          }
          resolve({ body: {}, sessionId: sid ?? null });
        });
      },
    );
    req.on("error", reject);
    req.write(payload);
    req.end();
  });
}

export function mcpNotify(
  url: string,
  method: string,
  params: Record<string, unknown>,
  apiKey: string,
  sessionId: string,
): Promise<void> {
  const payload = JSON.stringify({ jsonrpc: "2.0", method, params });
  const urlObj = new URL(url);

  return new Promise((resolve) => {
    const req = httpRequest(
      {
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
      },
      () => resolve(),
    );
    req.on("error", () => resolve());
    req.write(payload);
    req.end();
  });
}

export async function initializeSession(apiKey: string): Promise<string> {
  const url = mcpUrl();
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
