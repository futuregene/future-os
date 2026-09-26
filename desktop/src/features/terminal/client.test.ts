// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  connectTicket,
  connectUrl,
  createTerminal,
  listShells,
  listTerminals,
  removeTerminal,
  resetTerminalServerCache,
  terminalInfo,
  terminalServer,
  updateTerminal,
} from "./client";

const invokeCommand = vi.fn<(command: string) => Promise<unknown>>();

vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (command: string) => invokeCommand(command),
}));

const SERVER = { url: "http://127.0.0.1:41234", token: "t".repeat(64), port: 41234, maxSessions: 32 };

interface Call {
  body: unknown;
  headers: Record<string, string>;
  method: string;
  url: string;
}

let calls: Call[] = [];

function installFetch(handler: (call: Call) => { body: unknown; status: number; text?: string }) {
  calls = [];
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: Call = {
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
      headers: (init?.headers ?? {}) as Record<string, string>,
      method: init?.method ?? "GET",
      url: String(input),
    };
    calls.push(call);
    const { body, status, text } = handler(call);
    return new Response(text === undefined ? JSON.stringify(body) : text, {
      status,
      headers: { "content-type": "application/json" },
    });
  }));
}

beforeEach(() => {
  invokeCommand.mockReset();
  invokeCommand.mockResolvedValue(SERVER);
  resetTerminalServerCache();
});

afterEach(() => {
  vi.unstubAllGlobals();
  resetTerminalServerCache();
});

describe("terminal endpoint resolution", () => {
  it("asks the backend once and reuses the endpoint", async () => {
    installFetch(() => ({ body: [], status: 200 }));
    const [first, second] = await Promise.all([terminalServer(), terminalServer()]);
    expect(first).toEqual(SERVER);
    expect(second).toBe(first);
    expect(invokeCommand).toHaveBeenCalledTimes(1);
    expect(invokeCommand).toHaveBeenCalledWith("terminal_server_info");
  });

  it("does not cache a failed lookup, so a later attempt can succeed", async () => {
    installFetch(() => ({ body: [], status: 200 }));
    invokeCommand.mockRejectedValueOnce(new Error("listener not bound"));
    await expect(terminalServer()).rejects.toThrow("listener not bound");

    // The failure is retried rather than remembered.
    await expect(terminalServer()).resolves.toEqual(SERVER);
    expect(invokeCommand).toHaveBeenCalledTimes(2);

    resetTerminalServerCache();
    await terminalServer();
    expect(invokeCommand).toHaveBeenCalledTimes(3);
  });
});

describe("terminal control routes", () => {
  it("addresses each session route and carries the bearer token", async () => {
    installFetch(call => ({
      body: call.url.endsWith("/shells")
        ? [{ name: "bash", path: "/bin/bash" }]
        : { id: "term_1", threadId: "thread-1", title: "Terminal 1", status: "running" },
      status: 200,
    }));

    await listTerminals("thread-1");
    await createTerminal({ threadId: "thread-1", title: "Terminal 1" });
    await terminalInfo("term_1");
    await updateTerminal("term_1", { cols: 120, rows: 40 });
    await removeTerminal("term_1");
    await listShells();

    expect(calls.map(call => `${call.method} ${call.url.replace(SERVER.url, "")}`)).toEqual([
      "GET /terminal?threadId=thread-1",
      "POST /terminal",
      "GET /terminal/term_1",
      "PATCH /terminal/term_1",
      "DELETE /terminal/term_1",
      "GET /terminal/shells",
    ]);
    // Every control call is authenticated with the per-session secret.
    expect(calls.every(call => call.headers.authorization === `Bearer ${SERVER.token}`)).toBe(true);
    // A JSON body (and its content type) only where one is sent.
    expect(calls[1]!.body).toEqual({ threadId: "thread-1", title: "Terminal 1" });
    expect(calls[1]!.headers["content-type"]).toBe("application/json");
    expect(calls[3]!.body).toEqual({ cols: 120, rows: 40 });
    expect(calls[0]!.body).toBeUndefined();
    expect(calls[0]!.headers["content-type"]).toBeUndefined();
    expect(calls[4]!.body).toBeUndefined();
  });

  it("percent-encodes an id or thread id that is not URL-safe", async () => {
    installFetch(() => ({ body: {}, status: 200 }));
    await listTerminals("thread/one two");
    await terminalInfo("term/1 2");
    await updateTerminal("term/1 2", { cols: 80 });
    await removeTerminal("term/1 2");
    await connectTicket("term/1 2");
    expect(calls.map(call => call.url)).toEqual([
      `${SERVER.url}/terminal?threadId=thread%2Fone%20two`,
      `${SERVER.url}/terminal/term%2F1%202`,
      `${SERVER.url}/terminal/term%2F1%202`,
      `${SERVER.url}/terminal/term%2F1%202`,
      `${SERVER.url}/terminal/term%2F1%202/connect-token`,
    ]);
  });

  it("issues a one-time connect ticket", async () => {
    installFetch(() => ({ body: { expiresIn: 30, ticket: "ticket-abc" }, status: 200 }));
    await expect(connectTicket("term_1")).resolves.toBe("ticket-abc");
    expect(calls[0]!.method).toBe("POST");
    expect(calls[0]!.url).toBe(`${SERVER.url}/terminal/term_1/connect-token`);
  });

  it("reads a body-less 200 as undefined instead of throwing", async () => {
    // A 204-style empty response (or a proxy stripping the body) is not JSON.
    installFetch(() => ({ body: undefined, status: 200, text: "" }));
    await expect(removeTerminal("term_1")).resolves.toBeUndefined();

    // Garbage that is not JSON at all is treated the same way.
    installFetch(() => ({ body: undefined, status: 200, text: "<html>not json</html>" }));
    await expect(listShells()).resolves.toBeUndefined();
  });
});

describe("terminal control failures", () => {
  it("surfaces the server's stable error code and status", async () => {
    installFetch(() => ({
      body: { error: { code: "TERMINAL_NOT_FOUND", message: "TERMINAL_NOT_FOUND: gone" } },
      status: 404,
    }));
    const failure = await terminalInfo("missing").catch(error => error);
    expect(failure).toBeInstanceOf(Error);
    expect(failure).toMatchObject({
      code: "TERMINAL_NOT_FOUND",
      message: "TERMINAL_NOT_FOUND: gone",
      status: 404,
    });
    expect((failure as Error).name).toBe("TerminalApiError");
  });

  it("falls back to a generic code and message for an unstructured failure", async () => {
    installFetch(() => ({ body: { nope: true }, status: 502 }));
    await expect(listShells()).rejects.toMatchObject({
      code: "REQUEST_FAILED",
      message: "terminal request failed (502)",
      status: 502,
    });
  });

  it("falls back for a non-JSON failure body too", async () => {
    installFetch(() => ({ body: undefined, status: 500, text: "<html>bad gateway</html>" }));
    await expect(listShells()).rejects.toMatchObject({
      code: "REQUEST_FAILED",
      message: "terminal request failed (500)",
      status: 500,
    });
  });

  it("propagates a transport-level failure unchanged", async () => {
    invokeCommand.mockRejectedValue(new Error("terminal_server_info failed"));
    await expect(listTerminals("thread-1")).rejects.toThrow("terminal_server_info failed");
  });
});

describe("terminal websocket url", () => {
  it("converts the control base to ws and carries the cursor and ticket", () => {
    const url = new URL(connectUrl(SERVER, "term_1", 128, "ticket-abc"));
    expect(url.protocol).toBe("ws:");
    expect(url.pathname).toBe("/terminal/term_1/connect");
    expect(url.searchParams.get("cursor")).toBe("128");
    expect(url.searchParams.get("ticket")).toBe("ticket-abc");
  });

  it("tails from the end when no cursor is applied yet", () => {
    const url = new URL(connectUrl(SERVER, "term_1", undefined, "ticket-abc"));
    expect(url.searchParams.get("cursor")).toBe("-1");
    // A zero cursor is a real cursor, not a missing one.
    expect(new URL(connectUrl(SERVER, "term_1", 0, "t")).searchParams.get("cursor")).toBe("0");
  });

  it("upgrades https to wss and escapes the session id", () => {
    const url = connectUrl({ ...SERVER, url: "https://127.0.0.1:8443" }, "term/1", 5, "t");
    expect(url.startsWith("wss://127.0.0.1:8443/terminal/term%2F1/connect?")).toBe(true);
  });
});
