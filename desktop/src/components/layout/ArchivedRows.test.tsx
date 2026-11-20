// @vitest-environment jsdom
import type { StoredThread } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ArchivedThreadRow } from "./ArchivedRows";

/**
 * The archived shelf's row.
 *
 * Only one thing is asserted here: the shelf is a flat list, so its rows say
 * where a conversation lived — the second line the recent view draws. A shelved
 * workspace's own contents do not pass it; their heading still names it.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const thread: StoredThread = {
  id: "shelved",
  agentSessionId: "shelved",
  title: "Shelved conversation",
  mode: "workspace",
  workspaceId: "ws-research",
  status: "archived",
  pinned: false,
  readonly: false,
  createdAt: 0,
  updatedAt: 0,
};

function mount(whereLabel?: string) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(
    <ArchivedThreadRow
      active={false}
      members={1}
      onDelete={vi.fn()}
      onOpen={vi.fn()}
      onRename={vi.fn()}
      onRestore={vi.fn()}
      thread={thread}
      whereLabel={whereLabel}
    />,
  ));
  return {
    dispose: () => {
      act(() => root.unmount());
      container.remove();
    },
    row: () => container.firstElementChild as HTMLElement,
    subline: () => container.querySelector<HTMLElement>("[data-row-subline]"),
  };
}

describe("a shelved row on the loose shelf", () => {
  it("says which workspace the conversation lived in", () => {
    const view = mount("Research");
    try {
      expect(view.subline()?.textContent).toBe("Research");
      // Two lines need a row that can grow.
      expect(view.row().className).toContain("py-1");
      expect(view.row().className).not.toContain("h-7");
    }
    finally {
      view.dispose();
    }
  });

  it("stays one line inside a shelved workspace's own list", () => {
    // Its heading is right above it, still naming the workspace.
    const view = mount();
    try {
      expect(view.subline()).toBeNull();
      expect(view.row().className).toContain("h-7");
    }
    finally {
      view.dispose();
    }
  });
});
