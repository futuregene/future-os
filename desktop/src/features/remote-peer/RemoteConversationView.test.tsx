// @vitest-environment jsdom
import type { SessionEntry } from "@future-os/thread-projection";
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

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function entry(id: string, role: "assistant" | "user", text: string): SessionEntry {
  return {
    id,
    kind: "message",
    role,
    createdAtMs: 1_000,
    blocks: [{ kind: "text", text }],
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
  onFork?: (sourceEntryId: string, forkable: boolean) => void;
  persistedEntryIds?: string[];
  streaming?: boolean;
}) {
  const onFork = props.onFork ?? vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteConversationView
        approvals={[]}
        approvalErrors={{}}
        approvalPending={null}
        composer={<div data-testid="composer" />}
        entries={props.entries}
        error={null}
        hasMore={false}
        loading={false}
        loadingOlder={false}
        onDecideApproval={() => {}}
        onFork={onFork}
        onLoadOlder={() => {}}
        onRetry={() => {}}
        peer={peer}
        persistedEntryIds={new Set(props.persistedEntryIds ?? props.entries.map(item => item.id))}
        streaming={props.streaming ?? false}
        title="Remote conversation"
      />,
    );
  });
  return {
    container,
    // Revealed on hover in the product, but always mounted so it is reachable
    // by keyboard — which is also what makes it addressable here.
    forkButtons: () => [...container.querySelectorAll<HTMLButtonElement>("button[aria-label='Branch from here']")],
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
 * The control fades in on hover but is never unmounted, so it stays reachable
 * without a pointer. Both halves of that are asserted: a control removed from
 * the tree on mouse-out is invisible to a keyboard user.
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
