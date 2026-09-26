// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AboutPage } from "./AboutPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  buildInfo: { data: null as { version: string; isRelease: boolean } | null },
  openExternalUrl: vi.fn(),
}));

vi.mock("../../integrations/storage/files", () => ({ openExternalUrl: mocks.openExternalUrl }));
vi.mock("../../integrations/tauri/useBuildInfo", () => ({
  useBuildInfo: () => ({ data: mocks.buildInfo.data }),
}));
// The follow card has its own suite (clipboard, QR asset); keep this one about
// the build identity and the external links.
vi.mock("./WechatFollowCard", () => ({ WechatFollowCard: () => <div data-testid="wechat-card" /> }));

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  mocks.buildInfo.data = { version: "1.2.3", isRelease: true };
  mocks.openExternalUrl.mockReset();
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function mount() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  act(() => root.render(<AboutPage />));
  return container;
}

function link(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent === text) as HTMLButtonElement;
}

describe("aboutPage", () => {
  it("shows the release version with no test-build badge", () => {
    const container = mount();

    expect(container.textContent).toContain("1.2.3");
    expect(container.textContent).not.toContain("Test build");
    expect(container.textContent).toContain("MIT License");
  });

  it("badges a dev build", () => {
    mocks.buildInfo.data = { version: "1.2.3-dev.abcdef", isRelease: false };
    const container = mount();

    expect(container.textContent).toContain("Test build");
    expect(container.textContent).toContain("1.2.3-dev.abcdef");
  });

  it("shows a placeholder until the build info resolves", () => {
    mocks.buildInfo.data = null;
    const container = mount();

    expect(container.textContent).toContain("—");
    expect(container.textContent).not.toContain("Test build");
  });

  it("opens the website and the GitHub repository in the system browser", () => {
    const container = mount();

    act(() => link(container, "www.future-os.cn").click());
    expect(mocks.openExternalUrl).toHaveBeenLastCalledWith("https://www.future-os.cn");

    act(() => link(container, "github.com/futuregene/future-os").click());
    expect(mocks.openExternalUrl).toHaveBeenLastCalledWith("https://github.com/futuregene/future-os");
    expect(mocks.openExternalUrl).toHaveBeenCalledTimes(2);
  });

  it("credits the open-source stack", () => {
    const container = mount();

    expect(container.textContent).toContain("Built with open-source software, including Tauri, React, Tokio, Serde, SQLite, etc.");
    expect(container.querySelector("[data-testid=wechat-card]")).not.toBeNull();
  });
});
