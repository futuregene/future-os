import { spawn } from "node:child_process";
import { chmod, mkdir, readFile, writeFile } from "node:fs/promises";
import { platform as osPlatform } from "node:os";
import { dirname } from "node:path";

import { AUTH_FILE, FUTURE_AUTH_PROVIDER } from "../constants.js";
import { isNodeError, isRecord } from "../utils/object.js";
import { getPlatformUrl } from "../utils/platform.js";
import { sleep } from "../utils/time.js";

type DeviceCodeResponse = {
  device_code: string;
  user_code: string;
  verification_uri: string;
  verification_uri_complete: string;
  expires_in: number;
  interval: number;
};

type DeviceTokenResponse = {
  api_key: string;
  api_key_id: string;
  token_type: "api_key";
};

type DeviceErrorResponse = {
  error: string;
  message: string;
};

type FutureAuthEntry = {
  type?: string;
  key?: string;
  platform_base_url?: string;
};

type AuthFile = Record<string, unknown>;

export async function login(platformUrlOverride?: string): Promise<void> {
  const authFile = await loadAuthFile();
  const platformUrl = await getPlatformUrl(platformUrlOverride);
  const device = await post<DeviceCodeResponse>(platformUrl, "/client/v1/oauth/device/code", {
    client_name: "Future OS CLI",
  });

  const verificationUrl = device.verification_uri_complete || device.verification_uri;
  const opened = await openBrowser(verificationUrl);
  console.log(opened ? "Opened Future Platform Console:" : "Open this URL in your browser:");
  console.log(`  ${verificationUrl}`);
  console.log("");
  console.log("Sign in and authorize this device code:");
  console.log(`  ${device.user_code}`);
  console.log("");
  console.log("Waiting for authorization...");

  const startedAt = Date.now();
  while (Date.now() - startedAt < device.expires_in * 1000) {
    await sleep(device.interval * 1000);
    const response = await fetch(`${platformUrl}/client/v1/oauth/device/token`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ device_code: device.device_code }),
    });

    const body = (await response.json()) as DeviceTokenResponse | DeviceErrorResponse;
    if (response.ok) {
      const token = body as DeviceTokenResponse;
      await saveAuth(authFile, token, platformUrl);
      console.log(`Saved Future API key to ${AUTH_FILE}`);
      return;
    }

    const error = body as DeviceErrorResponse;
    if (error.error === "authorization_pending" || error.error === "slow_down") {
      process.stdout.write(".");
      continue;
    }

    throw new Error(error.message);
  }

  throw new Error("Device authorization expired.");
}

export async function status(): Promise<void> {
  try {
    const authFile = await loadAuthFile();
    const auth = getFutureAuthEntry(authFile);
    if (!auth?.key) {
      console.log("Not logged in.");
      return;
    }

    const platformUrl = auth.platform_base_url ?? await getPlatformUrl();
    console.log(`Platform: ${platformUrl}`);
    console.log(`API: ${platformUrl}/api/v1`);
  } catch {
    console.log("Not logged in.");
  }
}

export async function logout(): Promise<void> {
  const authFile = await loadAuthFile();
  const current = getFutureAuthEntry(authFile);
  let removedKey = false;

  if (current?.key) {
    const next = { ...current };
    delete next.key;
    authFile[FUTURE_AUTH_PROVIDER] = next;

    await writeAuthFile(authFile);
    console.log(`Removed Future API key from ${AUTH_FILE}`);
    removedKey = true;
  }

  if (!removedKey) {
    console.log("Not logged in.");
  }
}

async function post<T>(apiUrl: string, path: string, body: unknown): Promise<T> {
  const response = await fetch(`${apiUrl}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const data = (await response.json()) as { message?: string };
  if (!response.ok) {
    throw new Error(data.message ?? `Request failed with ${response.status}`);
  }
  return data as T;
}

async function loadAuthFile(): Promise<AuthFile> {
  let contents: string;
  try {
    contents = await readFile(AUTH_FILE, "utf8");
  } catch (error) {
    if (isNodeError(error) && error.code === "ENOENT") {
      return {};
    }
    throw error;
  }

  const parsed = JSON.parse(contents) as unknown;
  if (!isRecord(parsed)) {
    throw new Error(`${AUTH_FILE} must contain a JSON object.`);
  }
  return parsed;
}

async function saveAuth(authFile: AuthFile, token: DeviceTokenResponse, platformUrl: string): Promise<void> {
  const current = getFutureAuthEntry(authFile) ?? {};
  authFile[FUTURE_AUTH_PROVIDER] = {
    ...current,
    type: current.type ?? "api_key",
    key: token.api_key,
    platform_base_url: platformUrl,
  } satisfies FutureAuthEntry;

  await writeAuthFile(authFile);
}

async function writeAuthFile(authFile: AuthFile): Promise<void> {
  await mkdir(dirname(AUTH_FILE), { recursive: true });
  await writeFile(AUTH_FILE, `${JSON.stringify(authFile, null, 2)}\n`, { mode: 0o600 });
  await chmod(AUTH_FILE, 0o600);
}

function getFutureAuthEntry(authFile: AuthFile): FutureAuthEntry | undefined {
  const value = authFile[FUTURE_AUTH_PROVIDER];
  if (!isRecord(value)) {
    return undefined;
  }

  return {
    ...value,
    type: typeof value.type === "string" ? value.type : undefined,
    key: typeof value.key === "string" ? value.key : undefined,
    platform_base_url: typeof value.platform_base_url === "string" ? value.platform_base_url : undefined,
  };
}

async function openBrowser(url: string): Promise<boolean> {
  const platform = osPlatform();
  const command = platform === "darwin" ? "open" : platform === "win32" ? "cmd" : "xdg-open";
  const args = platform === "win32" ? ["/c", "start", "", url] : [url];

  return new Promise((resolve) => {
    const child = spawn(command, args, {
      detached: true,
      stdio: "ignore",
    });
    child.once("error", () => resolve(false));
    child.once("spawn", () => {
      child.unref();
      resolve(true);
    });
  });
}
