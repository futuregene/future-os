import { spawn } from "node:child_process";
import { chmod, mkdir, readFile, writeFile } from "node:fs/promises";
import { platform as osPlatform } from "node:os";
import { dirname } from "node:path";

import {
  AUTH_FILE,
  DEFAULT_API_URL,
  FUTURE_AUTH_PROVIDER,
  MODELS_FILE,
} from "../constants.js";
import { ensureRecordProperty, getRecord, isNodeError, isRecord } from "../utils/object.js";
import { trimTrailingSlash } from "../utils/string.js";
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

type AuthFile = Record<string, unknown>;

type FutureAuthEntry = {
  type?: string;
  key?: string;
  base_url?: string;
};

type ModelConfig = {
  id: string;
  name?: string;
  reasoning?: boolean;
  modalities?: string[];
  contextWindow?: number;
  maxTokens?: number;
  cost?: {
    input?: number;
    output?: number;
    cache_read?: number;
  };
};

type ModelsFile = Record<string, unknown>;

export async function login(): Promise<void> {
  const authFile = await loadAuthFile();
  const apiUrl = resolveApiUrl(authFile);
  const device = await post<DeviceCodeResponse>(apiUrl, "/oauth/device/code", {
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
    const response = await fetch(`${apiUrl}/oauth/device/token`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ device_code: device.device_code }),
    });

    const body = (await response.json()) as DeviceTokenResponse | DeviceErrorResponse;
    if (response.ok) {
      const token = body as DeviceTokenResponse;
      await saveAuth(authFile, token);
      console.log(`Saved Future API key to ${AUTH_FILE}`);
      const modelCount = await syncFutureModels(apiUrl, token.api_key);
      console.log(`Synced ${modelCount} Future model(s) to ${MODELS_FILE}`);
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

    console.log(`API: ${auth.base_url ?? DEFAULT_API_URL}`);
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

  const removedProvider = await removeFutureModelsProvider();
  if (removedProvider) {
    console.log(`Removed Future models provider from ${MODELS_FILE}`);
  }

  if (!removedKey && !removedProvider) {
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

function resolveApiUrl(authFile: AuthFile): string {
  const auth = getFutureAuthEntry(authFile);
  return trimTrailingSlash(auth?.base_url ?? DEFAULT_API_URL);
}

async function saveAuth(authFile: AuthFile, token: DeviceTokenResponse): Promise<void> {
  const current = getFutureAuthEntry(authFile) ?? {};
  authFile[FUTURE_AUTH_PROVIDER] = {
    ...current,
    type: current.type ?? "api_key",
    key: token.api_key,
  } satisfies FutureAuthEntry;

  await writeAuthFile(authFile);
}

async function writeAuthFile(authFile: AuthFile): Promise<void> {
  await mkdir(dirname(AUTH_FILE), { recursive: true });
  await writeFile(AUTH_FILE, `${JSON.stringify(authFile, null, 2)}\n`, { mode: 0o600 });
  await chmod(AUTH_FILE, 0o600);
}

async function syncFutureModels(apiUrl: string, apiKey: string): Promise<number> {
  const models = await fetchFutureModels(apiUrl, apiKey);
  const modelsFile = await loadModelsFile();
  const providers = ensureRecordProperty(modelsFile, "providers");
  providers[FUTURE_AUTH_PROVIDER] = {
    ...getRecord(providers[FUTURE_AUTH_PROVIDER]),
    api: "openai",
    baseUrl: `${apiUrl}/openai/v1`,
    models,
  };

  await writeModelsFile(modelsFile);
  return models.length;
}

async function removeFutureModelsProvider(): Promise<boolean> {
  const modelsFile = await loadModelsFile();
  const providers = getRecord(modelsFile.providers);
  if (!providers || !(FUTURE_AUTH_PROVIDER in providers)) {
    return false;
  }

  delete providers[FUTURE_AUTH_PROVIDER];
  modelsFile.providers = providers;
  await writeModelsFile(modelsFile);
  return true;
}

async function fetchFutureModels(apiUrl: string, apiKey: string): Promise<ModelConfig[]> {
  const response = await fetch(`${apiUrl}/openai/v1/models`, {
    headers: { Authorization: `Bearer ${apiKey}` },
  });
  const body = (await response.json()) as unknown;
  if (!response.ok) {
    const message = getRecord(body)?.message;
    throw new Error(
      typeof message === "string" ? message : `Failed to fetch models with ${response.status}`,
    );
  }

  const data = Array.isArray(body) ? body : getRecord(body)?.data;
  if (!Array.isArray(data)) {
    throw new Error("Future models response must be an array or contain a data array.");
  }

  const models: ModelConfig[] = [];
  for (const item of data) {
    const model = getRecord(item);
    if (!model || typeof model.id !== "string") {
      continue;
    }

    const id = model.id;
    models.push(toAgentModelConfig(id, model));
  }

  return models;
}

function toAgentModelConfig(id: string, model: Record<string, unknown>): ModelConfig {
  const ownedBy = typeof model.owned_by === "string" ? model.owned_by : undefined;
  const name = typeof model.name === "string" && model.name.trim() ? model.name : undefined;
  const supportedParameters = stringArray(model.supported_parameters);
  const contextWindow = firstNumber(model.contextWindow, model.context_length);
  const maxTokens = firstNumber(
    model.maxTokens,
    model.max_tokens,
    model.maxOutputTokens,
    model.max_output_tokens,
    model.maxCompletionTokens,
    model.max_completion_tokens,
    getRecord(model.top_provider)?.max_completion_tokens,
  );
  const modalities = modalitiesFromModel(model);
  const cost = costFromModel(model);

  return {
    id,
    name: name ?? (ownedBy ? `${ownedBy}: ${id}` : id),
    reasoning: supportsReasoning(supportedParameters),
    ...(modalities.length > 0 ? { modalities } : {}),
    ...(contextWindow !== undefined ? { contextWindow } : {}),
    ...(maxTokens !== undefined ? { maxTokens } : {}),
    ...(cost ? { cost } : {}),
  };
}

function modalitiesFromModel(model: Record<string, unknown>): string[] {
  const explicit = stringArray(model.modalities);
  if (explicit.length > 0) {
    return explicit;
  }

  const input = getRecord(model.modalities)?.input;
  const inputModalities = stringArray(input);
  if (inputModalities.length > 0) {
    return inputModalities;
  }

  const architecture = getRecord(model.architecture);
  const modality = architecture?.modality;
  if (typeof modality !== "string" || !modality.trim()) {
    return [];
  }

  const inputSide = modality.split("->", 1)[0] ?? modality;
  return inputSide
    .split("+")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
}

function costFromModel(model: Record<string, unknown>): ModelConfig["cost"] | undefined {
  const directCost = getRecord(model.cost);
  if (directCost) {
    const input = firstNumber(directCost.input, directCost.prompt);
    const output = firstNumber(directCost.output, directCost.completion);
    const cacheRead = firstNumber(
      directCost.cache_read,
      directCost.cacheRead,
      directCost.input_cache_read,
    );
    return compactCost(input, output, cacheRead);
  }

  const pricing = getRecord(model.pricing);
  if (!pricing) {
    return undefined;
  }

  return compactCost(
    perTokenPriceToPerMillion(pricing.prompt),
    perTokenPriceToPerMillion(pricing.completion),
    perTokenPriceToPerMillion(pricing.input_cache_read),
  );
}

function compactCost(
  input: number | undefined,
  output: number | undefined,
  cacheRead: number | undefined,
): ModelConfig["cost"] | undefined {
  const cost: NonNullable<ModelConfig["cost"]> = {};
  if (input !== undefined) cost.input = input;
  if (output !== undefined) cost.output = output;
  if (cacheRead !== undefined) cost.cache_read = cacheRead;
  return Object.keys(cost).length > 0 ? cost : undefined;
}

function supportsReasoning(supportedParameters: string[]): boolean {
  return supportedParameters.some(
    (parameter) => parameter === "reasoning" || parameter === "include_reasoning",
  );
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

function firstNumber(...values: unknown[]): number | undefined {
  for (const value of values) {
    const number = numberFromUnknown(value);
    if (number !== undefined) {
      return number;
    }
  }
  return undefined;
}

function numberFromUnknown(value: unknown): number | undefined {
  const number =
    typeof value === "number" ? value : typeof value === "string" ? Number(value) : NaN;
  return Number.isFinite(number) ? number : undefined;
}

function perTokenPriceToPerMillion(value: unknown): number | undefined {
  const price = numberFromUnknown(value);
  return price === undefined ? undefined : price * 1_000_000;
}

async function loadModelsFile(): Promise<ModelsFile> {
  let contents: string;
  try {
    contents = await readFile(MODELS_FILE, "utf8");
  } catch (error) {
    if (isNodeError(error) && error.code === "ENOENT") {
      return {};
    }
    throw error;
  }

  const parsed = JSON.parse(contents) as unknown;
  if (!isRecord(parsed)) {
    throw new Error(`${MODELS_FILE} must contain a JSON object.`);
  }
  return parsed;
}

async function writeModelsFile(modelsFile: ModelsFile): Promise<void> {
  await mkdir(dirname(MODELS_FILE), { recursive: true });
  await writeFile(MODELS_FILE, `${JSON.stringify(modelsFile, null, 2)}\n`, { mode: 0o600 });
  await chmod(MODELS_FILE, 0o600);
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
    base_url: typeof value.base_url === "string" ? value.base_url : undefined,
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
