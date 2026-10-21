// @vitest-environment jsdom
import type { RemoteWorkspaceRow } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { RemoteDraftTargetChooser } from "./RemoteDraftTargetChooser";

/**
 * The row under a remote draft answers the question the local new-conversation
 * screen asks before the first prompt: is this conversation in a workspace on
 * that machine, or a plain chat?
 *
 * The contract is narrow and worth pinning down, because every answer here is
 * carried to the host as an id when the session is created: the choice must be
 * reversible in both directions, the menu must show *that* machine's folders,
 * and a list that could not be read must not be presented as an empty disk.
 */

const workspaces: RemoteWorkspaceRow[] = [
  { id: "ws_1", name: "future-os", path: "/Users/me/code/future-os", pinned: true },
  { id: "ws_2", name: "notes", path: "/Users/me/notes", pinned: false },
];

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

async function mount(props: {
  error?: string | null;
  loading?: boolean;
  onOpenWorkspace?: () => void;
  onSelect?: (workspaceId: string | null) => void;
  rows?: RemoteWorkspaceRow[];
  workspaceId?: string | null;
} = {}) {
  const onSelect = props.onSelect ?? vi.fn();
  const onOpenWorkspace = props.onOpenWorkspace;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const resolved = {
    error: props.error ?? null,
    loading: props.loading ?? false,
    onOpenWorkspace,
    onSelect,
    rows: props.rows ?? workspaces,
    workspaceId: props.workspaceId ?? null,
  };
  const element = (overrides: Partial<typeof resolved> = {}) => (
    <RemoteDraftTargetChooser {...resolved} {...overrides} />
  );
  await act(async () => {
    root.render(element());
  });

  const buttons = () => [...container.querySelectorAll<HTMLButtonElement>("button")];
  /** A control of the row itself, by the words on it. */
  const button = (label: string) => buttons().find(node => node.textContent === label)!;
  const menu = () => container.querySelector<HTMLElement>("[role=menu]");
  const menuItems = () => [...(menu()?.querySelectorAll<HTMLButtonElement>("[role=menuitem]") ?? [])];
  /** A workspace row, whose text is the name followed by that machine's path. */
  const menuRow = (name: string) => menuItems().find(node => node.textContent?.startsWith(name));
  return {
    button,
    container,
    menu,
    menuItems,
    menuRow,
    onOpenWorkspace,
    onSelect,
    rerender: async (overrides: Partial<typeof resolved>) => {
      await act(async () => root.render(element(overrides)));
    },
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

/** A draft with no workspace starts as a plain conversation, and says so. */
it("starts on a plain conversation and explains what that is", async () => {
  const view = await mount();
  expect(view.text()).toContain("Start quickly in a temporary workspace FutureOS manages on that computer.");
  // The workspace button names the choice, not the host: which machine this is
  // belongs to the header's badge.
  expect(view.button("Workspaces")).toBeTruthy();
  expect(view.button("Chat")).toBeTruthy();
  await view.unmount();
});

/**
 * The menu is that host's own workspace list, and a row picks it.
 *
 * The path comes with the name because two folders on another machine can share
 * a name, and this side has no other way to tell them apart.
 */
it("lists the host's workspaces with their paths and hands the id back", async () => {
  const view = await mount();
  await act(async () => view.button("Workspaces").click());

  expect(view.menu()).not.toBeNull();
  expect(view.menuRow("future-os")?.textContent).toContain("/Users/me/code/future-os");
  expect(view.menuRow("notes")?.textContent).toContain("/Users/me/notes");

  await act(async () => view.menuRow("notes")!.click());
  expect(view.onSelect).toHaveBeenCalledWith("ws_2");
  // Picking an answer closes the question.
  expect(view.menu()).toBeNull();
  await view.unmount();
});

/** A draft that is in a workspace names it, and explains that instead. */
it("names the chosen workspace and its own hint", async () => {
  const view = await mount({ workspaceId: "ws_2" });
  expect(view.button("notes")).toBeTruthy();
  expect(view.text()).toContain("Use a project folder and its workspace resources on that computer.");
  expect(view.text()).not.toContain("Start quickly in a temporary workspace");

  await act(async () => view.button("notes").click());
  // The row it is in is marked, so the menu answers "where am I" as well.
  const row = view.menuRow("notes")!;
  expect(row.textContent).toBe("notes/Users/me/notes");
  expect(row.querySelector("svg.lucide-check")).not.toBeNull();
  expect(view.menuRow("future-os")?.querySelector("svg.lucide-check")).toBeNull();
  await view.unmount();
});

/**
 * The choice is reversible from either control: the chat button beside the
 * workspace button, and the same answer inside the menu.
 */
it("goes back to a plain conversation from the button", async () => {
  const view = await mount({ workspaceId: "ws_1" });
  await act(async () => view.button("Chat").click());
  expect(view.onSelect).toHaveBeenCalledWith(null);
  await view.unmount();
});

it("goes back to a plain conversation from the menu", async () => {
  const view = await mount({ workspaceId: "ws_1" });
  await act(async () => view.button("future-os").click());
  await act(async () => view.menuRow("Chat")!.click());
  expect(view.onSelect).toHaveBeenCalledWith(null);
  expect(view.menu()).toBeNull();
  await view.unmount();
});

/**
 * "Open a workspace" is the host's create form, and it is offered only when
 * this side can address the host at all: a row that cannot open anything would
 * be a control that does nothing.
 */
it("offers to open a folder on that machine only when it can", async () => {
  const without = await mount();
  await act(async () => without.button("Workspaces").click());
  expect(without.menuRow("Open a workspace…")).toBeUndefined();
  await without.unmount();

  const onOpenWorkspace = vi.fn();
  const withForm = await mount({ onOpenWorkspace });
  await act(async () => withForm.button("Workspaces").click());
  await act(async () => withForm.menuRow("Open a workspace…")!.click());
  expect(onOpenWorkspace).toHaveBeenCalledTimes(1);
  expect(withForm.menu()).toBeNull();
  await withForm.unmount();
});

/**
 * Three states the menu must not confuse: a list being read, a list that came
 * back empty, and a read that failed. "That computer has no workspaces" is a
 * fact about the host's disk; an unreachable host or a read in flight is not.
 */
it("distinguishes reading, empty, and a failed read", async () => {
  const reading = await mount({ loading: true, rows: [] });
  await act(async () => reading.button("Workspaces").click());
  expect(reading.text()).toContain("Reading that computer's workspaces…");
  expect(reading.text()).not.toContain("That computer has no workspaces yet.");
  await reading.unmount();

  const empty = await mount({ rows: [] });
  await act(async () => empty.button("Workspaces").click());
  expect(empty.text()).toContain("That computer has no workspaces yet.");
  await empty.unmount();

  const failed = await mount({ error: "peer_not_connected", rows: [] });
  await act(async () => failed.button("Workspaces").click());
  expect(failed.text()).toContain("peer_not_connected");
  expect(failed.text()).not.toContain("That computer has no workspaces yet.");
  await failed.unmount();
});

/**
 * The menu opens upward.
 *
 * The composer rides at the foot of the window (that is the conversation frame's
 * placement), so a panel below the row would be outside the viewport: the same
 * reason the rail's menus flip when they reach the bottom of their list.
 */
it("opens the menu above the row", async () => {
  const view = await mount();
  await act(async () => view.button("Workspaces").click());
  expect(view.menu()!.className).toContain("bottom-full");
  await view.unmount();
});
