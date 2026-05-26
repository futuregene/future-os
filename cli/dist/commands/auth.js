import { spawn } from "node:child_process";
import { chmod, mkdir, readFile, writeFile } from "node:fs/promises";
import { platform as osPlatform } from "node:os";
import { dirname } from "node:path";
import { AUTH_FILE, DEFAULT_API_URL, FUTURE_AUTH_PROVIDER, MODELS_FILE, } from "../constants.js";
import { ensureRecordProperty, getRecord, isNodeError, isRecord } from "../utils/object.js";
import { trimTrailingSlash } from "../utils/string.js";
import { sleep } from "../utils/time.js";
export async function login() {
    const authFile = await loadAuthFile();
    const apiUrl = resolveApiUrl(authFile);
    const device = await post(apiUrl, "/oauth/device/code", {
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
        const body = (await response.json());
        if (response.ok) {
            const token = body;
            await saveAuth(authFile, token);
            console.log(`Saved Future API key to ${AUTH_FILE}`);
            const modelCount = await syncFutureModels(apiUrl, token.api_key);
            console.log(`Synced ${modelCount} Future model(s) to ${MODELS_FILE}`);
            return;
        }
        const error = body;
        if (error.error === "authorization_pending" || error.error === "slow_down") {
            process.stdout.write(".");
            continue;
        }
        throw new Error(error.message);
    }
    throw new Error("Device authorization expired.");
}
export async function status() {
    try {
        const authFile = await loadAuthFile();
        const auth = getFutureAuthEntry(authFile);
        if (!auth?.key) {
            console.log("Not logged in.");
            return;
        }
        console.log(`API: ${auth.base_url ?? DEFAULT_API_URL}`);
    }
    catch {
        console.log("Not logged in.");
    }
}
export async function logout() {
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
async function post(apiUrl, path, body) {
    const response = await fetch(`${apiUrl}${path}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
    });
    const data = (await response.json());
    if (!response.ok) {
        throw new Error(data.message ?? `Request failed with ${response.status}`);
    }
    return data;
}
async function loadAuthFile() {
    let contents;
    try {
        contents = await readFile(AUTH_FILE, "utf8");
    }
    catch (error) {
        if (isNodeError(error) && error.code === "ENOENT") {
            return {};
        }
        throw error;
    }
    const parsed = JSON.parse(contents);
    if (!isRecord(parsed)) {
        throw new Error(`${AUTH_FILE} must contain a JSON object.`);
    }
    return parsed;
}
function resolveApiUrl(authFile) {
    const auth = getFutureAuthEntry(authFile);
    return trimTrailingSlash(auth?.base_url ?? DEFAULT_API_URL);
}
async function saveAuth(authFile, token) {
    const current = getFutureAuthEntry(authFile) ?? {};
    authFile[FUTURE_AUTH_PROVIDER] = {
        ...current,
        type: current.type ?? "api_key",
        key: token.api_key,
    };
    await writeAuthFile(authFile);
}
async function writeAuthFile(authFile) {
    await mkdir(dirname(AUTH_FILE), { recursive: true });
    await writeFile(AUTH_FILE, `${JSON.stringify(authFile, null, 2)}\n`, { mode: 0o600 });
    await chmod(AUTH_FILE, 0o600);
}
async function syncFutureModels(apiUrl, apiKey) {
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
async function removeFutureModelsProvider() {
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
async function fetchFutureModels(apiUrl, apiKey) {
    const response = await fetch(`${apiUrl}/openai/v1/models`, {
        headers: { Authorization: `Bearer ${apiKey}` },
    });
    const body = (await response.json());
    if (!response.ok) {
        throw new Error(body.message ?? `Failed to fetch models with ${response.status}`);
    }
    if (!Array.isArray(body.data)) {
        throw new Error("Future models response must contain a data array.");
    }
    const models = [];
    for (const item of body.data) {
        const model = getRecord(item);
        if (!model || typeof model.id !== "string") {
            continue;
        }
        const id = model.id;
        const ownedBy = typeof model.owned_by === "string" ? model.owned_by : undefined;
        models.push({
            id,
            name: ownedBy ? `${ownedBy}: ${id}` : id,
        });
    }
    return models;
}
async function loadModelsFile() {
    let contents;
    try {
        contents = await readFile(MODELS_FILE, "utf8");
    }
    catch (error) {
        if (isNodeError(error) && error.code === "ENOENT") {
            return {};
        }
        throw error;
    }
    const parsed = JSON.parse(contents);
    if (!isRecord(parsed)) {
        throw new Error(`${MODELS_FILE} must contain a JSON object.`);
    }
    return parsed;
}
async function writeModelsFile(modelsFile) {
    await mkdir(dirname(MODELS_FILE), { recursive: true });
    await writeFile(MODELS_FILE, `${JSON.stringify(modelsFile, null, 2)}\n`, { mode: 0o600 });
    await chmod(MODELS_FILE, 0o600);
}
function getFutureAuthEntry(authFile) {
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
async function openBrowser(url) {
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
