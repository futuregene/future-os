// @vitest-environment jsdom
import { act, createElement, Fragment, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../../test/renderHook";
import { useAutoUpgradeSkills } from "./useAutoUpgradeSkills";

const mocks = vi.hoisted(() => ({ sync: vi.fn(), emit: vi.fn() }));
vi.mock("../../../integrations/skills/skillsClient", () => ({ syncSkills: mocks.sync }));
vi.mock("../../../lib/futureEvents", () => ({ emitFutureEvent: mocks.emit }));

const unmounts: Array<() => void> = [];
function mount(enabled = true, strict = false) {
  const container = document.createElement("div");
  const root = createRoot(container);
  function App({ enabled: active }: { enabled: boolean }) {
    useAutoUpgradeSkills(active);
    return null;
  }
  const render = (active: boolean) => act(() => {
    root.render(createElement(strict ? StrictMode : Fragment, null, createElement(App, { enabled: active })));
  });
  render(enabled);
  const unmount = () => act(() => root.unmount());
  unmounts.push(unmount);
  return { render };
}
function deferred() {
  let resolve!: (value: { installed: string[]; upgraded: string[]; failed: string[] }) => void;
  const promise = new Promise<{ installed: string[]; upgraded: string[]; failed: string[] }>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
const empty = { installed: [], upgraded: [], skipped: [], failed: [] };

beforeEach(() => {
  vi.resetAllMocks();
  mocks.sync.mockResolvedValue(empty);
});
afterEach(() => {
  for (const unmount of unmounts.splice(0))
    unmount();
  vi.restoreAllMocks();
});

describe("useAutoUpgradeSkills lifecycle", () => {
  it("runs one Agent sync under StrictMode and broadcasts changes", async () => {
    mocks.sync.mockResolvedValueOnce({ ...empty, installed: ["new"] });
    mount(true, true);
    await flushAsync();
    expect(mocks.sync).toHaveBeenCalledTimes(1);
    expect(mocks.emit).toHaveBeenCalledWith("skills-changed", undefined);
  });

  it("starts only when enabled", async () => {
    const h = mount(false);
    await flushAsync();
    expect(mocks.sync).not.toHaveBeenCalled();
    h.render(true);
    await flushAsync();
    expect(mocks.sync).toHaveBeenCalledTimes(1);
  });

  it("queues a re-enable behind an in-flight sync", async () => {
    const first = deferred();
    mocks.sync.mockReturnValueOnce(first.promise);
    const h = mount();
    await flushAsync();
    h.render(false);
    h.render(true);
    await flushAsync();
    expect(mocks.sync).toHaveBeenCalledTimes(1);
    first.resolve({ installed: ["old"], upgraded: [], failed: [] });
    await flushAsync();
    expect(mocks.sync).toHaveBeenCalledTimes(2);
    expect(mocks.emit).not.toHaveBeenCalled();
  });

  it("does not notify after unmount", async () => {
    const first = deferred();
    mocks.sync.mockReturnValueOnce(first.promise);
    mount();
    await flushAsync();
    unmounts.pop()!();
    first.resolve({ installed: ["new"], upgraded: [], failed: [] });
    await flushAsync();
    expect(mocks.emit).not.toHaveBeenCalled();
  });

  it("retries after an RPC failure on the next enable", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    mocks.sync.mockRejectedValueOnce(new Error("offline"));
    const h = mount();
    await flushAsync();
    h.render(false);
    h.render(true);
    await flushAsync();
    expect(mocks.sync).toHaveBeenCalledTimes(2);
  });
});
