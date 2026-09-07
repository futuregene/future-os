// @vitest-environment jsdom
import { act, createElement, Fragment, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../../test/renderHook";
import { useAutoUpgradeSkills } from "./useAutoUpgradeSkills";

const mocks = vi.hoisted(() => ({
  installed: vi.fn(),
  available: vi.fn(),
  install: vi.fn(),
  emit: vi.fn(),
}));
vi.mock("../../../integrations/skills/skillsClient", () => ({
  listInstalledSkills: mocks.installed,
  listAvailableSkills: mocks.available,
  installSkill: mocks.install,
}));
vi.mock("../../../features/skills/autoUpgrade", () => ({
  computeSkillUpgrades: () => [{ id: "example", version: "2.0" }],
}));
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
  let resolve!: (value: never[]) => void;
  const promise = new Promise<never[]>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.installed.mockResolvedValue([]);
  mocks.available.mockResolvedValue([]);
  mocks.install.mockResolvedValue(undefined);
});
afterEach(() => {
  for (const unmount of unmounts.splice(0))
    unmount();
  vi.restoreAllMocks();
});

describe("useAutoUpgradeSkills lifecycle", () => {
  it("upgrades once under StrictMode", async () => {
    mount(true, true);
    await flushAsync();
    expect(mocks.install).toHaveBeenCalledExactlyOnceWith("example", "2.0");
    expect(mocks.emit).toHaveBeenCalledWith("skills-changed", undefined);
  });

  it("does nothing while disabled and starts when enabled", async () => {
    const h = mount(false);
    await flushAsync();
    expect(mocks.available).not.toHaveBeenCalled();
    h.render(true);
    await flushAsync();
    expect(mocks.install).toHaveBeenCalledTimes(1);
  });

  it("does not lose a re-enable while the cancelled catalogue fetch is pending", async () => {
    const first = deferred();
    mocks.installed.mockReturnValueOnce(first.promise);
    const h = mount();
    await flushAsync();
    h.render(false);
    h.render(true);
    await flushAsync();
    expect(mocks.installed).toHaveBeenCalledTimes(1);
    first.resolve([]);
    await flushAsync();
    expect(mocks.installed).toHaveBeenCalledTimes(2);
    expect(mocks.install).toHaveBeenCalledTimes(1);
  });

  it("waits for an in-flight install before starting the replacement effect", async () => {
    const first = deferred();
    mocks.install.mockReturnValueOnce(first.promise);
    const h = mount();
    await flushAsync();
    expect(mocks.install).toHaveBeenCalledTimes(1);
    h.render(false);
    h.render(true);
    await flushAsync();
    expect(mocks.installed).toHaveBeenCalledTimes(1);
    first.resolve([]);
    await flushAsync();
    expect(mocks.installed).toHaveBeenCalledTimes(2);
    expect(mocks.install).toHaveBeenCalledTimes(2);
    expect(mocks.emit).toHaveBeenCalledTimes(1);
  });

  it("does not install or notify after unmount", async () => {
    const first = deferred();
    mocks.installed.mockReturnValueOnce(first.promise);
    mount();
    await flushAsync();
    unmounts.pop()!();
    first.resolve([]);
    await flushAsync();
    expect(mocks.install).not.toHaveBeenCalled();
    expect(mocks.emit).not.toHaveBeenCalled();
  });

  it("a catalogue failure does not poison the next enable", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    mocks.available.mockRejectedValueOnce(new Error("offline"));
    const h = mount();
    await flushAsync();
    h.render(false);
    h.render(true);
    await flushAsync();
    expect(mocks.install).toHaveBeenCalledTimes(1);
  });
});
