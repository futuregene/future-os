// @vitest-environment jsdom
import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { MessageBlock } from "./MessageBlock";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * A compaction-only message — the shape both the durable history row and the
 * live standalone projector produce for a committed checkpoint.
 */
function compactionMessage(
  segment: Partial<Extract<MessageSegment, { kind: "compaction" }>>,
): AgentMessage {
  return {
    id: "m_cp",
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    createdAt: new Date().toISOString(),
    status: "complete",
    segments: [{ id: "seg_cp", kind: "compaction", ...segment }],
  };
}

async function renderDivider(message: AgentMessage) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () =>
    root.render(
      <MessageBlock message={message} hovered={false} onHover={vi.fn()} onLeave={vi.fn()} />,
    ));
  return {
    label: () => container.querySelector("[role=\"status\"]")?.textContent,
    dispose: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

it("shows the pre-compaction tokens and the agent's post-compaction estimate", async () => {
  const languages = [
    { language: "en", expected: "Context compacted · 190,000 → 20,000 tokens (estimated)" },
    { language: "zh", expected: "上下文已压缩 · 190,000 → 20,000 tokens（预估）" },
  ];
  const rendered = await renderDivider(
    compactionMessage({ tokensBefore: 190_000, tokensAfter: 20_000, trigger: "automatic" }),
  );
  try {
    for (const { language, expected } of languages) {
      await act(async () => {
        await i18n.changeLanguage(language);
      });
      expect(rendered.label()).toBe(expected);
    }
  }
  finally {
    rendered.dispose();
    await i18n.changeLanguage("en");
  }
});

it("keeps a manual compaction's divider labelled with both counts", async () => {
  const rendered = await renderDivider(
    compactionMessage({
      tokensBefore: 190_000,
      tokensAfter: 20_000,
      trigger: "manual",
    }),
  );
  try {
    expect(rendered.label()).toBe(
      "You compacted this conversation's context · 190,000 → 20,000 tokens (estimated)",
    );
  }
  finally {
    rendered.dispose();
  }
});

it("falls back to the pre-compaction count alone when no estimate was reported", async () => {
  // A released run journal's `compaction_end`, and the retry-path compaction,
  // carry only `tokens_before`.
  const only = await renderDivider(compactionMessage({ tokensBefore: 190_000 }));
  const none = await renderDivider(compactionMessage({}));
  try {
    expect(only.label()).toBe("Context compacted · 190,000 tokens");
    expect(none.label()).toBe("Context compacted");
  }
  finally {
    only.dispose();
    none.dispose();
  }
});

it("keeps running and failed dividers free of token counts", async () => {
  // The checkpoint does not exist yet while a compaction runs, and a failed one
  // never commits: there is no pair to report, only the outcome.
  const running = await renderDivider(compactionMessage({ status: "running" }));
  const failed = await renderDivider(
    compactionMessage({ status: "failed", error: "summary failed" }),
  );
  try {
    expect(running.label()).toBe("Compacting context…");
    expect(failed.label()).toBe("Context compaction failed");
    expect(failed.label()).not.toContain("tokens");
  }
  finally {
    running.dispose();
    failed.dispose();
  }
});
