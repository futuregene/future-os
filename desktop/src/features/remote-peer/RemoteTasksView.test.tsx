// @vitest-environment jsdom
import type { RemotePeer } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteTasksView } from "./RemoteTasksView";

/**
 * Another computer's tasks, opened from its settings page.
 *
 * The editor itself is covered where it lives; what this file is for is the
 * wiring around it — that the host's models are what the form offers, and that a
 * run's "open the conversation" action resolves to the right session on that
 * host.
 */

vi.mock("../tasks/TasksView", () => ({
  TasksView: (props: {
    backend: { kind: string };
    modelOptions: { id: string }[];
    onOpenThread: (threadId: string) => void;
  }) => (
    <div
      data-backend={props.backend.kind}
      data-models={props.modelOptions.map(model => model.id).join(",")}
      data-testid="tasks-view"
    >
      <button onClick={() => props.onOpenThread("th_1")} type="button">open</button>
    </div>
  ),
}));

const requests = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: requests.invoke }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function peer(): RemotePeer {
  return {
    agentAvailable: true,
    bridgeInstanceId: "b",
    connected: true,
    desktopId: "desktop_a",
    error: null,
    features: [],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
  };
}

async function mount(props: {
  onOpenSession?: (desktopId: string, sessionId: string) => void;
} = {}) {
  const onOpenSession = props.onOpenSession ?? vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteTasksView
        leftPanelExpanded
        onBack={vi.fn()}
        onOpenSession={onOpenSession}
        onToggleLeftPanel={vi.fn()}
        peer={peer()}
      />,
    );
  });
  await settle();
  return {
    container,
    onOpenSession,
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 6; i += 1)
      await Promise.resolve();
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
  requests.invoke.mockReset();
});

/** The host's own models, and a backend that is the host's. */
it("points the editor at the host, with the host's models", async () => {
  requests.invoke.mockImplementation(async (command: string) =>
    command === "remote_peer_request"
      ? { models: [{ id: "deepseek-chat", label: "DeepSeek", provider: "deepseek", reasoning: true }] }
      : undefined);
  const view = await mount();
  const editor = view.container.querySelector("[data-testid='tasks-view']")!;

  expect((editor as HTMLElement).dataset.backend).toBe("remote");
  // Prefixed with the provider, the same reference the host resolves.
  expect((editor as HTMLElement).dataset.models).toBe("deepseek/deepseek-chat");
  expect(requests.invoke).toHaveBeenCalledWith("remote_peer_request", {
    desktopId: "desktop_a",
    command: { type: "list_models" },
    lane: "list",
  });
  await view.unmount();
});

/** A failed model read is reported rather than leaving an empty picker. */
it("reports a failed read of the host's models", async () => {
  requests.invoke.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount();

  expect(view.text()).toContain("peer_not_connected");
  await view.unmount();
});

/**
 * Opening a run's conversation means translating its thread id.
 *
 * The host names those two ids differently and they are not interchangeable, so
 * the session is looked up in the host's catalogue rather than assumed.
 */
it("opens a run's conversation by looking its session up on the host", async () => {
  requests.invoke.mockImplementation(async (command: string) => {
    if (command === "remote_peer_request")
      return { models: [] };
    if (command === "remote_peer_sessions") {
      return {
        sessions: [
          { sessionId: "s_other", threadId: "th_x", title: "other" },
          { sessionId: "s_wanted", threadId: "th_1", title: "wanted" },
        ],
      };
    }
    return undefined;
  });
  const view = await mount();

  await act(async () => {
    view.container.querySelector<HTMLButtonElement>("[data-testid='tasks-view'] button")!.click();
  });
  await settle();

  expect(view.onOpenSession).toHaveBeenCalledWith("desktop_a", "s_wanted");
  await view.unmount();
});

/** A run whose conversation is gone says so instead of opening a stranger's. */
it("refuses to open a run whose conversation is not in the catalogue", async () => {
  requests.invoke.mockImplementation(async (command: string) => {
    if (command === "remote_peer_request")
      return { models: [] };
    if (command === "remote_peer_sessions")
      return { sessions: [{ sessionId: "s_other", threadId: "th_x", title: "other" }] };
    return undefined;
  });
  const view = await mount();

  await act(async () => {
    view.container.querySelector<HTMLButtonElement>("[data-testid='tasks-view'] button")!.click();
  });
  await settle();

  expect(view.onOpenSession).not.toHaveBeenCalled();
  expect(view.text()).toContain("no longer there");
  await view.unmount();
});

/** A failed catalogue read is reported, and nothing is opened. */
it("reports a failed catalogue read rather than opening the wrong thing", async () => {
  requests.invoke.mockImplementation(async (command: string) => {
    if (command === "remote_peer_request")
      return { models: [] };
    throw new Error("catalog_unavailable");
  });
  const view = await mount();

  await act(async () => {
    view.container.querySelector<HTMLButtonElement>("[data-testid='tasks-view'] button")!.click();
  });
  await settle();

  expect(view.onOpenSession).not.toHaveBeenCalled();
  expect(view.text()).toContain("catalog_unavailable");
  await view.unmount();
});
