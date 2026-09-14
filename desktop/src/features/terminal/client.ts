import type {
  CreateTerminalInput,
  TerminalInfo,
  TerminalServerHandle,
  TerminalShell,
  UpdateTerminalInput,
} from "./types";
/**
 * Control-route client for the embedded terminal server.
 *
 * The webview asks the Rust side once for the loopback URL and per-process
 * secret (`terminal_server_info`), then talks HTTP directly. Terminal bytes
 * travel over the WebSocket in `TerminalView`, never through Tauri IPC.
 */
import { invokeCommand } from "../../integrations/tauri/invoke";

/** A control-route failure, carrying the server's stable error code. */
export class TerminalApiError extends Error {
  readonly code: string;
  /**
   * True only for `CWD_INVALID`: retrying with `cwdPolicy: "homeConfirmed"`
   * is a decision the user has to make, not something the client may assume.
   */
  readonly allowsHomeFallback: boolean;
  readonly status: number;

  constructor(code: string, message: string, allowsHomeFallback: boolean, status: number) {
    super(message);
    this.name = "TerminalApiError";
    this.code = code;
    this.allowsHomeFallback = allowsHomeFallback;
    this.status = status;
  }
}

let cachedServer: Promise<TerminalServerHandle> | null = null;

/**
 * Resolve the terminal server endpoint, once per session.
 *
 * The listener is bound during app setup, so a failure here means the feature
 * is unavailable in this build (or the request raced startup); the caller
 * surfaces it and stops, rather than retrying forever.
 */
export function terminalServer(): Promise<TerminalServerHandle> {
  cachedServer ??= invokeCommand<TerminalServerHandle>("terminal_server_info").catch((error: unknown) => {
    // Do not cache a failure: a later attempt (after startup settles) may work.
    cachedServer = null;
    throw error;
  });
  return cachedServer;
}

/** Test seam: forget the cached endpoint. */
export function resetTerminalServerCache(): void {
  cachedServer = null;
}

async function request<T>(
  path: string,
  init: { method: "GET" | "POST" | "PATCH" | "DELETE"; body?: unknown } = { method: "GET" },
): Promise<T> {
  const server = await terminalServer();
  const response = await fetch(`${server.url}${path}`, {
    method: init.method,
    headers: {
      authorization: `Bearer ${server.token}`,
      ...(init.body === undefined ? {} : { "content-type": "application/json" }),
    },
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
  });

  const text = await response.text();
  let parsed: unknown;
  try {
    parsed = text ? JSON.parse(text) : undefined;
  }
  catch {
    parsed = undefined;
  }

  if (!response.ok) {
    const error = (parsed as { error?: { code?: string; message?: string; allowsHomeFallback?: boolean } } | undefined)?.error;
    throw new TerminalApiError(
      error?.code ?? "REQUEST_FAILED",
      error?.message ?? `terminal request failed (${response.status})`,
      error?.allowsHomeFallback === true,
      response.status,
    );
  }
  return parsed as T;
}

/** Sessions for one conversation (running plus recently exited). */
export function listTerminals(threadId: string): Promise<TerminalInfo[]> {
  return request<TerminalInfo[]>(`/terminal?threadId=${encodeURIComponent(threadId)}`);
}

/** Start a shell for a conversation. The backend resolves the directory. */
export function createTerminal(input: CreateTerminalInput): Promise<TerminalInfo> {
  return request<TerminalInfo>("/terminal", { method: "POST", body: input });
}

export function updateTerminal(id: string, input: UpdateTerminalInput): Promise<TerminalInfo> {
  return request<TerminalInfo>(`/terminal/${encodeURIComponent(id)}`, { method: "PATCH", body: input });
}

export function removeTerminal(id: string): Promise<{ removed: boolean }> {
  return request<{ removed: boolean }>(`/terminal/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export function terminalInfo(id: string): Promise<TerminalInfo> {
  return request<TerminalInfo>(`/terminal/${encodeURIComponent(id)}`);
}

export function listShells(): Promise<TerminalShell[]> {
  return request<TerminalShell[]>("/terminal/shells");
}

/**
 * One-time ticket for the WebSocket handshake (a browser cannot set an
 * Authorization header on a handshake).
 */
export async function connectTicket(id: string): Promise<string> {
  const issued = await request<{ ticket: string; expiresIn: number }>(
    `/terminal/${encodeURIComponent(id)}/connect-token`,
    { method: "POST" },
  );
  return issued.ticket;
}

/**
 * WebSocket URL for a session. `cursor` is the absolute output offset the
 * client already applied; `-1` tails from the current end.
 */
export function connectUrl(server: TerminalServerHandle, id: string, cursor: number | undefined, ticket: string): string {
  const base = server.url.replace(/^http/, "ws");
  const query = new URLSearchParams();
  query.set("cursor", cursor === undefined ? "-1" : String(cursor));
  query.set("ticket", ticket);
  return `${base}/terminal/${encodeURIComponent(id)}/connect?${query.toString()}`;
}
