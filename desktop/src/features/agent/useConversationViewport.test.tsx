// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useMessagePaging } from "./useMessagePaging";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function message(id: string): AgentMessage {
  return { id, role: "user", content: id, status: "complete" } as AgentMessage;
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function setup() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const heights = new Map<string, number>();
  let resize!: () => void;
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        resize = callback;
      }

      observe() {}
      disconnect() {}
    },
  );
  const rows = () =>
    Array.from(container.querySelectorAll<HTMLElement>("[data-message-id]"));
  Object.defineProperty(container, "clientHeight", { get: () => 100 });
  Object.defineProperty(container, "scrollHeight", {
    get: () =>
      rows().reduce(
        (sum, row) => sum + (heights.get(row.dataset.messageId!) ?? 100),
        0,
      ),
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    function (this: HTMLElement) {
      if (this === container)
        return { top: 0, bottom: 100 } as DOMRect;
      const index = rows().indexOf(this);
      const top
        = rows()
          .slice(0, index)
          .reduce(
            (sum, row) => sum + (heights.get(row.dataset.messageId!) ?? 100),
            0,
          ) - container.scrollTop;
      return {
        top,
        bottom: top + (heights.get(this.dataset.messageId!) ?? 100),
      } as DOMRect;
    },
  );
  let current!: ReturnType<typeof useMessagePaging>;
  let replaceMessages!: (messages: AgentMessage[]) => void;
  let complete!: (page?: AgentMessage[]) => void;
  function Harness() {
    const [messages, setMessages] = useState(() => [
      message("u3"),
      message("u4"),
    ]);
    replaceMessages = setMessages;
    current = useMessagePaging({
      messages,
      scrollRef: { current: container },
      userExchangeCount: 2,
      hasOlderHistory: true,
      loadOlderHistory: beforeCommit =>
        new Promise<void>((resolve) => {
          complete = (page) => {
            if (page) {
              beforeCommit?.(page);
              setMessages(previous => [...page, ...previous]);
            }
            resolve();
          };
        }),
    });
    return (
      <>
        {current.visibleMessages.map(row => (
          <div key={row.id} data-message-id={row.id} />
        ))}
      </>
    );
  }
  const root = createRoot(container);
  act(() => root.render(<Harness />));
  const scroll = (top: number) =>
    act(() => {
      container.scrollTop = top;
      current.handleScroll();
    });
  return {
    container,
    heights,
    scroll,
    current: () => current,
    resize: () => act(() => resize()),
    replace: (messages: AgentMessage[]) => act(() => replaceMessages(messages)),
    complete: async (page?: AgentMessage[]) => act(async () => complete(page)),
    dispose: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

describe("conversation viewport ownership", () => {
  it("captures at commit, corrects late resize, and respects subsequent user movement", async () => {
    const h = setup();
    h.scroll(0);
    act(() => h.current().loadOlder());
    h.scroll(40); // User keeps moving while storage is pending.
    await h.complete([message("u1"), message("u2")]);
    expect(h.container.scrollTop).toBe(240);
    h.heights.set("u1", 180); // Image above the anchor finishes loading.
    h.resize();
    expect(h.container.scrollTop).toBe(320);
    h.scroll(150); // New intent supersedes the old anchor.
    h.heights.set("u1", 200);
    h.resize();
    expect(h.container.scrollTop).toBe(150);
    h.dispose();
  });

  it("does not move or expand the window for an uncommitted page and permits retry", async () => {
    const h = setup();
    h.scroll(30);
    act(() => h.current().loadOlder());
    await h.complete(); // Failed/stale/no-op storage response has no commit.
    expect(h.container.scrollTop).toBe(30);
    expect(h.current().visibleMessages.map(row => row.id)).toEqual([
      "u3",
      "u4",
    ]);
    act(() => h.current().loadOlder());
    await h.complete([message("u2")]);
    expect(h.container.scrollTop).toBe(130);
    h.dispose();
  });

  it("does not evict the window start when a new exchange arrives", () => {
    const h = setup();
    h.scroll(10);
    h.replace([message("u3"), message("u4"), message("u5")]);
    expect(h.current().visibleMessages[0]?.id).toBe("u3");
    expect(h.container.scrollTop).toBe(10);
    h.dispose();
  });

  it("paging overrides near-bottom follow and explicit latest restores follow", async () => {
    const h = setup();
    h.scroll(80); // Inside the 48px auto-follow threshold.
    act(() => h.current().loadOlder());
    await h.complete([message("u2")]);
    expect(h.container.scrollTop).toBe(180);
    act(() => h.current().scrollToLatest());
    h.heights.set("u4", 180);
    h.resize();
    expect(h.container.scrollTop).toBe(h.container.scrollHeight);
    h.dispose();
  });
});
