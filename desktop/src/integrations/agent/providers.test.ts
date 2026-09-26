// @vitest-environment jsdom
import type { ProvidersView } from "./providers";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearFutureBalanceCache,
  clearFutureProfileCache,
  deleteCustomProvider,
  FUTURE_PROVIDER_ID,
  getFutureAuthState,
  getFutureBalance,
  getFutureEnvironment,
  getFutureProfile,
  listAgentProviders,
  logoutFutureProvider,
  peekAgentProviders,
  peekFutureBalance,
  peekFutureProfile,
  pollFutureLogin,
  setBuiltinProviderBaseUrl,
  startFutureLogin,
  storeFutureBalance,
  updateBuiltinProvider,
  updateBuiltinProviderKey,
  upsertCustomProvider,
} from "./providers";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => invokeMock(command, args),
}));

const BUILTIN = {
  id: "future",
  name: "FutureOS",
  baseUrl: "https://api.example.com",
  hasApiKey: true,
  modelCount: 3,
  requiresBaseUrl: false,
};

const EMPTY_VIEW: ProvidersView = { builtin: [BUILTIN], custom: [] };

/** Commands invoked since the mock was last reset, in order. */
function commands(): string[] {
  return invokeMock.mock.calls.map(([command]) => command as string);
}

/** Count `futureos:future-auth-changed` events seen while `run` executes. */
async function countAuthEvents(run: () => Promise<void>): Promise<number> {
  let seen = 0;
  const listener = () => {
    seen += 1;
  };
  window.addEventListener("futureos:future-auth-changed", listener);
  try {
    await run();
  }
  finally {
    window.removeEventListener("futureos:future-auth-changed", listener);
  }
  return seen;
}

describe("agent provider catalogue", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    clearFutureProfileCache();
    clearFutureBalanceCache();
  });

  it("lists providers through the native command and remembers the response", async () => {
    invokeMock.mockResolvedValue(EMPTY_VIEW);

    await expect(listAgentProviders()).resolves.toBe(EMPTY_VIEW);
    expect(invokeMock).toHaveBeenCalledWith("list_agent_providers", undefined);
    expect(peekAgentProviders()).toBe(EMPTY_VIEW);
  });

  it("refetches on every list call and replaces the cached view", async () => {
    const next: ProvidersView = { builtin: [], custom: [] };
    invokeMock.mockResolvedValueOnce(EMPTY_VIEW).mockResolvedValueOnce(next);

    await listAgentProviders();
    await expect(listAgentProviders()).resolves.toBe(next);
    expect(commands()).toEqual(["list_agent_providers", "list_agent_providers"]);
    expect(peekAgentProviders()).toBe(next);
  });

  it("keeps the previous cache when the backend rejects", async () => {
    invokeMock.mockResolvedValueOnce(EMPTY_VIEW);
    await listAgentProviders();

    invokeMock.mockRejectedValueOnce("agent offline");
    await expect(listAgentProviders()).rejects.toThrow("agent offline");
    expect(peekAgentProviders()).toBe(EMPTY_VIEW);
  });

  it("wraps custom provider mutations in an `input` argument", async () => {
    const input = { id: "custom-1", name: "Local", api: "openai", baseUrl: "http://x", models: [], create: true };
    invokeMock.mockResolvedValue(EMPTY_VIEW);

    await upsertCustomProvider(input);
    expect(invokeMock).toHaveBeenCalledWith("upsert_custom_provider", { input });

    await updateBuiltinProvider({ id: "anthropic", baseUrl: "http://y", updateApiKey: true });
    expect(invokeMock).toHaveBeenCalledWith("update_builtin_provider", {
      input: { id: "anthropic", baseUrl: "http://y", updateApiKey: true },
    });

    await setBuiltinProviderBaseUrl({ id: "anthropic", baseUrl: "" });
    expect(invokeMock).toHaveBeenCalledWith("set_builtin_provider_base_url", {
      input: { id: "anthropic", baseUrl: "" },
    });

    await deleteCustomProvider("custom-1");
    expect(invokeMock).toHaveBeenCalledWith("delete_custom_provider", { id: "custom-1" });
    expect(peekAgentProviders()).toBe(EMPTY_VIEW);
  });

  it("hand-editing the FutureOS key clears the account caches and announces the change", async () => {
    invokeMock.mockResolvedValue(EMPTY_VIEW);
    invokeMock.mockResolvedValueOnce({ email: "a@b.c", userId: "u", emailVerified: true, createdAt: null });
    await getFutureProfile();
    invokeMock.mockResolvedValueOnce({ credits: 5 });
    await getFutureBalance();
    expect(peekFutureProfile()).not.toBeNull();
    expect(peekFutureBalance()).not.toBeNull();

    const events = await countAuthEvents(async () => {
      await updateBuiltinProviderKey({ id: FUTURE_PROVIDER_ID, apiKey: "k" });
    });

    expect(events).toBe(1);
    expect(peekFutureProfile()).toBeNull();
    expect(peekFutureBalance()).toBeNull();
  });

  it("editing another provider's key leaves the FutureOS account caches alone", async () => {
    invokeMock.mockResolvedValue(EMPTY_VIEW);
    invokeMock.mockResolvedValueOnce({ email: "a@b.c", userId: "u", emailVerified: true, createdAt: null });
    await getFutureProfile();

    const events = await countAuthEvents(async () => {
      await updateBuiltinProviderKey({ id: "anthropic", apiKey: "k" });
    });

    expect(events).toBe(0);
    expect(peekFutureProfile()).not.toBeNull();
  });

  it("starts a device login and returns the server payload verbatim", async () => {
    const start = {
      userCode: "ABCD-1234",
      verificationUri: "https://example.com/device",
      verificationUriComplete: "https://example.com/device?code=ABCD-1234",
      interval: 5,
      expiresIn: 600,
      deviceCode: "device-code",
    };
    invokeMock.mockResolvedValue(start);

    await expect(startFutureLogin()).resolves.toBe(start);
    expect(invokeMock).toHaveBeenCalledWith("start_future_login", undefined);
  });

  it("a non-authorized poll neither invalidates the catalogue nor emits an event", async () => {
    invokeMock.mockResolvedValueOnce(EMPTY_VIEW);
    await listAgentProviders();

    invokeMock.mockResolvedValueOnce({ status: "slow_down", retryAfterSeconds: 12 });
    const events = await countAuthEvents(async () => {
      await expect(pollFutureLogin("device-code")).resolves.toEqual({ status: "slow_down", retryAfterSeconds: 12 });
    });

    expect(events).toBe(0);
    expect(peekAgentProviders()).toBe(EMPTY_VIEW);
    expect(invokeMock).toHaveBeenLastCalledWith("poll_future_login", { deviceCode: "device-code" });
  });

  it("an authorized poll drops the catalogue cache and announces the login", async () => {
    invokeMock.mockResolvedValueOnce(EMPTY_VIEW);
    await listAgentProviders();

    invokeMock.mockResolvedValueOnce({ status: "authorized" });
    const events = await countAuthEvents(async () => {
      await pollFutureLogin("device-code");
    });

    expect(events).toBe(1);
    expect(peekAgentProviders()).toBeNull();
  });

  it("signing out drops the cached profile and announces the change", async () => {
    invokeMock.mockResolvedValue(EMPTY_VIEW);
    invokeMock.mockResolvedValueOnce({ email: "a@b.c", userId: "u", emailVerified: true, createdAt: null });
    await getFutureProfile();

    const events = await countAuthEvents(async () => {
      await logoutFutureProvider();
    });

    expect(events).toBe(1);
    expect(peekFutureProfile()).toBeNull();
    expect(invokeMock).toHaveBeenCalledWith("logout_future_provider", undefined);
  });

  it("exposes the auth state and platform environment from the backend", async () => {
    const auth = { status: "signed_out", profile: null };
    const environment = { environment: "custom", platformUrl: "https://platform.example.com" };
    invokeMock.mockResolvedValueOnce(auth).mockResolvedValueOnce(environment);

    await expect(getFutureAuthState()).resolves.toBe(auth);
    await expect(getFutureEnvironment()).resolves.toBe(environment);
    expect(commands()).toEqual(["get_future_auth_state", "get_future_environment"]);
  });
});

describe("account caches", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    clearFutureProfileCache();
    clearFutureBalanceCache();
  });

  it("serves the balance from the session cache until forced", async () => {
    invokeMock.mockResolvedValueOnce({ credits: 1 });
    await expect(getFutureBalance()).resolves.toEqual({ credits: 1 });
    await expect(getFutureBalance()).resolves.toEqual({ credits: 1 });
    expect(commands()).toEqual(["get_future_balance"]);

    invokeMock.mockResolvedValueOnce({ credits: 2 });
    await expect(getFutureBalance(true)).resolves.toEqual({ credits: 2 });
    expect(commands()).toEqual(["get_future_balance", "get_future_balance"]);
    expect(peekFutureBalance()).toEqual({ credits: 2 });
  });

  it("a rejected balance fetch is not cached, and the next call retries", async () => {
    invokeMock.mockRejectedValueOnce(new Error("signed out"));
    await expect(getFutureBalance()).rejects.toThrow("signed out");
    expect(peekFutureBalance()).toBeNull();

    invokeMock.mockResolvedValueOnce({ credits: 7 });
    await expect(getFutureBalance()).resolves.toEqual({ credits: 7 });
    expect(peekFutureBalance()).toEqual({ credits: 7 });
  });

  it("accepts a pushed balance and drops it on explicit clear", () => {
    expect(peekFutureBalance()).toBeNull();
    storeFutureBalance({ credits: 42 });
    expect(peekFutureBalance()).toEqual({ credits: 42 });
    clearFutureBalanceCache();
    expect(peekFutureBalance()).toBeNull();
  });

  it("zero credits survive the cache (a falsy amount is still a value)", async () => {
    storeFutureBalance({ credits: 0 });
    expect(peekFutureBalance()).toEqual({ credits: 0 });
    await expect(getFutureBalance()).resolves.toEqual({ credits: 0 });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("serves the profile from the session cache until forced", async () => {
    const profile = { email: "a@b.c", userId: "u", emailVerified: false, createdAt: null };
    invokeMock.mockResolvedValueOnce(profile);
    await expect(getFutureProfile()).resolves.toBe(profile);
    await expect(getFutureProfile()).resolves.toBe(profile);
    expect(commands()).toEqual(["get_future_profile"]);

    const updated = { ...profile, emailVerified: true };
    invokeMock.mockResolvedValueOnce(updated);
    await expect(getFutureProfile(true)).resolves.toBe(updated);
    expect(peekFutureProfile()).toBe(updated);
  });

  it("a rejected profile fetch is not cached", async () => {
    invokeMock.mockRejectedValueOnce("not logged in");
    await expect(getFutureProfile()).rejects.toThrow("not logged in");
    expect(peekFutureProfile()).toBeNull();
  });
});
