// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { useFusionAvailability } from "./settings";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  stops: [] as ReturnType<typeof vi.fn>[],
}));
vi.mock("../tauri/invoke", () => ({ invokeCommand: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    mocks.handlers.set(event, handler);
    const stop = vi.fn(() => mocks.handlers.delete(event));
    mocks.stops.push(stop);
    return stop;
  }),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.handlers.clear();
  mocks.stops.length = 0;
  mocks.invoke.mockReset().mockResolvedValue({ enabled: false });
});

function Availability() {
  return <span>{String(useFusionAvailability())}</span>;
}

it("refreshes Fusion availability on settings and provider changes and releases listeners", async () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<Availability />));
    expect(container.textContent).toBe("false");
    expect([...mocks.handlers.keys()]).toEqual(["app_settings_changed", "provider-config-changed"]);

    mocks.invoke.mockResolvedValue({ enabled: true });
    await act(async () => mocks.handlers.get("app_settings_changed")!({ payload: {} }));
    expect(container.textContent).toBe("true");

    mocks.invoke.mockResolvedValue({ enabled: false });
    await act(async () => mocks.handlers.get("provider-config-changed")!({ payload: {} }));
    expect(container.textContent).toBe("false");
    expect(mocks.invoke.mock.calls).toEqual(Array.from({ length: 3 }, () => ["get_fusion_settings"]));
  }
  finally {
    await act(async () => root.unmount());
  }
  expect(mocks.handlers.size).toBe(0);
  expect(mocks.stops).toHaveLength(2);
  for (const stop of mocks.stops)
    expect(stop).toHaveBeenCalledExactlyOnceWith();
});

it("keeps activation unavailable when the settings request fails", async () => {
  mocks.invoke.mockRejectedValueOnce(new Error("host unavailable"));
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<Availability />));
    expect(container.textContent).toBe("false");
  }
  finally {
    await act(async () => root.unmount());
  }
});
