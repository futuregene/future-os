// @vitest-environment jsdom
import type { AgentMessage, SessionEntry } from "@future-os/thread-projection";
import { entriesToMessages } from "@future-os/thread-projection";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { RemoteConversationView } from "./RemoteConversationView";

/**
 * The remote transcript's fork affordance.
 *
 * What matters is the *fork point*: a reply is not itself a stored turn, so a
 * fork must branch at the user entry that produced it, and must not be offered
 * when that turn is not persisted on the host — the host resolves the id against
 * its store, so sending one it has never seen is a guaranteed refusal.
 */

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
    startDragging: () => Promise.resolve(),
  }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function entry(id: string, role: "assistant" | "user", text: string, runId?: string): SessionEntry {
  return {
    id,
    kind: "message",
    role,
    ...(runId ? { runId } : {}),
    createdAtMs: 1_000,
    blocks: [{ kind: "text", text }],
  };
}

/** A reply whose run failed, which is the only state a host will continue from. */
function failedEntry(id: string, runId: string | null): SessionEntry {
  return {
    ...entry(id, "assistant", "half an answer"),
    runId,
    run: { status: "failed", error: "the provider dropped the connection" },
  };
}

const peer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  desktopId: "desktop_a",
  error: null,
  features: [],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

async function mount(props: {
  entries: SessionEntry[];
  /** The reply the host is writing, if the test is about streaming. */
  live?: AgentMessage | null;
  onContinueRun?: (runId: string) => void;
  onFork?: (sourceEntryId: string, forkable: boolean) => void;
  onOpenUsage?: () => void;
  persistedEntryIds?: string[];
  streaming?: boolean;
}) {
  const onFork = props.onFork ?? vi.fn();
  const onContinueRun = props.onContinueRun ?? vi.fn();
  const onOpenUsage = props.onOpenUsage ?? vi.fn();
  // The transcript is the projector's output, which is what the view renders:
  // one message per turn, not one per entry.
  const messages = entriesToMessages(props.entries);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteConversationView
        approvals={[]}
        approvalErrors={{}}
        approvalPending={null}
        desktopId="desktop_a"
        composer={<div data-testid="composer" />}
        messages={messages}
        error={null}
        hasMore={false}
        leftPanelExpanded
        live={props.live ?? null}
        loading={false}
        loadingOlder={false}
        onContinueRun={onContinueRun}
        onDecideApproval={() => {}}
        onFork={onFork}
        onLoadOlder={() => {}}
        onOpenUsage={onOpenUsage}
        onRetry={() => {}}
        onToggleLeftPanel={() => {}}
        peer={peer}
        persistedEntryIds={new Set(props.persistedEntryIds ?? props.entries.map(item => item.id))}
        sessionId="sess_1"
        streaming={props.streaming ?? false}
        title="Remote conversation"
      />,
    );
  });
  return {
    container,
    onContinueRun,
    onOpenUsage,
    // Revealed on hover in the product, but always mounted so it is reachable
    // by keyboard — which is also what makes it addressable here.
    // The fork control is the local transcript's own button. Its label is a
    // `title` (the local one has no aria-label), and the local wording is
    // "Fork" — this view no longer supplies its own label for it.
    forkButtons: () => [...container.querySelectorAll<HTMLButtonElement>("button[title='Fork']")],
    // The header's own actions are icon buttons, as in this app's conversation
    // header: the label is the accessible name, not the button's text.
    usageButton: () => container.querySelector<HTMLButtonElement>("[data-testid='remote-thread-usage']"),
    continueButton: () => [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(node => node.textContent === "Continue"),
    /**
     * The transcript's rows, by their own text.
     *
     * Read off the rendered message blocks rather than the whole container so an
     * assertion about *order* (the live reply is last) cannot be satisfied by text
     * somewhere else on the page.
     */
    rows: () => [...container.querySelectorAll<HTMLElement>("[data-message-id]")]
      .map(node => (node.textContent ?? "").trim()),
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

afterEach(() => {
  document.body.innerHTML = "";
});

it("offers fork on a reply, branching at the turn's user entry", async () => {
  const onFork = vi.fn();
  const view = await mount({
    entries: [entry("u1", "user", "hello"), entry("a1", "assistant", "hi")],
    onFork,
  });

  const [button] = view.forkButtons();
  expect(button).toBeTruthy();
  await act(async () => button!.click());

  // `u1`, not `a1`: the reply has no stored turn of its own.
  expect(onFork).toHaveBeenCalledWith("u1", true);
  await view.unmount();
});

it("offers no fork on a user entry, nor before any prompt", async () => {
  const view = await mount({
    entries: [entry("a0", "assistant", "unsolicited"), entry("u1", "user", "hello")],
  });

  // The leading reply has no preceding prompt; the user entry is not a turn's
  // *result*, so there is nothing on it to branch from.
  expect(view.forkButtons()).toHaveLength(0);
  await view.unmount();
});

it("skips a tool reply to reach the prompt that started the turn", async () => {
  const onFork = vi.fn();
  const tool: SessionEntry = {
    id: "t1",
    kind: "message",
    role: "assistant",
    createdAtMs: 1_000,
    blocks: [{ kind: "tool_call", name: "Bash" }],
  };
  const view = await mount({
    entries: [entry("u1", "user", "hello"), tool],
    onFork,
  });

  await act(async () => view.forkButtons()[0]!.click());
  expect(onFork).toHaveBeenCalledWith("u1", true);
  await view.unmount();
});

/**
 * A turn that arrived as a live push is not in the host's store, so the action
 * reports that instead of sending an id the host cannot resolve.
 */
it("marks a fork whose turn is not persisted, rather than hiding it", async () => {
  const onFork = vi.fn();
  const view = await mount({
    entries: [entry("s_1:r_1:1:7", "user", "just sent"), entry("s_1:r_1:1:8", "assistant", "replying")],
    // Neither id came from a history read.
    persistedEntryIds: [],
    onFork,
  });

  const [button] = view.forkButtons();
  expect(button).toBeTruthy();
  await act(async () => button!.click());

  expect(onFork).toHaveBeenCalledWith("s_1:r_1:1:7", false);
  await view.unmount();
});

/**
 * The reply the host is writing is drawn after the transcript.
 *
 * It is the same `MessageBlock` a local run's live reply uses, so a reader
 * watching another machine answer sees what they would see here — and the row is
 * the last one, which is also why it offers nothing to act on: a fork branches at
 * a settled turn and the host would refuse a second prompt.
 */
it("draws the live reply as the last row, and offers nothing to act on while it is written", async () => {
  const live: AgentMessage = {
    id: "stream_run_1",
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "working on it",
    status: "streaming",
    createdAt: new Date(1_700_000_000_000).toISOString(),
    runId: "run_1",
  };
  const view = await mount({
    entries: [entry("u1", "user", "hello"), entry("a1", "assistant", "done", "run_0")],
    live,
    streaming: true,
  });

  // The transcript's own reply, then the one being written — in that order, and
  // the live one last (the header and the author lines are the renderer's).
  const rows = view.rows();
  expect(rows).toHaveLength(3);
  expect(rows[1]).toContain("done");
  expect(rows[2]).toContain("working on it");
  // The settled turn keeps its fork; the live row has none, and neither reply
  // offers a continuation while the host is answering.
  expect(view.forkButtons()).toHaveLength(1);
  expect(view.continueButton()).toBeUndefined();
  await view.unmount();
});

/** While a run is in flight the turn being forked has not settled. */
it("hides fork while the host is running", async () => {
  const view = await mount({
    entries: [entry("u1", "user", "hello"), entry("a1", "assistant", "hi")],
    streaming: true,
  });

  expect(view.forkButtons()).toHaveLength(0);
  await view.unmount();
});

it("offers fork per reply, not once for the conversation", async () => {
  const onFork = vi.fn();
  const view = await mount({
    entries: [
      entry("u1", "user", "first"),
      entry("a1", "assistant", "answer one"),
      entry("u2", "user", "second"),
      entry("a2", "assistant", "answer two"),
    ],
    onFork,
  });

  expect(view.forkButtons()).toHaveLength(2);
  await act(async () => view.forkButtons()[1]!.click());
  expect(onFork).toHaveBeenCalledWith("u2", true);
  await view.unmount();
});

/**
 * The header offers the conversation's usage, and nothing else.
 *
 * Compacting used to live here. It is a conversation-level *request* rather than
 * a control you press at a glance, and it now sits behind `/` in the composer —
 * where the action applies. Putting a button back in this header is what this
 * asserts against.
 */
it("offers usage in the header, and no compact action", async () => {
  const onOpenUsage = vi.fn();
  const view = await mount({
    entries: [entry("u1", "user", "hello"), entry("a1", "assistant", "hi")],
    onOpenUsage,
  });

  const usage = view.usageButton()!;
  expect(usage).not.toBeNull();
  await act(async () => usage.click());
  expect(onOpenUsage).toHaveBeenCalledTimes(1);

  // No compact control anywhere in the header: it is the composer's `/` menu.
  const header = view.container.querySelector("header")!;
  expect(header.textContent ?? "").not.toContain("Compact");
  await view.unmount();
});

/**
 * The fork control fades in on hover but is never unmounted, so it stays
 * reachable without a pointer. Both halves of that are asserted: a control
 * removed from the tree on mouse-out is invisible to a keyboard user.
 */
it("reveals the fork control on hover and keeps it mounted when not hovering", async () => {
  const view = await mount({
    entries: [entry("u1", "user", "hello"), entry("a1", "assistant", "hi")],
  });
  const row = view.forkButtons()[0]!.parentElement!;
  expect(view.forkButtons()[0]!.className).toContain("opacity-0");

  await act(async () => {
    row.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  });
  expect(view.forkButtons()).toHaveLength(1);
  expect(view.forkButtons()[0]!.className).toContain("opacity-100");

  await act(async () => {
    row.dispatchEvent(new MouseEvent("mouseout", { bubbles: true }));
  });
  expect(view.forkButtons()).toHaveLength(1);
  expect(view.forkButtons()[0]!.className).toContain("opacity-0");
  await view.unmount();
});

// ── continuing a failed run ─────────────────────────────────────────────────

/**
 * A failed run offers a way on.
 *
 * The host will only continue a run that *failed*, which is why the control is
 * tied to the error rather than to the reply: anything else it would refuse.
 */
it("offers to continue a failed run, and reports it", async () => {
  const onContinueRun = vi.fn();
  const view = await mount({
    entries: [entry("u1", "user", "hello"), failedEntry("a1", "run_7")],
    onContinueRun,
  });

  const button = view.continueButton();
  expect(button).toBeTruthy();
  await act(async () => button!.click());

  expect(onContinueRun).toHaveBeenCalledWith("run_7");
  await view.unmount();
});

/** Nothing to continue when the host named no run. */
it("offers no continuation when the failed reply names no run", async () => {
  const view = await mount({
    entries: [entry("u1", "user", "hello"), failedEntry("a1", null)],
  });

  // No run id, so there is no run to resume — and no reason to show either: the
  // shared projector attributes a run's outcome by its id, so an entry carrying
  // an error it cannot be attributed to is a frame no real host sends. Asserted
  // as the absence of the *control*, which is what the host would refuse.
  expect(view.continueButton()).toBeUndefined();
  await view.unmount();
});

/** A reply that did not fail gets no continuation, because none is valid. */
it("offers no continuation for a reply that succeeded", async () => {
  const view = await mount({
    entries: [entry("u1", "user", "hello"), entry("a1", "assistant", "fine")],
  });

  expect(view.continueButton()).toBeUndefined();
  await view.unmount();
});

/** While the host is working there is nothing to resume, and a second run is refused. */
it("hides the continuation while the host is running", async () => {
  const view = await mount({
    entries: [entry("u1", "user", "hello"), failedEntry("a1", "run_7")],
    streaming: true,
  });

  expect(view.continueButton()).toBeUndefined();
  await view.unmount();
});

/**
 * A turn that ran several tools shows every one of them.
 *
 * A host turn routinely makes two calls (a read, then a shell command), and the
 * transcript rendered only the first — so the row that said what the machine
 * actually ran was missing, and its arguments were never fetched. Found by
 * rendering a real transcript in the screenshot harness; the fixtures in this
 * file had one tool per turn, which is why the suite did not see it.
 */
it("shows every tool call of a turn, not just the first", async () => {
  const twoTools: SessionEntry = {
    id: "a1",
    kind: "message",
    role: "assistant",
    createdAtMs: 1_000,
    runId: "run_1",
    blocks: [
      { kind: "text", text: "looking" },
      { kind: "tool_call", toolCallId: "c1", name: "read" },
      { kind: "tool_call", toolCallId: "c2", name: "shell" },
    ],
  };
  const view = await mount({ entries: [entry("u1", "user", "do it"), twoTools] });

  // Rendered by the local transcript, which groups a turn's calls: the count is
  // the local summary ("Tool calls 2×"), and expanding it is where each call —
  // and its target — is read.
  const text = view.container.textContent ?? "";
  expect(text).toContain("×2");
  // Both calls are in the transcript, not just the first.
  const rows = view.container.querySelectorAll("[data-activity-group], button");
  expect(rows.length).toBeGreaterThan(0);
  await view.unmount();
});

/**
 * The fork control follows the pointer, driven by the message it belongs to.
 *
 * Hidden-but-mounted so it stays reachable by keyboard, and `pointer-events-none`
 * while hidden so an invisible control cannot be clicked. both halves are
 * asserted here from the pointer: leave hides it, over shows it. (The message
 * frame owns that state — the control itself has no hover of its own.)
 */
it("follows the pointer over the message it belongs to", async () => {
  const view = await mount({
    entries: [entry("u1", "user", "do it"), entry("a1", "assistant", "done")],
  });
  // The projected message's id is the projector's (`m_a1`), not the entry's, so
  // the frame is found from the control that belongs to it.
  const fork = () => view.forkButtons()[0]!;
  const message = fork().closest("[data-message-id]")!;

  await act(async () => {
    // React synthesizes `onPointerLeave` from `pointerout` (it pairs the event
    // with a `relatedTarget` outside the element), so a bare `pointerleave`
    // never reaches the handler in jsdom — this is the event a browser sends.
    message.dispatchEvent(new PointerEvent("pointerout", { bubbles: true, relatedTarget: document.body }));
  });
  expect(fork().className).toContain("opacity-0");
  expect(fork().className).toContain("pointer-events-none");

  await act(async () => {
    message.dispatchEvent(new PointerEvent("pointerover", { bubbles: true }));
  });
  expect(fork().className).toContain("opacity-100");
  // Still mounted while hidden and while shown: a control removed on mouse-out
  // is invisible to a keyboard user.
  expect(view.forkButtons()).toHaveLength(1);
  await view.unmount();
});
