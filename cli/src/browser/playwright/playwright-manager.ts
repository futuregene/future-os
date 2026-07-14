/**
 * PlaywrightManager — wraps browser launch/status behind BrowserManager interface.
 */
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";
import { createConnection } from "node:net";

import type { BrowserKind, BrowserProtocol, BrowserConnectionConfig } from "../types.js";
import type { BrowserManager, BrowserLaunchOptions, InternalBrowserStartResult, BrowserStatusResult } from "../backend.js";
import { findBrowser } from "../browser-discovery.js";
import { BrowserNotFoundError } from "../errors.js";
import { getBrowserDir } from "../browser-state.js";

const BROWSER_DIR = getBrowserDir();
const DEFAULT_PROFILE_DIR = join(BROWSER_DIR, "profile");
const DEFAULT_PORT = 9222;

export class PlaywrightManager implements BrowserManager {
  readonly kind: BrowserKind;
  readonly protocol: BrowserProtocol = "cdp";

  constructor(kind: BrowserKind) {
    this.kind = kind;
  }

  async start(options: BrowserLaunchOptions = {}): Promise<InternalBrowserStartResult> {
    const requestedPort = options.port ?? DEFAULT_PORT;
    const port = await resolvePort(requestedPort);
    const endpoint = `http://127.0.0.1:${port}`;

    // Check if already reachable
    if (await endpointReachable(endpoint)) {
      return {
        connection: {
          protocol: "cdp",
          browserKind: this.kind as "chrome" | "edge" | "chromium",
          endpoint,
        },
        launcher: "existing",
        port,
        status: "already_running",
      };
    }

    const discovered = findBrowser(options.executablePath);
    if (!discovered) {
      throw new BrowserNotFoundError("Pass executablePath to specify the browser location.");
    }

    const profileDir = options.profileDir ?? (
      port === requestedPort ? DEFAULT_PROFILE_DIR : join(BROWSER_DIR, `profile-${port}`)
    );
    const url = options.url ?? "about:blank";
    await mkdir(profileDir, { recursive: true });
    await mkdir(BROWSER_DIR, { recursive: true });

    const chromeArgs = [
      `--remote-debugging-port=${port}`,
      `--user-data-dir=${profileDir}`,
      "--no-first-run",
      "--no-default-browser-check",
      url,
    ];

    const child = spawn(discovered.executablePath, chromeArgs, {
      detached: true,
      stdio: "ignore",
    });
    child.unref();

    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      if (await endpointReachable(endpoint)) {
        return {
          connection: {
            protocol: "cdp",
            browserKind: discovered.kind,
            endpoint,
          },
          launcher: discovered.executablePath,
          profileDir,
          port,
          status: "started",
        };
      }
      await sleep(250);
    }

    // Started but not yet reachable
    return {
      connection: {
        protocol: "cdp",
        browserKind: discovered.kind,
        endpoint,
      },
      launcher: discovered.executablePath,
      profileDir,
      port,
      status: "started",
    };
  }

  async status(connection: BrowserConnectionConfig): Promise<BrowserStatusResult> {
    if (connection.protocol !== "cdp") {
      return { endpoint: connection.endpoint, reachable: false, error: "Not a CDP endpoint" };
    }
    try {
      const response = await fetch(new URL("/json/version", connection.endpoint));
      if (!response.ok) {
        return { endpoint: connection.endpoint, reachable: false, error: `HTTP ${response.status}` };
      }
      const version = await response.json() as unknown;
      return { endpoint: connection.endpoint, reachable: true, version };
    } catch (error) {
      return {
        endpoint: connection.endpoint,
        reachable: false,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }
}

// ── Helpers ──────────────────────────────────────────────────────────

async function endpointReachable(endpoint: string): Promise<boolean> {
  try {
    const response = await fetch(new URL("/json/version", endpoint), {
      signal: AbortSignal.timeout(1000),
    });
    return response.ok;
  } catch {
    return false;
  }
}

async function resolvePort(requestedPort: number): Promise<number> {
  const endpoint = `http://127.0.0.1:${requestedPort}`;
  if (await endpointReachable(endpoint)) return requestedPort;
  if (!await portHasListener(requestedPort)) return requestedPort;

  for (let port = requestedPort + 1; port < requestedPort + 50; port++) {
    if (!await portHasListener(port)) return port;
  }
  throw new Error(`No available browser debugging port found near ${requestedPort}.`);
}

function portHasListener(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    socket.setTimeout(500);
    socket.once("connect", () => { socket.destroy(); resolve(true); });
    socket.once("timeout", () => { socket.destroy(); resolve(false); });
    socket.once("error", () => resolve(false));
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
