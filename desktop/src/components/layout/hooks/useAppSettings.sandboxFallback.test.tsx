import type { SandboxAvailability } from "../../../integrations/agent/useSandboxAvailability";
// @vitest-environment jsdom
/**
 * The sandbox-unavailable fallback: a machine that cannot run the sandbox tier
 * must fall back to manual approval, exactly once.
 *
 * Its own file because the mock of `useSandboxAvailability` has to *report* the
 * unavailable probe, while the sibling suite pins it available.
 *
 * What keeps the count at one is the optimistic update inside `changeSettings`
 * (the tier is "manual" before the write even starts), not the `sandboxFallbackRef`
 * guard: StrictMode's effect replay reads the *latest* render, in which the
 * requirement is already false. These tests therefore pin the optimistic update —
 * drop it and the replay writes twice.
 */
import type { AppSettings } from "../../../integrations/storage/appSettings";
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { useSandboxAvailability } from "../../../integrations/agent/useSandboxAvailability";
import { DEFAULT_APP_SETTINGS, getAppSettings, updateAppSettings } from "../../../integrations/storage/appSettings";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useAppSettings } from "./useAppSettings";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const availability: SandboxAvailability = {
  available: false,
  definitive: true,
  resolved: true,
  code: "no_sandbox",
};

vi.mock("../../../lib/useTauriEvent", () => ({ useTauriEvent: vi.fn() }));
vi.mock("../../../integrations/agent/useSandboxAvailability", async importOriginal => ({
  ...await importOriginal<typeof import("../../../integrations/agent/useSandboxAvailability")>(),
  useSandboxAvailability: vi.fn(),
}));
vi.mock("../../../integrations/storage/appSettings", async importOriginal => ({
  ...await importOriginal<typeof import("../../../integrations/storage/appSettings")>(),
  getAppSettings: vi.fn(),
  updateAppSettings: vi.fn(),
}));

let stored: AppSettings = { ...DEFAULT_APP_SETTINGS, approvalTier: "sandbox" };

beforeEach(() => {
  vi.mocked(useSandboxAvailability).mockReturnValue(availability);
  vi.mocked(getAppSettings).mockImplementation(async () => ({ ...stored }));
  vi.mocked(updateAppSettings).mockImplementation(async (patch) => {
    stored = { ...stored, ...patch };
    return { ...stored };
  });
  vi.mocked(updateAppSettings).mockClear();
});

it("falls back to manual approval exactly once under StrictMode", async () => {
  // The app renders under StrictMode (main.tsx), so every effect runs twice in
  // development. The downgrade must be written once: the optimistic tier update
  // inside `changeSettings` is what the replay then sees (already "manual"), so
  // the second pass finds nothing left to fix — and the user gets one notice.
  stored = { ...DEFAULT_APP_SETTINGS, approvalTier: "sandbox" };
  const toasts: unknown[] = [];
  const stop = onFutureEvent("toast", payload => toasts.push(payload));

  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<StrictMode><Host /></StrictMode>));

    const approvalWrites = vi.mocked(updateAppSettings).mock.calls.map(([patch]) => patch.approvalTier).filter(tier => tier !== undefined);
    expect(approvalWrites).toEqual(["manual"]);
    expect(toasts).toHaveLength(1);
    expect(stored.approvalTier).toBe("manual");
  }
  finally {
    stop();
    await act(async () => root.unmount());
  }
});

it("leaves a manual tier alone — there is nothing to fall back from", async () => {
  stored = { ...DEFAULT_APP_SETTINGS, approvalTier: "manual" };
  const toasts: unknown[] = [];
  const stop = onFutureEvent("toast", payload => toasts.push(payload));

  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Host />));
    expect(vi.mocked(updateAppSettings).mock.calls.map(([patch]) => patch.approvalTier).filter(t => t !== undefined))
      .toEqual([]);
    expect(toasts).toHaveLength(0);
  }
  finally {
    stop();
    await act(async () => root.unmount());
  }
});

function Host() {
  useAppSettings("authenticated");
  return null;
}
