// @vitest-environment jsdom
import type { MergedConversation } from "../../features/remote-peer/mergeConversations";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteConversationRow } from "./RemoteConversationRow";

/**
 * The remote row: it opens the conversation it shows, and its overflow control
 * opens the *host* menu without also opening the conversation underneath it.
 */

vi.mock("../../features/remote-peer/RemoteConversationMenu", () => ({
  RemoteConversationMenu: ({ conversation, onClose, onOpenRename }: {
    conversation: MergedConversation;
    onClose: () => void;
    onOpenRename: () => void;
  }) => (
    <div data-testid="menu">
      <span>{`menu for ${conversation.id}`}</span>
      <button onClick={onClose} type="button">close</button>
      <button onClick={onOpenRename} type="button">rename</button>
    </div>
  ),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

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

function conversation(overrides: Partial<MergedConversation> = {}): MergedConversation {
  return {
    key: "desktop_a::sess_1",
    desktopId: "desktop_a",
    id: "sess_1",
    threadId: "thread_1",
    title: "Fix the login bug",
    pinned: false,
    streaming: false,
    lastMessageAt: 1,
    mode: "chat",
    workspaceId: null,
    ...overrides,
  };
}

async function mount(props: {
  conversation?: MergedConversation;
  unread?: boolean;
} = {}) {
  const onChanged = vi.fn();
  const onOpen = vi.fn();
  const onOpenRename = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteConversationRow
        active={false}
        conversation={props.conversation ?? conversation()}
        onChanged={onChanged}
        onOpen={onOpen}
        onOpenRename={onOpenRename}
        peer={peer}
        unread={props.unread}
      />,
    );
  });
  return {
    container,
    onChanged,
    onOpen,
    onOpenRename,
    action: () => container.querySelector<HTMLButtonElement>("button[aria-label='More actions']")!,
    row: () => container.querySelector<HTMLButtonElement>("button[title]")!,
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

beforeEach(() => {
  document.body.innerHTML = "";
});

it("opens the conversation it shows", async () => {
  const view = await mount();
  await act(async () => view.row().click());

  expect(view.onOpen).toHaveBeenCalledTimes(1);
  await view.unmount();
});

/**
 * The overflow control sits inside the row. Without stopping the event it would
 * also reach the row's own click handler, and the user asking for the menu would
 * get the conversation opened instead.
 */
it("opens the actions menu without opening the conversation", async () => {
  const view = await mount();
  expect(view.container.querySelector("[data-testid='menu']")).toBeNull();

  await act(async () => view.action().click());

  expect(view.container.querySelector("[data-testid='menu']")).not.toBeNull();
  expect(view.onOpen).not.toHaveBeenCalled();
  await view.unmount();
});

it("closes the menu again", async () => {
  const view = await mount();
  await act(async () => view.action().click());
  const close = [...view.container.querySelectorAll<HTMLButtonElement>("[data-testid='menu'] button")]
    .find(node => node.textContent === "close")!;
  await act(async () => close.click());

  expect(view.container.querySelector("[data-testid='menu']")).toBeNull();
  await view.unmount();
});

/** Clicking anywhere else dismisses the menu, including outside the row. */
it("dismisses the menu when the user clicks away", async () => {
  const view = await mount();
  await act(async () => view.action().click());
  expect(view.container.querySelector("[data-testid='menu']")).not.toBeNull();

  await act(async () => {
    const outside = document.createElement("div");
    document.body.append(outside);
    outside.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  });

  expect(view.container.querySelector("[data-testid='menu']")).toBeNull();
  await view.unmount();
});

/** Rename hands off to the shell's dialog and dismisses the menu. */
it("hands rename to the caller and closes the menu", async () => {
  const view = await mount();
  await act(async () => view.action().click());
  const rename = [...view.container.querySelectorAll<HTMLButtonElement>("[data-testid='menu'] button")]
    .find(node => node.textContent === "rename")!;
  await act(async () => rename.click());

  expect(view.onOpenRename).toHaveBeenCalledTimes(1);
  expect(view.onOpenRename.mock.calls[0]![0]).toMatchObject({ id: "sess_1" });
  expect(view.container.querySelector("[data-testid='menu']")).toBeNull();
  await view.unmount();
});

/** The badge is the machine's icon, and the title names it for the tooltip. */
it("shows the machine's icon and names the machine", async () => {
  const view = await mount();
  expect(view.container.querySelector("[data-testid='remote-source-icon']")).not.toBeNull();
  expect(view.container.querySelector("[title]")!.getAttribute("title")).toContain("Studio iMac");
  await view.unmount();
});

it("shows a running conversation and an unread one differently", async () => {
  const status = (node: HTMLElement) => [...node.querySelectorAll("[aria-label]")]
    .map(child => child.getAttribute("aria-label"))
    .filter(label => label === "Running" || label === "Unread");

  const running = await mount({ conversation: conversation({ streaming: true }) });
  // A running row says so rather than claiming unread.
  expect(status(running.container)).toEqual(["Running"]);
  await running.unmount();

  const unread = await mount({ unread: true });
  expect(status(unread.container)).toEqual(["Unread"]);
  await unread.unmount();

  const quiet = await mount();
  expect(status(quiet.container)).toEqual([]);
  await quiet.unmount();
});
