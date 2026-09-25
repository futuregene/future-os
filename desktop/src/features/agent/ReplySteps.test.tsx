// @vitest-environment jsdom
import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { MessageBlock } from "./MessageBlock";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const container = document.createElement("div");
let root: ReturnType<typeof createRoot> | undefined;
const thought: MessageSegment = { id: "thought", kind: "thinking", text: "Full reasoning content" };
const tool: MessageSegment = { id: "tool", kind: "activity", item: { id: "tool", kind: "read", status: "completed", target: "/workspace/file.ts" } };
const failed: MessageSegment = { id: "failed", kind: "activity", item: { id: "failed", kind: "shell", status: "failed", target: "npm test" } };
const running: MessageSegment = { id: "running", kind: "activity", item: { id: "running", kind: "shell", status: "running", target: "npm run typecheck" } };
const prose: MessageSegment = { id: "prose", kind: "text", text: "Visible response" };

async function render(segments: MessageSegment[], { streaming = false, thinkingActive = false } = {}) {
  if (!root) {
    document.body.append(container);
    root = createRoot(container);
  }
  const message: AgentMessage = {
    id: "reply",
    runId: "run",
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    createdAt: "2026-09-17T00:00:00Z",
    status: streaming ? "streaming" : "complete",
    segments,
    thinkingActive,
  };
  await act(async () => root!.render(<MessageBlock message={message} hovered={false} onHover={vi.fn()} onLeave={vi.fn()} workspacePath="/workspace" />));
}

function summary() {
  return Array.from(container.querySelectorAll("button[aria-label]")).find(button => button.querySelector(".lucide-wrench, .lucide-brain"))!;
}

async function click(button: Element) {
  await act(async () => (button as HTMLButtonElement).click());
}

afterEach(async () => {
  act(() => root?.unmount());
  root = undefined;
  container.remove();
  await i18n.changeLanguage("en");
});

it.each(["en", "zh"])("folds mixed steps into a muted right-aligned glyph/count summary with an accessible failure label (%s)", async (language) => {
  await i18n.changeLanguage(language);
  await render([prose, thought, tool, failed, running], { streaming: true });
  const button = summary();
  expect(button.textContent).toBe("×2·×1");
  expect(button.getAttribute("aria-label")).toBe(language === "zh" ? "工具调用 2 次 · 思考 1 次 · 1 次失败" : "Tool calls 2× · Thought 1× · 1 failed");
  expect(button.getAttribute("aria-expanded")).toBe("false");
  expect(button.classList.contains("self-end")).toBe(true);
  expect(button.parentElement!.classList.contains("text-ink-muted")).toBe(true);
  expect(button.querySelector(".lucide-triangle-alert")).not.toBeNull();
  expect(container.textContent).toContain("Visible response");
  expect(container.textContent).toContain(i18n.t("agent:activity.runningCommand"));
  expect(container.textContent).not.toContain("Full reasoning content");
  expect(container.textContent).not.toContain("file.ts");
});

it("expands steps in timeline order on the left and keeps each detail independently collapsible", async () => {
  await render([thought, tool, failed]);
  const button = summary();
  await click(button);
  expect(button.classList.contains("self-end")).toBe(true);
  expect(button.getAttribute("aria-expanded")).toBe("true");
  const children = button.nextElementSibling!;
  const headers = Array.from(children.querySelectorAll("button"));
  expect(headers.map(header => header.textContent)).toEqual([
    i18n.t("agent:activity.thoughtCompleted"),
    i18n.t("agent:activity.readCompleted"),
    i18n.t("agent:activity.failed.shell"),
  ]);
  expect(headers[0]!.classList.contains("self-start")).toBe(true);
  expect(headers[1]!.parentElement!.classList.contains("self-start")).toBe(true);
  expect(children.textContent).not.toContain("Full reasoning content");
  expect(children.textContent).not.toContain("file.ts");
  await click(headers[0]!);
  await click(headers[1]!);
  expect(children.textContent).toContain("Full reasoning content");
  expect(children.textContent).toContain("file.ts");
  expect(children.querySelector(".wrap-anywhere.text-left")).not.toBeNull();
  await click(button);
  expect(container.textContent).not.toContain("Full reasoning content");
  expect(container.textContent).not.toContain("file.ts");
});

it("always lets users expand and collapse standalone reasoning without a setting", async () => {
  await render([thought]);
  const thoughtButton = container.querySelector("button[aria-expanded]")!;
  expect(thoughtButton.getAttribute("aria-expanded")).toBe("false");
  expect(container.textContent).not.toContain("Full reasoning content");
  await click(thoughtButton);
  expect(thoughtButton.getAttribute("aria-expanded")).toBe("true");
  expect(container.textContent).toContain("Full reasoning content");
  await click(thoughtButton);
  expect(container.textContent).not.toContain("Full reasoning content");
});

it("keeps the initial thinking hint until an inline reasoning row arrives", async () => {
  await render([], { streaming: true, thinkingActive: true });
  expect(container.textContent).toContain(i18n.t("agent:message.thinking"));
  await render([thought], { streaming: true, thinkingActive: true });
  expect(container.textContent).not.toContain(i18n.t("agent:message.thinking"));
  expect(container.textContent).toContain(i18n.t("agent:activity.thinking"));
  await render([]);
  expect(container.textContent).not.toContain(i18n.t("agent:message.thinking"));
});

it("keeps live reasoning outside the group, collapsed but expandable", async () => {
  const liveThought: MessageSegment = { id: "live-thought", kind: "thinking", text: "Currently reasoning" };
  await render([tool, failed, liveThought], { streaming: true });
  expect(summary().textContent).toBe("×2");
  const thinkingButton = Array.from(container.querySelectorAll("button")).find(button => button.textContent === i18n.t("agent:activity.thinking"))!;
  expect(thinkingButton.classList.contains("self-end")).toBe(true);
  expect(container.textContent).not.toContain("Currently reasoning");
  await click(thinkingButton);
  expect(container.textContent).toContain("Currently reasoning");
  expect(thinkingButton.classList.contains("self-start")).toBe(true);
});

it("keeps a same-kind burst as one projected step and counts every call behind it", async () => {
  const burst: MessageSegment = { id: "burst", kind: "activity", item: {
    id: "burst",
    kind: "read",
    status: "completed",
    count: 2,
    children: [
      { id: "one", kind: "read", status: "completed", target: "/workspace/one.ts" },
      { id: "two", kind: "read", status: "completed", target: "/workspace/two.ts" },
    ],
  } };
  await render([burst, thought]);
  // The burst is one row, but it stands for its two calls: the summary has to
  // agree with the "Read 2 files" row it reveals, not with the row count.
  expect(summary().textContent).toBe("×2·×1");
  await click(summary());
  const burstButton = summary().nextElementSibling!.querySelector("button")!;
  expect(burstButton.textContent).toBe(i18n.t("agent:activity.readFiles", { count: 2 }));
  await click(burstButton);
  expect(container.textContent).toContain("one.ts");
  expect(container.textContent).toContain("two.ts");
});

it("preserves an expanded summary as new settled steps arrive", async () => {
  await render([thought, tool, running], { streaming: true });
  await click(summary());
  await render([thought, tool, failed, running], { streaming: true });
  expect(summary().getAttribute("aria-expanded")).toBe("true");
  expect(summary().textContent).toBe("×2·×1");
  expect(summary().nextElementSibling!.textContent).toContain(i18n.t("agent:activity.failed.shell"));
});
