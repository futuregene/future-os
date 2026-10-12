import { act } from "react";
// @vitest-environment jsdom
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteUsageDialog } from "./RemoteUsageDialog";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The conversation's account book, quoting the host's figures.
 *
 * The tokens were spent on that machine, so its accounting is the only truthful
 * one: a dialog that recomputed a cost here would report a number the host never
 * charged.
 */
const request = vi.fn();
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: (...args: unknown[]) => request(...args),
}));

beforeEach(() => {
  request.mockReset().mockResolvedValue({
    usage: { inputTokens: 1_200, outputTokens: 340, cacheReadTokens: 100, costCny: 1.25, costInputCny: 0.7, costOutputCny: 0.5, costCacheReadCny: 0.05, costCacheWriteCny: 0 },
  });
});

async function mount() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onClose = vi.fn();
  await act(async () => {
    root.render(<RemoteUsageDialog desktopId="desktop_a" onClose={onClose} sessionId="sess_1" title="A conversation" />);
  });
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
  return {
    container,
    onClose,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

it("shows the host's usage for this conversation", async () => {
  const view = await mount();
  expect(view.container.textContent).toContain("A conversation");
  // This app's own dialog, quoting the host's figures: the token label and the
  // amount it charged there.
  expect(view.container.textContent).toMatch(/input/i);
  expect(view.container.textContent).toContain("1.25");
  await view.unmount();
});

/** A host that cannot report is not a host that spent nothing. */
it("reports a read failure rather than a fabricated zero", async () => {
  request.mockRejectedValue(new Error("host is not connected"));
  const view = await mount();
  expect(view.container.textContent).toContain("host is not connected");
  await act(async () => {
    [...view.container.querySelectorAll("button")].pop()!.click();
  });
  expect(view.onClose).toHaveBeenCalled();
  await view.unmount();
});
