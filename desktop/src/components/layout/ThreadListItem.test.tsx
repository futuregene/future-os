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
  it.each([
    { compact: false, depth: 0, hasChildren: false, padding: 12, spacer: false },
    { compact: false, depth: 0, hasChildren: true, padding: 12, spacer: false },
    { compact: false, depth: 1, hasChildren: false, padding: 28, spacer: true },
    { compact: false, depth: 1, hasChildren: true, padding: 28, spacer: false },
    { compact: true, depth: 0, hasChildren: false, padding: 28, spacer: false },
    { compact: true, depth: 0, hasChildren: true, padding: 28, spacer: false },
    { compact: true, depth: 1, hasChildren: false, padding: 44, spacer: false },
    { compact: true, depth: 1, hasChildren: true, padding: 44, spacer: false },
    { compact: true, depth: 2, hasChildren: false, padding: 60, spacer: false },
    { compact: true, depth: 2, hasChildren: true, padding: 60, spacer: false },
  ])("uses only the necessary gutter ($compact, depth=$depth, children=$hasChildren)", ({ compact, depth, hasChildren, padding, spacer }) => {
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
        expect(expander.classList.contains("absolute")).toBe(compact);
        expect(expander.style.left).toBe(compact ? `${8 + depth * 16}px` : "");
        act(() => expander.click());
        expect(p.onToggleExpanded).toHaveBeenCalledWith(p.thread);
        expect(p.onSelectThread).not.toHaveBeenCalled();
        act(() => root.render(<ThreadListItem {...p} compact={compact} depth={depth} hasChildren expanded />));
        expect(expander.getAttribute("aria-expanded")).toBe("true");
      }
      else if (spacer) {
        expect(title.previousElementSibling?.tagName).toBe("SPAN");
        expect(title.previousElementSibling?.classList.contains("size-4")).toBe(true);
      }
      else {
        expect(title.previousElementSibling).toBe(select);
      }
      act(() => select.click());
      expect(p.onSelectThread).toHaveBeenCalledExactlyOnceWith(p.thread);
    }
    finally {
      act(() => root.unmount());
      container.remove();
    }
  });
});
