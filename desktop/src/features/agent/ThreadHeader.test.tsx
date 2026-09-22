import type { Root } from "react-dom/client";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { ThreadHeader } from "./ThreadHeader";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("../../lib/windowDrag", () => ({ startWindowDrag: () => {} }));
vi.mock("../../components/layout/LeftPanelTitlebarToggle", () => ({
  LeftPanelTitlebarToggle: () => null,
}));

const revalidateUsage = vi.fn();
vi.mock("../../integrations/agent/agentStateCache", () => ({
  revalidateAgentState: (threadId: string) => revalidateUsage(threadId),
}));

const thread = {
  id: "t1",
  title: "Priced conversation",
  agentSessionId: "s1",
} as unknown as Parameters<typeof ThreadHeader>[0]["thread"];

const usage = {
  inputTokens: 1_000_000,
  outputTokens: 100_000,
  cacheReadTokens: 200_000,
  cacheWriteTokens: 50_000,
  costCny: 1.2345,
  costInputCny: 0.75,
  costOutputCny: 0.4,
  costCacheReadCny: 0.08,
  costCacheWriteCny: 0.0045,
};

let roots: Root[] = [];
afterEach(() => {
  const mounted = roots;
  roots = [];
  act(() => mounted.forEach(root => root.unmount()));
  document.body.innerHTML = "";
});

function render(element: Parameters<Root["render"]>[0]) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push(root);
  act(() => root.render(element));
  return container;
}

it("opens the breakdown from an icon, and keeps the amount out of the header", async () => {
  revalidateUsage.mockClear();
  const container = render(
    <ThreadHeader
      leftPanelExpanded
      onToggleLeftPanel={() => {}}
      thread={thread}
      usage={usage}
    />,
  );
  const button = container.querySelector<HTMLButtonElement>("[data-testid=thread-usage]")!;
  // An icon, not the figure: the header is for the conversation.
  expect(button.textContent).toBe("");
  expect(button.querySelector("svg")).not.toBeNull();
  expect(container.textContent).not.toContain("¥");

  await act(async () => button.click());

  // Opening the panel re-reads the session: a cached figure can be a run behind,
  // and the panel's whole job is to show what this conversation spent.
  expect(revalidateUsage).toHaveBeenCalledWith("t1");

  // The dialog names the conversation and splits the tokens by category, with
  // the non-cached input remainder billed as plain input.
  expect(document.body.textContent).toContain("Token usage and amount");
  expect(document.body.textContent).toContain("Priced conversation");
  const row = (label: string) =>
    document.querySelector(`[data-testid="usage-row-${label}"]`)?.textContent;
  expect(row("Input")).toContain("750,000");
  expect(row("Input")).toContain("¥0.75");
  expect(row("Output")).toContain("100,000");
  expect(row("Output")).toContain("¥0.4");
  expect(row("Cache read")).toContain("¥0.08");
  expect(row("Cache write")).toContain("¥0.0045");
  expect(document.querySelector("[data-testid=usage-total]")!.textContent).toBe("¥1.2345");
});

it("omits the button without an agent session and reports missing usage honestly", async () => {
  const container = render(
    <ThreadHeader
      leftPanelExpanded
      onToggleLeftPanel={() => {}}
      thread={{ ...thread, agentSessionId: null } as typeof thread}
    />,
  );
  expect(container.querySelector("[data-testid=thread-usage]")).toBeNull();

  const withSession = render(
    <ThreadHeader
      leftPanelExpanded
      onToggleLeftPanel={() => {}}
      thread={thread}
    />,
  );
  expect(withSession.querySelector("[data-testid=thread-usage]")).not.toBeNull();
  await act(async () => withSession.querySelector<HTMLButtonElement>("[data-testid=thread-usage]")!.click());
  expect(document.body.textContent).toContain("No usage recorded for this conversation yet.");
});

it("shows tokens only when the model has no prices configured", async () => {
  const container = render(
    <ThreadHeader
      leftPanelExpanded
      onToggleLeftPanel={() => {}}
      thread={thread}
      usage={{ ...usage, costInputCny: 0, costOutputCny: 0, costCacheReadCny: 0, costCacheWriteCny: 0 }}
    />,
  );
  await act(async () => container.querySelector<HTMLButtonElement>("[data-testid=thread-usage]")!.click());
  expect(document.querySelector(`[data-testid="usage-row-Output"]`)!.textContent).toContain("—");
  expect(document.body.textContent).toContain("no prices configured");
  // The billed total is still shown — it is the provider's own figure.
  expect(document.querySelector("[data-testid=usage-total]")!.textContent).toBe("¥1.2345");
});
