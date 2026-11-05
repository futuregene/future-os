// @vitest-environment jsdom
import type { ComponentProps } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ThreadListItem } from "./ThreadListItem";

vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => undefined }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function props(): ComponentProps<typeof ThreadListItem> {
  return {
    active: false,
    menuOpen: false,
    thread: {
      id: "thread",
      agentSessionId: "thread",
      title: "A long conversation title that should retain its available width",
      mode: "chat",
      workspaceId: "workspace",
      status: "active",
      pinned: false,
      readonly: false,
      createdAt: 0,
      updatedAt: 0,
    },
    onDeleteThread: vi.fn(),
    onMenuOpenChange: vi.fn(),
    onRenameThread: vi.fn(),
    onRestoreThread: vi.fn(),
    onSelectThread: vi.fn(),
    onTogglePinThread: vi.fn(),
    onToggleExpanded: vi.fn(),
  };
}

describe("thread row title space", () => {
  // Row start = the leaf title column (chat 16 / workspace 28, unchanged); each
  // level adds 16px of toggle + 4px of gap, so a child's start is its parent's
  // title column and descendant titles line up with the parent title.
  it.each([
    { compact: false, depth: 0, hasChildren: false, padding: 16 },
    { compact: false, depth: 0, hasChildren: true, padding: 16 },
    { compact: false, depth: 1, hasChildren: false, padding: 36 },
    { compact: false, depth: 1, hasChildren: true, padding: 36 },
    { compact: false, depth: 2, hasChildren: false, padding: 56 },
    { compact: true, depth: 0, hasChildren: false, padding: 28 },
    { compact: true, depth: 0, hasChildren: true, padding: 28 },
    { compact: true, depth: 1, hasChildren: false, padding: 48 },
    { compact: true, depth: 1, hasChildren: true, padding: 48 },
    { compact: true, depth: 2, hasChildren: false, padding: 68 },
    { compact: true, depth: 2, hasChildren: true, padding: 68 },
  ])("uses only the necessary gutter ($compact, depth=$depth, children=$hasChildren)", ({ compact, depth, hasChildren, padding }) => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    const p = props();
    act(() => root.render(<ThreadListItem {...p} compact={compact} depth={depth} hasChildren={hasChildren} />));
    try {
      const row = container.firstElementChild as HTMLElement;
      const title = row.querySelector<HTMLElement>("span.truncate")!;
      const select = row.querySelector<HTMLButtonElement>("button[title]")!;
      const expander = row.querySelector<HTMLButtonElement>("button[aria-expanded]:not([aria-haspopup])");
      expect(row.style.paddingLeft).toBe(`${padding}px`);
      expect(title.textContent).toBe(p.thread.title);
      expect(select.title).toBe(p.thread.title);
      expect(title.classList.contains("min-w-0")).toBe(true);
      expect(title.classList.contains("flex-1")).toBe(true);
      expect(expander !== null).toBe(hasChildren);
      if (expander) {
        expect(title.previousElementSibling).toBe(expander);
        // The expander always sits inline in the row's own content flow, for
        // workspace (compact) rows too: an absolutely placed workspace expander
        // landed in the exact column of the workspace group's collapse chevron.
        expect(expander.classList.contains("absolute")).toBe(false);
        expect(expander.classList.contains("relative")).toBe(true);
        expect(expander.style.left).toBe("");
        // A +/− tree toggle, never the chevron the workspace/section headers
        // use (the two read as one control otherwise).
        const toggle = expander.querySelector("svg")!;
        expect(toggle.classList.contains("lucide-plus")).toBe(true);
        expect(toggle.classList.contains("fill-current")).toBe(false);
        act(() => expander.click());
        expect(p.onToggleExpanded).toHaveBeenCalledWith(p.thread);
        expect(p.onSelectThread).not.toHaveBeenCalled();
        act(() => root.render(<ThreadListItem {...p} compact={compact} depth={depth} hasChildren expanded />));
        expect(expander.getAttribute("aria-expanded")).toBe("true");
        expect(expander.querySelector("svg")!.classList.contains("lucide-minus")).toBe(true);
      }
      else {
        expect(title.previousElementSibling).toBe(select);
      }
      // A child row carries no toggle column of its own: its start (the padding
      // above) *is* the parent's title column.
      if (depth > 0)
        expect(row.querySelector("span.size-4")).toBeNull();
      // Header chevrons stay the headers' own: no row renders one.
      expect(row.querySelector("svg.lucide-chevron-right, svg.lucide-chevron-down")).toBeNull();
      act(() => select.click());
      expect(p.onSelectThread).toHaveBeenCalledExactlyOnceWith(p.thread);
    }
    finally {
      act(() => root.unmount());
      container.remove();
    }
  });
});

describe("a fork's marker in the row title", () => {
  // The host names a fork "<parent> (fork)" (host/src/agent_bridge/session.rs),
  // so the marker is part of the string the row draws. The row draws one
  // truncated line, and that used to cut the marker off the end of a long title
  // — "make install 全流程安装成功 (fo…" — which left a fork recognisable only by
  // an indent every other child shares.
  const parent = "make install 全流程安装成功的一段很长的标题";
  const forked = `${parent} (fork)`;

  function mount(title: string) {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    const p = props();
    p.thread = { ...p.thread, title };
    act(() => root.render(<ThreadListItem {...p} />));
    return {
      dispose: () => {
        act(() => root.unmount());
        container.remove();
      },
      marker: () => container.querySelector<HTMLElement>("[data-fork-marker]"),
      select: () => container.querySelector<HTMLButtonElement>("button[title]")!,
      title: () => container.querySelector<HTMLElement>("span.truncate")!,
    };
  }

  it("draws the marker beside the truncating title instead of inside it", () => {
    const view = mount(forked);
    try {
      // The title is the flexible, truncating element; the marker keeps its own
      // width, so the title is what loses characters when the column is narrow.
      expect(view.title().classList.contains("truncate")).toBe(true);
      expect(view.title().textContent).toBe(parent);
      const marker = view.marker()!;
      expect(marker.textContent).toBe(" (fork)");
      expect(marker.classList.contains("truncate")).toBe(false);
      expect(marker.classList.contains("shrink-0")).toBe(true);
      // Still one title to a reader (and to a screen reader): the marker follows
      // the title in the same row.
      expect(view.title().nextElementSibling).toBe(marker);
      expect(`${view.title().textContent}${marker.textContent}`).toBe(forked);
      // ...and the row's click target and tooltip name the whole thing.
      expect(view.select().title).toBe(forked);
    }
    finally {
      view.dispose();
    }
  });

  it("leaves a title that is not a fork exactly as it was", () => {
    const view = mount(parent);
    try {
      expect(view.marker()).toBeNull();
      expect(view.title().textContent).toBe(parent);
      expect(view.select().title).toBe(parent);
    }
    finally {
      view.dispose();
    }
  });
});
