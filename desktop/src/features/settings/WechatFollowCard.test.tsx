// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { WechatFollowCard } from "./WechatFollowCard";

const copyText = vi.fn<(value: string) => Promise<void>>(async () => {});
const openExternalUrl = vi.fn<(url: string) => Promise<void>>(async () => {});
vi.mock("../../lib/clipboard", () => ({ copyText: (value: string) => copyText(value) }));
vi.mock("../../integrations/storage/files", () => ({
  openExternalUrl: (url: string) => openExternalUrl(url),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

async function mount() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<WechatFollowCard />));
  return { container, root };
}

it("shows a scannable QR code for the account", async () => {
  const { container, root } = await mount();
  try {
    const img = container.querySelector("img")!;
    // The asset is imported through Vite, so assert it resolved to a real URL
    // (a broken import would render an empty src and no scannable code at all).
    expect(img.getAttribute("src")).toContain("wechat-qrcode");
    // The alt text names the account, so a screen reader conveys what scanning does.
    expect(img.getAttribute("alt")).toContain("FutureOS");
    expect(img.getAttribute("width")).toBe("128");
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});

it("copies the account name and reads back the copied state", async () => {
  const { container, root } = await mount();
  try {
    const button = [...container.querySelectorAll("button")]
      .find(node => node.textContent === "Copy account name")!;
    expect(button).toBeDefined();
    await act(async () => button.click());
    // The name is what a user pastes into WeChat's search box, so it must be
    // exactly the account name — not the URL and not a prefixed string.
    expect(copyText).toHaveBeenCalledExactlyOnceWith("FutureOS");
    expect(container.textContent).toContain("Copied");
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});

it("opens the published article in the system browser", async () => {
  const { container, root } = await mount();
  try {
    const button = [...container.querySelectorAll("button")]
      .find(node => node.textContent === "Read an article")!;
    await act(async () => button.click());
    expect(openExternalUrl).toHaveBeenCalledExactlyOnceWith(
      "https://mp.weixin.qq.com/s/qefitj15rYRhK6lYmTj2dA",
    );
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
