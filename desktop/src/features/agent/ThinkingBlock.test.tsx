// @vitest-environment jsdom
import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { setReplyDetail } from "../../lib/replyDisplay";
import { MessageBlock } from "./MessageBlock";

/**
 * A reasoning row on the trimmed lane arrives as a boundary and no text.
 *
 * The row is still a disclosure there — the body is read the first time a reader
 * expands it — and this pins both halves: nothing is fetched for drawing the
 * reply, and the row asks for *its own* block in the run (the index the host
 * resolves the text by). Without a resolver (a local run, or a host that cannot
 * answer) the row stays the plain label it was.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const container = document.createElement("div");
let root: ReturnType<typeof createRoot> | undefined;

beforeEach(() => {
  // These cases describe the reasoning row itself. The reading setting's
  // `default` level would fold it behind the reply's "process" row first, which
  // is a separate behaviour with its own suite.
  setReplyDetail("detailed");
  if (!root) {
    document.body.append(container);
    root = createRoot(container);
  }
});

afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
  container.innerHTML = "";
  document.body.innerHTML = "";
  container.remove();
});

const bodyless: MessageSegment = { id: "thought-1", kind: "thinking", text: "" };
const prose: MessageSegment = { id: "prose", kind: "text", text: "Visible response" };

async function render(
  segments: MessageSegment[],
  resolver?: (runId: string, index: number, settled?: boolean) => Promise<string | null>,
  { streaming = false } = {},
) {
  const message: AgentMessage = {
    id: "reply",
    runId: "run-9",
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    createdAt: "2026-09-17T00:00:00Z",
    status: streaming ? "streaming" : "complete",
    segments,
  };
  await act(async () =>
    root!.render(
      <MessageBlock
        message={message}
        hovered={false}
        onHover={() => {}}
        onLeave={() => {}}
        workspacePath="/workspace"
        onResolveReasoning={resolver}
      />,
    ),
  );
}

/** The reasoning rows' own toggles, never the folded run's summary above them. */
function thoughtRows() {
  const label = i18n.t("activity.thoughtCompleted", { ns: "agent" });
  return Array.from(container.querySelectorAll("button")).filter(
    button => button.textContent?.includes(label) && button.querySelector(".lucide-brain"),
  );
}

function thoughtRow() {
  return thoughtRows()[0];
}

/**
 * Open the folded step run a settled (or middle-of-a-stream) reply draws over
 * consecutive steps: the rows below it are what these cases are about.
 */
async function expandSteps() {
  const fold = Array.from(container.querySelectorAll("button[aria-label]")).find(
    button => button.querySelector(".lucide-brain, .lucide-wrench"),
  );
  if (fold && fold.getAttribute("aria-expanded") === "false")
    await click(fold);
}

async function click(button: Element) {
  await act(async () => (button as HTMLButtonElement).click());
}

it("draws nothing extra until the row is opened, then reads its own block", async () => {
  const resolve = vi.fn().mockResolvedValue("because the test said so");
  await render([bodyless, prose], resolve);

  // Drawing the reply is not a request.
  expect(resolve).toHaveBeenCalledTimes(0);
  await expandSteps();
  const row = thoughtRow();
  expect(row, "a fetchable row is a disclosure").toBeTruthy();

  await click(row!);
  expect(resolve).toHaveBeenCalledWith("run-9", 0, true);
  expect(container.textContent).toContain("because the test said so");
});

it("reads the block the row stands for, and labels a streaming reply as unsettled", async () => {
  const resolve = vi.fn().mockResolvedValue("second thought");
  const first: MessageSegment = { id: "thought-0", kind: "thinking", text: "first thought" };
  await render([first, bodyless, prose], resolve, { streaming: true });

  // The first block already carries its body; only the second is fetched.
  expect(resolve).toHaveBeenCalledTimes(0);
  await expandSteps();
  await click(thoughtRows()[1]!);
  expect(resolve).toHaveBeenCalledWith("run-9", 1, false);
  expect(container.textContent).toContain("second thought");
});

it("says so when the body cannot be read", async () => {
  const resolve = vi.fn().mockResolvedValue(null);
  await render([bodyless, prose], resolve);
  await expandSteps();
  await click(thoughtRow()!);
  expect(container.textContent).toContain(i18n.t("activity.thoughtUnavailable", { ns: "agent" }));
});

it("is a plain label when nothing can fetch it", async () => {
  await render([bodyless, prose]);
  await expandSteps();
  // No resolver: rendering a chevron anyway would open an empty box, which is
  // exactly the complaint this row's shape is answering.
  expect(thoughtRow()).toBeUndefined();
  expect(container.textContent).toContain(i18n.t("activity.thoughtCompleted", { ns: "agent" }));
});

it("reopening a row it already read does not ask again", async () => {
  const resolve = vi.fn().mockResolvedValue("because the test said so");
  await render([bodyless, prose], resolve);

  await expandSteps();
  await click(thoughtRow()!);
  await click(thoughtRow()!);
  expect(container.textContent).not.toContain("because the test said so");
  await click(thoughtRow()!);
  expect(resolve).toHaveBeenCalledTimes(1);
  expect(container.textContent).toContain("because the test said so");
});
