// @vitest-environment jsdom
import type { AgentActivityItem, AgentMessage } from "@future-os/thread-projection";
import type { ReactNode } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { onFutureEvent } from "../../lib/futureEvents";
import { AgentActivityLine } from "./AgentActivityList";
import { MessageBlock } from "./MessageBlock";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const container = document.createElement("div");
let root: ReturnType<typeof createRoot> | undefined;

async function render(node: ReactNode) {
  if (!root) {
    document.body.append(container);
    root = createRoot(container);
  }
  await act(async () => root!.render(node));
}

afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
  container.remove();
  vi.useRealTimers();
});

const message: AgentMessage = {
  id: "rail-message",
  role: "assistant",
  authorKey: "author.researchCopilot",
  content: "A completed response.",
  createdAt: "2026-09-17T00:00:00Z",
  status: "complete",
  durationMs: 20_000,
  outputTokens: 590,
};

function messageRow(patch: Partial<AgentMessage> = {}) {
  return <MessageBlock message={{ ...message, ...patch }} hovered={false} onHover={vi.fn()} onLeave={vi.fn()} />;
}

it("keeps settled stats and assistant copy visible on the right without hovering", async () => {
  await render(messageRow());
  const copy = container.querySelector(`button[aria-label="${i18n.t("common:copy")}"]`)!;
  const footer = copy.parentElement!;
  expect(footer.classList.contains("justify-end")).toBe(true);
  expect(copy.classList.contains("opacity-100")).toBe(true);
  expect(copy.classList.contains("pointer-events-none")).toBe(false);
  expect(footer.textContent).toContain("20s · 590 tokens");
  const stats = footer.lastElementChild!;
  expect(stats.classList.contains("text-ink-muted")).toBe(true);
  expect(stats.classList.contains("opacity-0")).toBe(false);
});

it("keeps the live dot and ticking timer on the same right rail until completion", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-09-17T00:00:20Z"));
  await render(messageRow({ status: "streaming", runStartedAt: Date.now() - 20_000, outputTokens: null }));
  const status = container.querySelector("[role=status]")!;
  const footer = status.parentElement!;
  expect(footer.classList.contains("justify-end")).toBe(true);
  expect(status.querySelector(".bg-generating")).not.toBeNull();
  expect(footer.textContent).toBe("20s");
  expect(footer.querySelector("button")).toBeNull();
  await act(async () => {
    vi.advanceTimersByTime(1000);
  });
  expect(footer.textContent).toBe("21s");
  await render(messageRow());
  expect(container.querySelector("[role=status]")).toBeNull();
  expect(footer.textContent).toContain("20s · 590 tokens");
});

it("preserves the user message's hover-only copy behavior", async () => {
  await render(messageRow({ role: "user", authorKey: "author.you" }));
  const copy = container.querySelector(`button[aria-label="${i18n.t("common:copy")}"]`)!;
  expect(copy.classList.contains("opacity-0")).toBe(true);
  expect(copy.parentElement!.classList.contains("justify-end")).toBe(true);
  expect(copy.parentElement!.textContent).not.toContain("tokens");
});

it.each(["running", "completed", "failed"] as const)("right-aligns a %s tool header and opens a wrapping, left-aligned target", async (status) => {
  const target = "printf 'a long command with spaces'";
  const inspect = vi.fn();
  const unsubscribe = onFutureEvent("inspect-tool", inspect);
  try {
    await render(<AgentActivityLine item={{ id: "tool", kind: "shell", status, target }} runId="run" />);
    const button = container.querySelector("button")!;
    expect(button.parentElement!.classList.contains("self-end")).toBe(true);
    expect(container.firstElementChild!.classList.contains("text-ink-muted")).toBe(true);
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(container.textContent).not.toContain(target);
    await act(async () => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("true");
    const detail = container.firstElementChild!.lastElementChild!;
    expect(detail.textContent).toBe(target);
    expect(detail.classList.contains("text-left")).toBe(true);
    expect(detail.classList.contains("wrap-anywhere")).toBe(true);
    expect(inspect).toHaveBeenCalledWith({ runId: "run", toolId: "tool" });
    if (status === "failed")
      expect(button.querySelector(".lucide-triangle-alert")).not.toBeNull();
    await act(async () => button.click());
    expect(container.textContent).not.toContain(target);
  }
  finally {
    unsubscribe();
  }
});

it("moves an opened same-kind group into the reading column, including legacy rows without a run", async () => {
  const children: AgentActivityItem[] = [
    { id: "read-1", kind: "read", status: "completed", target: "/workspace/src/one.ts" },
    { id: "read-2", kind: "read", status: "completed", target: "/workspace/src/two.ts" },
  ];
  await render(<AgentActivityLine item={{ id: "group", kind: "read", status: "completed", count: 2, children }} workspacePath="/workspace" />);
  const button = container.querySelector("button")!;
  expect(button.classList.contains("self-end")).toBe(true);
  expect(container.textContent).not.toContain("src/one.ts");
  await act(async () => button.click());
  expect(button.classList.contains("self-start")).toBe(true);
  expect(button.getAttribute("aria-expanded")).toBe("true");
  expect(container.textContent).toContain("src/one.ts");
  expect(container.textContent).toContain("src/two.ts");
  expect(container.querySelectorAll(".wrap-anywhere.text-left")).toHaveLength(2);
});

it("opens a standalone legacy target without requiring an inspector run", async () => {
  await render(<AgentActivityLine item={{ id: "legacy", kind: "read", status: "completed", target: "/workspace/file.ts" }} workspacePath="/workspace" />);
  const button = container.querySelector("button")!;
  await act(async () => button.click());
  expect(container.textContent).toContain("file.ts");
  expect(container.textContent).not.toContain("/workspace/");
});
