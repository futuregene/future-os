// @vitest-environment jsdom
import type { RemoteSkillRecommendation } from "./RemoteComposer";
import type { RemotePeer } from "./remotePeerClient";
import { act, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteComposer } from "./RemoteComposer";

/**
 * The remote composer's own contract, plus the skill-recommendation hold.
 *
 * The hold is the part worth testing: a recommendation makes the composer
 * delay* a message, and delaying it wrongly either sends something the user
 * had not agreed to send or drops the message entirely.
 */

const prompt = vi.fn<(desktopId: string, sessionId: string, message: string, uploads: string[]) => Promise<{ sessionId: string }>>();
vi.mock("./remotePeerClient", () => ({
  abortRemoteRun: vi.fn(),
  promptRemoteConversation: (...args: Parameters<typeof prompt>) => prompt(...args),
  uploadRemoteFile: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

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

/**
 * A harness that owns the card the way the shell does.
 *
 * The composer renders whatever card it is given; who decides that is the
 * caller (the recommendation hook's state), so the fake has to be stateful or
 * it would be testing a composer that never receives one.
 */
async function mount(fake: {
  onDismiss?: () => void;
  onEvaluate?: (draft: string) => Promise<{ name: string; description: string } | null>;
  onInstall?: (card: { name: string; description: string }) => Promise<boolean>;
} = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  function Harness() {
    const [card, setCard] = useState<{ name: string; description: string } | null>(null);
    const reco = useMemo<RemoteSkillRecommendation>(() => ({
      card,
      onDismiss: () => {
        fake.onDismiss?.();
        setCard(null);
      },
      onEvaluate: async (draft) => {
        const next = await (fake.onEvaluate?.(draft) ?? Promise.resolve(null));
        setCard(next);
        return next;
      },
      onInstall: async (chosen) => {
        const installed = await (fake.onInstall?.(chosen) ?? Promise.resolve(false));
        if (installed)
          setCard(null);
        return installed;
      },
    }), [card]);
    return (
      <RemoteComposer
        desktopId="desktop_a"
        onSent={vi.fn()}
        peer={peer()}
        sessionId="sess_1"
        skillRecommendation={reco}
      />
    );
  }

  await act(async () => root.render(<Harness />));
  const input = container.querySelector("input")!;
  return {
    container,
    input,
    button: (text: string) =>
      [...container.querySelectorAll("button")].find(node => node.textContent === text),
    text: () => container.textContent ?? "",
    type: async (value: string) => {
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
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
    for (let i = 0; i < 8; i += 1)
      await Promise.resolve();
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
  prompt.mockReset().mockResolvedValue({ sessionId: "sess_1" });
});

/** Without a recommendation the composer sends as it always did. */
it("sends as before when no recommendation is wired", async () => {
  const view = await mount();
  await view.type("hello");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "hello", []);
  await view.unmount();
});

/**
 * A returned card holds the message.
 *
 * The user is deciding whether to install a skill first, and sending in the
 * meantime would send a message they had not agreed to send yet.
 */
it("holds the message while a recommendation is on screen", async () => {
  const card = { name: "pdf-tools", description: "Read PDFs" };
  const view = await mount({ onEvaluate: async () => card });

  await view.type("please summarise this pdf for me");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).not.toHaveBeenCalled();
  expect(view.text()).toContain("pdf-tools");
  await view.unmount();
});

/**
 * Installing sends the message with the skill selected.
 *
 * `onInstall` returning false means the skill could not be installed; the card
 * stays and nothing is sent, because the message was written for that skill.
 */
it("sends with the skill once it is installed", async () => {
  const card = { name: "pdf-tools", description: "Read PDFs" };
  const onInstall = vi.fn().mockResolvedValue(true);
  const view = await mount({ onEvaluate: async () => card, onInstall });

  await view.type("please summarise this pdf");
  // The card comes from asking the host on send, so the send is what puts it up.
  await act(async () => view.button("Send")!.click());
  await settle();
  await act(async () => view.button("Install & use")!.click());
  await settle();

  expect(onInstall).toHaveBeenCalledWith(card);
  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", expect.stringContaining("/pdf-tools"), []);
  await view.unmount();
});

it("holds the message when the skill could not be installed", async () => {
  const card = { name: "pdf-tools", description: "Read PDFs" };
  const view = await mount({
    onEvaluate: async () => card,
    onInstall: vi.fn().mockResolvedValue(false),
  });

  await view.type("please summarise this pdf");
  // The card comes from asking the host on send, so the send is what puts it up.
  await act(async () => view.button("Send")!.click());
  await settle();
  await act(async () => view.button("Install & use")!.click());
  await settle();

  expect(prompt).not.toHaveBeenCalled();
  expect(view.text()).toContain("pdf-tools");
  await view.unmount();
});

/** Dismissing sends the message without the skill. */
it("sends the message when the card is dismissed", async () => {
  const card = { name: "pdf-tools", description: "Read PDFs" };
  const onDismiss = vi.fn();
  const view = await mount({ onDismiss, onEvaluate: async () => card });

  await view.type("please summarise this pdf");
  await act(async () => view.button("Send")!.click());
  await settle();
  await act(async () => view.button("Send without it")!.click());
  await settle();

  expect(onDismiss).toHaveBeenCalled();
  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "please summarise this pdf", []);
  await view.unmount();
});

/**
 * A plain send while the card is up means "send without it".
 *
 * The send button stays live in that state (only the recommend wait disables
 * it), so returning silently would read as a broken button.
 */
it("treats a plain send on an open card as dismiss-and-send", async () => {
  const card = { name: "pdf-tools", description: "Read PDFs" };
  const view = await mount({ onEvaluate: async () => card });

  await view.type("please summarise this pdf");
  // First send puts the card up and holds the message...
  await act(async () => view.button("Send")!.click());
  await settle();
  expect(prompt).not.toHaveBeenCalled();

  // ...and a plain send with the card up means "send it anyway".
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "please summarise this pdf", []);
  await view.unmount();
});

/**
 * A failed install leaves the card up and the message unsent.
 *
 * The message was written for a skill that is not there; sending it anyway
 * would be a different message from the one the user agreed to.
 */
it("reports a failed install and keeps the card", async () => {
  const card = { name: "pdf-tools", description: "Read PDFs" };
  const view = await mount({
    onEvaluate: async () => card,
    onInstall: async () => {
      throw new Error("skill_not_found");
    },
  });

  await view.type("please summarise this pdf");
  await act(async () => view.button("Send")!.click());
  await settle();
  await act(async () => view.button("Install & use")!.click());
  await settle();

  expect(view.text()).toContain("skill_not_found");
  expect(prompt).not.toHaveBeenCalled();
  await view.unmount();
});

/** A refusal or a timeout sends normally: recommendation is best-effort. */
it("sends normally when the host recommends nothing", async () => {
  const view = await mount({ onEvaluate: async () => null });

  await view.type("please summarise this pdf");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "please summarise this pdf", []);
  await view.unmount();
});

/**
 * The draft is asked about once per send, and the send button is closed while
 * the answer is outstanding — the message that gets sent must be the one that
 * was evaluated.
 */
it("closes the send button while the host is being asked", async () => {
  let answer: ((card: null) => void) | null = null;
  const onEvaluate = vi.fn(() => new Promise<null>((resolve) => {
    answer = resolve;
  }));
  const view = await mount({ onEvaluate });

  await view.type("please summarise this pdf");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(onEvaluate).toHaveBeenCalledTimes(1);
  expect(view.button("Send")!.disabled).toBe(true);
  expect(prompt).not.toHaveBeenCalled();

  await act(async () => answer?.(null));
  await settle();
  expect(prompt).toHaveBeenCalledTimes(1);
  await view.unmount();
});

/** A message with no words is not evaluated: there is nothing to recommend for. */
it("does not ask about an empty draft", async () => {
  const onEvaluate = vi.fn().mockResolvedValue(null);
  const view = await mount({ onEvaluate });

  await act(async () => view.button("Send")!.click());
  await settle();

  expect(onEvaluate).not.toHaveBeenCalled();
  expect(prompt).not.toHaveBeenCalled();
  await view.unmount();
});
