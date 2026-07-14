/**
 * Safari browser manager — launches safaridriver, creates WebDriver sessions.
 *
 * safaridriver is at /usr/bin/safaridriver on macOS.
 * User must first enable remote automation:
 *   Safari → Develop → Allow Remote Automation
 *   or: safaridriver --enable
 */
import { spawn } from "node:child_process";
import { platform } from "node:os";
import { createConnection } from "node:net";

import type { BrowserKind, BrowserProtocol, BrowserConnectionConfig } from "../types.js";
import type {
  BrowserManager,
  BrowserLaunchOptions,
  InternalBrowserStartResult,
  BrowserStatusResult,
} from "../backend.js";
import { BrowserPermissionError } from "../errors.js";
import { WebDriverClient } from "./webdriver-client.js";

const SAFARIDRIVER_PATH = "/usr/bin/safaridriver";

export class SafariManager implements BrowserManager {
  readonly kind: BrowserKind = "safari";
  readonly protocol: BrowserProtocol = "webdriver";

  async start(options: BrowserLaunchOptions = {}): Promise<InternalBrowserStartResult> {
    if (platform() !== "darwin") {
      throw new Error("Safari is only available on macOS.");
    }

    const requestedPort = options.port ?? 4444;
    const port = await this.resolvePort(requestedPort);
    const driverEndpoint = `http://127.0.0.1:${port}`;

    // Check if safaridriver is already running on this port
    if (await this.endpointReachable(driverEndpoint)) {
      // Try to create a session — may fail if remote automation is not enabled
      let sessionId: string;
      try {
        const client = new WebDriverClient(driverEndpoint);
        sessionId = await client.createSession();
      } catch (e) {
        throw this.translateError(e);
      }

      return {
        connection: {
          protocol: "webdriver",
          browserKind: "safari",
          endpoint: driverEndpoint,
          sessionId,
          driverPid: undefined,
        },
        launcher: SAFARIDRIVER_PATH,
        port,
        status: "already_running",
      };
    }

    // Launch safaridriver
    const child = spawn(SAFARIDRIVER_PATH, ["--port", String(port)], {
      detached: true,
      stdio: "ignore",
    });
    child.unref();
    const pid = child.pid!;

    // Wait for it to be ready
    const deadline = Date.now() + 10_000;
    let lastError: string | undefined;
    while (Date.now() < deadline) {
      if (await this.endpointReachable(driverEndpoint)) {
        // Create a WebDriver session
        const client = new WebDriverClient(driverEndpoint);
        let sessionId: string;
        try {
          sessionId = await client.createSession();
        } catch (e) {
          throw this.translateError(e);
        }

        return {
          connection: {
            protocol: "webdriver",
            browserKind: "safari",
            endpoint: driverEndpoint,
            sessionId,
            driverPid: pid,
          },
          launcher: SAFARIDRIVER_PATH,
          port,
          status: "started",
        };
      }
      await sleep(250);
    }

    // Started but unreachable
    throw new Error(
      `safaridriver did not respond at ${driverEndpoint} within 10s. ` +
      `Last error: ${lastError ?? "timeout"}`,
    );
  }

  async status(connection: BrowserConnectionConfig): Promise<BrowserStatusResult> {
    if (connection.protocol !== "webdriver") {
      return { endpoint: connection.endpoint, reachable: false, error: "Not a WebDriver endpoint" };
    }
    try {
      const response = await fetch(`${connection.endpoint}/status`, {
        signal: AbortSignal.timeout(2000),
      });
      if (!response.ok) {
        return { endpoint: connection.endpoint, reachable: false, error: `HTTP ${response.status}` };
      }
      const data = await response.json() as unknown;
      return { endpoint: connection.endpoint, reachable: true, version: data };
    } catch (e) {
      return {
        endpoint: connection.endpoint,
        reachable: false,
        error: e instanceof Error ? e.message : String(e),
      };
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────

  /**
   * Translate WebDriver/launch errors into user-actionable messages.
   * Permission errors (safaridriver --enable required) get a clear
   * single-line remedy.
   */
  private translateError(e: unknown): Error {
    const msg = (e instanceof Error ? e.message : String(e)).toLowerCase();
    if (msg.includes("allow remote automation") || msg.includes("remote automation")) {
      return new BrowserPermissionError(
        "Safari",
        "safaridriver --enable",
      );
    }
    if (msg.includes("session not created")) {
      return new BrowserPermissionError(
        "Safari",
        "safaridriver --enable",
      );
    }
    return e instanceof Error ? e : new Error(String(e));
  }

  private async endpointReachable(url: string): Promise<boolean> {
    try {
      const resp = await fetch(`${url}/status`, { signal: AbortSignal.timeout(1000) });
      return resp.ok;
    } catch {
      return false;
    }
  }

  private async resolvePort(requestedPort: number): Promise<number> {
    if (!await this.portInUse(requestedPort)) return requestedPort;
    for (let port = requestedPort + 1; port < requestedPort + 50; port++) {
      if (!await this.portInUse(port)) return port;
    }
    throw new Error(`No available port found near ${requestedPort}`);
  }

  private portInUse(port: number): Promise<boolean> {
    return new Promise((resolve) => {
      const socket = createConnection({ host: "127.0.0.1", port });
      socket.setTimeout(500);
      socket.once("connect", () => { socket.destroy(); resolve(true); });
      socket.once("timeout", () => { socket.destroy(); resolve(false); });
      socket.once("error", () => resolve(false));
    });
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
