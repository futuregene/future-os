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
  stopped: false,
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
  depth?: number;
  expanded?: boolean;
  hasChildren?: boolean;
  onToggleExpanded?: (conversation: MergedConversation) => void;
  onToggleSelection?: (conversation: MergedConversation) => void;
  selected?: boolean;
  selectionMode?: boolean;
  unread?: boolean;
} = {}) {
  const onChanged = vi.fn();
  const onDeletedRemote = vi.fn();
  const onOpen = vi.fn();
  const onOpenRename = vi.fn();
  const onToggleSelection = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteConversationRow
        active={false}
        conversation={props.conversation ?? conversation()}
        depth={props.depth}
        expanded={props.expanded}
        hasChildren={props.hasChildren}
        onChanged={onChanged}
        onDeletedRemote={onDeletedRemote}
        onOpen={onOpen}
        onOpenRename={onOpenRename}
        onToggleExpanded={props.onToggleExpanded}
        onToggleSelection={props.onToggleSelection ?? onToggleSelection}
        peer={peer}
        selected={props.selected}
        selectionMode={props.selectionMode}
        unread={props.unread}
      />,
    );
  });
  return {
    container,
    onChanged,
    onDeletedRemote,
    onOpen,
    onOpenRename,
    onToggleSelection,
    action: () => container.querySelector<HTMLButtonElement>("button[aria-label='More actions']")!,
    checkbox: () => container.querySelector<HTMLInputElement>("input[type='checkbox']")!,
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

/**
 * The indicator is the local row's, labels included.
 *
 * It used to be this row's own: a dot of its own size, in a slot of its own
 * width, with labels from the remote vocabulary ("Unread"). In a list holding
 * both kinds that read as two different things reporting two different states,
 * so the remote row now reports through `RailRowIndicator` — the same ring, the
 * same reserved slot and the same words as the local row beside it.
 */
it("reports a run through the shared indicator, in the local row's words", async () => {
  const labels = (node: HTMLElement) => [...node.querySelectorAll("[aria-label]")]
    .map(child => child.getAttribute("aria-label"))
    .filter(label => label === "Running" || label === "Completed");

  const running = await mount({ conversation: conversation({ streaming: true }) });
  // A running row says so rather than claiming a finished one.
  expect(labels(running.container)).toEqual(["Running"]);
  await running.unmount();

  const unread = await mount({ unread: true });
  expect(labels(unread.container)).toEqual(["Completed"]);
  await unread.unmount();

  const quiet = await mount();
  expect(labels(quiet.container)).toEqual([]);
  await quiet.unmount();
});

/**
 * A conversation on another machine folds its forks exactly as a local one does.
 *
 * The host reports the session each one was forked from, so a fork is an indented
 * row under its parent with the `+`/`−` toggle the local row draws — and that
 * toggle must never reach the row's click target, which would open the parent
 * when the user asked to fold it.
 */
it("folds its forks with the local row's own toggle, without opening the row", async () => {
  const onToggleExpanded = vi.fn();
  const folded = await mount({ depth: 1, hasChildren: true, onToggleExpanded });

  // The tree toggle, not the actions trigger: both announce an expanded state,
  // and only the toggle says what it folds.
  const toggle = folded.container.querySelector<HTMLButtonElement>("button[aria-expanded]:not([aria-haspopup])")!;
  expect(toggle).not.toBeNull();
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  expect(toggle.getAttribute("aria-label")).toContain("Fix the login bug");
  // The child row is one indent in from its parent's start, as a local fork is
  // (the row's own geometry: 16px for a top-level row, 20px per level).
  expect(folded.container.firstElementChild!.getAttribute("style")).toContain("padding-left: 36px");

  await act(async () => toggle.click());
  expect(onToggleExpanded).toHaveBeenCalledTimes(1);
  expect(onToggleExpanded.mock.calls[0]![0]).toMatchObject({ key: "desktop_a::sess_1" });
  // The toggle is its own control: asking to fold is not asking to open.
  expect(folded.onOpen).not.toHaveBeenCalled();
  await folded.unmount();

  const laidOut = await mount({ depth: 1, expanded: true, hasChildren: true });
  expect(laidOut.container.querySelector("button[aria-expanded]:not([aria-haspopup])")!.getAttribute("aria-expanded")).toBe("true");
  await laidOut.unmount();

  // A conversation with no forks has no toggle: the slot is not decoration.
  const leaf = await mount({ depth: 1 });
  expect(leaf.container.querySelector("button[aria-expanded]:not([aria-haspopup])")).toBeNull();
  await leaf.unmount();
});

/**
 * While a batch is being assembled the checkbox takes the actions trigger's
 * place, exactly as it does on a local row.
 *
 * Leaving the `…` on the row put it under the pointer that is ticking rows —
 * the moment the row is answering "in or out of the batch" it offered rename /
 * delete / fork for the single conversation about to be ticked, so the two kinds
 * of row showed two different sets of controls for the same state.
 */
it("shows the batch's checkbox instead of the actions trigger", async () => {
  const view = await mount({ selected: true, selectionMode: true });

  const checkbox = view.checkbox();
  expect(checkbox.checked).toBe(true);
  expect(checkbox.getAttribute("aria-label")).toBe("Select Fix the login bug");
  expect(view.container.querySelector("button[aria-haspopup='menu']")).toBeNull();
  await act(async () => checkbox.click());
  expect(view.onToggleSelection).toHaveBeenCalledTimes(1);
  // Ticking is not opening: the row's click target stays behind the checkbox.
  expect(view.onOpen).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * The menu trigger is the shared one: 20px, out of the layout while hidden.
 *
 * It was 24px and always occupied its space, so the merged list's titles sat a
 * few pixels left of the local list's and moved on hover.
 */
it("uses the shared 20px actions trigger, hidden until hover", async () => {
  const view = await mount();
  const trigger = view.container.querySelector("button[aria-haspopup='menu']")!;
  expect(trigger.className).toContain("size-5");
  expect(trigger.className).toContain("hidden");
  expect(trigger.className).toContain("group-hover/thread:inline-flex");
  await view.unmount();
});

/** Right-click opens the same menu, exactly as on a local row. */
it("opens its menu on right-click", async () => {
  const view = await mount();
  expect(view.container.querySelector("[data-testid='menu']")).toBeNull();

  await act(async () => {
    view.container.firstElementChild!.dispatchEvent(
      new MouseEvent("contextmenu", { bubbles: true, cancelable: true }),
    );
  });

  expect(view.container.querySelector("[data-testid='menu']")).not.toBeNull();
  // The right-click opened the menu, not the conversation behind it.
  expect(view.onOpen).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * The source badge sits after the title. In front of it, every title in the list
 * starts one indent further in for no gain: the badge is a qualifier of the title,
 * not a leading label.
 */
it("shows the source icon after the title, not in front of it", async () => {
  const view = await mount();
  const title = [...view.container.querySelectorAll("span")]
    .find(node => node.textContent === "Fix the login bug")!;
  const icon = view.container.querySelector("[data-testid='remote-source-icon']")!;

  // `compareDocumentPosition` rather than a child index: the assertion is about
  // the order the user reads, which the element order alone would not settle.
  const iconFollowsTitle = title.compareDocumentPosition(icon) & Node.DOCUMENT_POSITION_FOLLOWING;
  expect(iconFollowsTitle).toBeTruthy();
  await view.unmount();
});
