// @vitest-environment jsdom
import type { AgentMessage, SessionEntry } from "@future-os/thread-projection";
import { entriesToMessages } from "@future-os/thread-projection";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteConversationView } from "./RemoteConversationView";
import { RemoteWorkspaceMenu } from "./RemoteWorkspaceMenu";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
    startDragging: () => Promise.resolve(),
  }),
}));
vi.mock("./remoteToolTarget", () => ({
  fetchRemoteToolTarget: vi.fn(async () => null),
  forgetRemoteToolTargets: vi.fn(),
}));
vi.mock("./remotePeerClient", () => ({
  deleteRemoteWorkspace: vi.fn(async () => {}),
  setRemoteWorkspacePinned: vi.fn(async () => {}),
}));

beforeEach(() => {
  document.body.innerHTML = "";
});

/** Escape closes the workspace menu without acting — the layer's own dismissal. */
it("closes the workspace menu when the reader dismisses it", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<RemoteWorkspaceMenu desktopId="desktop_a" name="图表脚本" onChanged={vi.fn()} pinned={false} workspaceId="ws_9" />);
  });

  await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label^='Workspace actions']")!.click());
  expect(container.querySelector("[role='menu']")).not.toBeNull();

  await act(async () => {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  });
  expect(container.querySelector("[role='menu']")).toBeNull();
  await act(async () => root.unmount());
});

function entry(id: string, role: "user" | "assistant", text: string): SessionEntry {
  return { blocks: [{ kind: "text", text }], createdAtMs: 1, id, kind: "message", role } as unknown as SessionEntry;
}

async function mountView(messages: AgentMessage[], onFork: (source: string, forkable: boolean) => void) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteConversationView
        approvals={[]}
        approvalErrors={{}}
        approvalPending={null}
        composer={<div />}
        desktopId="desktop_a"
        error={null}
        hasMore={false}
        leftPanelExpanded
        live={null}
        loading={false}
        loadingOlder={false}
        messages={messages}
        onContinueRun={vi.fn()}
        onDecideApproval={vi.fn()}
        onFork={onFork}
        onLoadOlder={vi.fn()}
        onOpenUsage={vi.fn()}
        onRetry={vi.fn()}
        onToggleLeftPanel={vi.fn()}
        peer={undefined}
        persistedEntryIds={new Set()}
        sessionId="sess_1"
        streaming={false}
        title="A conversation"
      />,
    );
  });
  return {
    container,
    fork: () => container.querySelector<HTMLButtonElement>("button[title='Fork']"),
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

/**
 * A reply with no prompt before it offers no fork.
 *
 * The host forks at a *user* entry, so a reply with no preceding prompt has
 * nothing to branch at — the control is absent rather than present-and-refusing.
 */
it("offers no fork for a reply with no prompt before it", async () => {
  const onFork = vi.fn();
  const view = await mountView(entriesToMessages([entry("a1", "assistant", "a reply on its own")]), onFork);
  expect(view.fork()).toBeNull();
  await view.unmount();
});

/**
 * A prompt the host cannot address is not forked at.
 *
 * The id a fork is addressed by is the host's stored entry id; a message the
 * projector built without one (a live push) would be refused there, so nothing is
 * sent.
 */
it("does not fork when the prompt has no stored id", async () => {
  const onFork = vi.fn();
  const messages = entriesToMessages([entry("u1", "user", "do it"), entry("a1", "assistant", "done")]);
  // The host's id is what the projection carries over; strip it, as a live push
  // would arrive without one.
  const withoutSource = messages.map(message => ({ ...message, sourceEntryId: undefined }));
  const view = await mountView(withoutSource, onFork);

  const fork = view.fork();
  if (fork)
    await act(async () => fork.click());
  expect(onFork).not.toHaveBeenCalled();
  await view.unmount();
});
