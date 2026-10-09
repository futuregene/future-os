// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteComposer } from "./RemoteComposer";

/**
 * The composer's contract is about *which machine* a prompt or a stop goes to,
 * and about not losing the user's text: a prompt that failed on the wire must
 * not cost them the paragraph they wrote.
 */

const prompt = vi.fn<(desktopId: string, sessionId: string, message: string) => Promise<{ sessionId: string; threadId: string }>>();
const abort = vi.fn<(desktopId: string, sessionId: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", () => ({
  promptRemoteConversation: (...args: Parameters<typeof prompt>) => prompt(...args),
  abortRemoteRun: (...args: Parameters<typeof abort>) => abort(...args),
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

async function mount(props: {
  onCreated?: (sessionId: string) => void;
  onSent?: () => void;
  sessionId?: string;
  streaming?: boolean;
} = {}) {
  const onCreated = props.onCreated ?? vi.fn();
  const onSent = props.onSent ?? vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteComposer
        desktopId="desktop_a"
        onCreated={onCreated}
        onSent={onSent}
        peer={peer}
        sessionId={props.sessionId ?? "sess_1"}
        streaming={props.streaming}
      />,
    );
  });
  const input = container.querySelector("input")!;
  const button = (label: string) => [...container.querySelectorAll("button")]
    .find(node => node.textContent === label);
  return {
    button,
    container,
    input,
    onCreated,
    onSent,
    text: () => container.textContent ?? "",
    type: async (value: string) => {
      await act(async () => {
        // React reads the value off the event, so set it then dispatch input.
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    press: async (key: string) => {
      await act(async () => {
        input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key }));
      });
    },
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  prompt.mockReset().mockResolvedValue({ sessionId: "sess_1", threadId: "thread_1" });
  abort.mockReset().mockResolvedValue(undefined);
});

it("prompts the conversation it belongs to and clears only on success", async () => {
  const view = await mount();
  await view.type("hello host");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "hello host");
  expect(view.onSent).toHaveBeenCalledTimes(1);
  expect(view.input.value).toBe("");
  // An existing conversation was not created, so nothing adopts an id.
  expect(view.onCreated).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * The draft case: with no session id the host creates the conversation, and the
 * ids in the ack are the only way the caller can open it.
 */
it("adopts the conversation the host created for a draft", async () => {
  prompt.mockResolvedValue({ sessionId: "sess_new", threadId: "thread_new" });
  const view = await mount({ sessionId: "" });
  await view.type("first message");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "", "first message");
  expect(view.onCreated).toHaveBeenCalledWith("sess_new");
  expect(view.onSent).toHaveBeenCalledTimes(1);
  await view.unmount();
});

it("keeps the text when the host refuses the prompt", async () => {
  prompt.mockRejectedValue(new Error("agent_unavailable"));
  const view = await mount();
  await view.type("do not lose me");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(view.text()).toContain("agent_unavailable");
  expect(view.input.value).toBe("do not lose me");
  expect(view.onSent).not.toHaveBeenCalled();
  await view.unmount();
});

it("sends on Enter, but not on Shift+Enter", async () => {
  const view = await mount();
  await view.type("via keyboard");
  await view.press("Enter");
  await settle();
  expect(prompt).toHaveBeenCalledTimes(1);

  prompt.mockClear();
  await view.type("newline");
  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter", shiftKey: true }));
  });
  await settle();
  expect(prompt).not.toHaveBeenCalled();
  await view.unmount();
});

/** Whitespace alone is not a prompt. */
it("refuses to send an empty message", async () => {
  const view = await mount();
  expect(view.button("Send")!.disabled).toBe(true);
  await view.type("   ");
  await act(async () => view.button("Send")!.click());
  await settle();
  expect(prompt).not.toHaveBeenCalled();
  await view.unmount();
});

/** Stop replaces Send while a run is in flight, since the host would refuse a second prompt. */
it("stops a run in flight instead of sending", async () => {
  const view = await mount({ streaming: true });

  expect(view.button("Stop")).toBeTruthy();
  expect(view.button("Send")).toBeUndefined();
  await act(async () => view.button("Stop")!.click());
  await settle();

  expect(abort).toHaveBeenCalledWith("desktop_a", "sess_1");
  expect(view.onSent).toHaveBeenCalledTimes(1);
  await view.unmount();
});

/** Typing during a run brings Send back: the user's next message is not lost. */
it("shows Send again once the user types during a run", async () => {
  const view = await mount({ streaming: true });
  await view.type("a follow-up");
  expect(view.button("Send")).toBeTruthy();
  expect(view.button("Stop")).toBeUndefined();
  await view.unmount();
});

it("surfaces a failed stop", async () => {
  abort.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount({ streaming: true });
  await act(async () => view.button("Stop")!.click());
  await settle();

  expect(view.text()).toContain("peer_not_connected");
  await view.unmount();
});

it("ignores a second stop while one is in flight", async () => {
  let release: (() => void) | null = null;
  abort.mockImplementation(() => new Promise((resolve) => {
    release = () => resolve(undefined);
  }));
  const view = await mount({ streaming: true });

  await act(async () => view.button("Stop")!.click());
  // The label changes while stopping, so address it by position.
  await act(async () => view.button("Stopping…")!.click());
  expect(abort).toHaveBeenCalledTimes(1);

  await act(async () => {
    release?.();
  });
  await settle();
  await view.unmount();
});

/** The composer states where the prompt runs, using the host's local name. */
it("names the machine the prompt runs on", async () => {
  const view = await mount();
  expect(view.text()).toContain("Studio iMac");
  await view.unmount();
});
