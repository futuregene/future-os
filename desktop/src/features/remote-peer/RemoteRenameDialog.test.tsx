// @vitest-environment jsdom
import type { MergedConversation } from "./mergeConversations";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteRenameDialog } from "./RemoteRenameDialog";

/**
 * The remote rename dialog. Its contract: the new name is sent to the host that
 * owns the conversation, addressed with the session id the host uses, and a
 * refusal keeps the dialog open with the user's text intact.
 */

const rename = vi.fn<(desktopId: string, address: unknown, name: string) => Promise<unknown>>();
const generate = vi.fn<(desktopId: string, sessionId: string, language: string) => Promise<string>>();
vi.mock("./remotePeerClient", () => ({
  generateRemoteTitle: (...args: Parameters<typeof generate>) => generate(...args),
  renameRemoteConversation: (...args: Parameters<typeof rename>) => rename(...args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function conversation(overrides: Partial<MergedConversation> = {}): MergedConversation {
  return {
    key: "desktop_a::sess_1",
    desktopId: "desktop_a",
    id: "sess_1",
    threadId: "thread_1",
    title: "Old name",
    pinned: false,
    streaming: false,
    lastMessageAt: 1,
    mode: "chat",
    workspaceId: null,
    ...overrides,
  };
}

async function mount(row = conversation()) {
  const onClose = vi.fn();
  const onRenamed = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<RemoteRenameDialog conversation={row} onClose={onClose} onRenamed={onRenamed} />);
  });
  const input = container.querySelector("input")!;
  return {
    container,
    input,
    onClose,
    onRenamed,
    button: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent === label),
    setValue: async (value: string) => {
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    pressEnter: async () => {
      await act(async () => {
        input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
      });
    },
    text: () => container.textContent ?? "",
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
  rename.mockReset().mockResolvedValue(undefined);
  generate.mockReset().mockResolvedValue("A generated name");
});

it("starts from the current title and renames on the owning host", async () => {
  const view = await mount();
  expect(view.input.value).toBe("Old name");

  await view.setValue("Better name");
  await act(async () => view.button("Save")!.click());
  await settle();

  expect(rename).toHaveBeenCalledWith(
    "desktop_a",
    { sessionId: "sess_1", threadId: "thread_1" },
    "Better name",
  );
  expect(view.onRenamed).toHaveBeenCalledTimes(1);
  expect(view.onClose).toHaveBeenCalledTimes(1);
  await view.unmount();
});

it("trims the name it sends", async () => {
  const view = await mount();
  await view.setValue("  padded  ");
  await act(async () => view.button("Save")!.click());
  await settle();

  expect(rename).toHaveBeenCalledWith("desktop_a", expect.anything(), "padded");
  await view.unmount();
});

it("submits on Enter", async () => {
  const view = await mount();
  await view.setValue("Keyboard name");
  await view.pressEnter();
  await settle();

  expect(rename).toHaveBeenCalledTimes(1);
  await view.unmount();
});

it("refuses an empty name", async () => {
  const view = await mount();
  await view.setValue("   ");
  expect(view.button("Save")!.disabled).toBe(true);
  await view.pressEnter();
  await settle();

  expect(rename).not.toHaveBeenCalled();
  await view.unmount();
});

/** A refusal keeps the dialog open so the name can be corrected and retried. */
it("shows a refusal and stays open", async () => {
  rename.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount();
  await view.setValue("New name");
  await act(async () => view.button("Save")!.click());
  await settle();

  expect(view.text()).toContain("peer_not_connected");
  expect(view.onClose).not.toHaveBeenCalled();
  expect(view.onRenamed).not.toHaveBeenCalled();
  await view.unmount();
});

it("renders a non-Error refusal as text", async () => {
  rename.mockRejectedValue("offline");
  const view = await mount();
  await view.setValue("New name");
  await act(async () => view.button("Save")!.click());
  await settle();

  expect(view.text()).toContain("offline");
  await view.unmount();
});

it("cancels without sending anything", async () => {
  const view = await mount();
  await act(async () => view.button("Cancel")!.click());

  expect(rename).not.toHaveBeenCalled();
  expect(view.onClose).toHaveBeenCalledTimes(1);
  await view.unmount();
});

/**
 * A row that belongs to this machine has no host to name it.
 *
 * The merge type allows a local row (`desktopId: null`) even though this dialog
 * is only opened for remote ones — and asking a null host would be a request
 * against nothing.
 */
it("asks no host when the row has none", async () => {
  const view = await mount(conversation({ desktopId: null }));
  await act(async () => view.button("Auto-generate")!.click());

  expect(generate).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * The same "auto-generate" action this app's own rename dialog has, pointed at
 * the host that owns the conversation.
 *
 * It asks the host's own agent, with the conversation's own model — which is
 * why it must be that host's request and not this machine's.
 */
it("asks the host to name the conversation, and fills the field", async () => {
  const view = await mount();

  await act(async () => view.button("Auto-generate")!.click());
  await act(async () => {
    for (let i = 0; i < 4; i += 1)
      await Promise.resolve();
  });

  expect(generate).toHaveBeenCalledWith("desktop_a", "sess_1", expect.any(String));
  expect((view.input as HTMLInputElement).value).toBe("A generated name");
  // Nothing is saved: the local dialog behaves the same way, and a generated
  // title usually needs a word changed first.
  expect(rename).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * The host names the conversation in the language the user is reading, so the
 * UI language is sent with the request rather than left to the host's own.
 */
it("asks for a title in the language the user is reading", async () => {
  const view = await mount();
  await act(async () => view.button("Auto-generate")!.click());
  await act(async () => {
    for (let i = 0; i < 4; i += 1)
      await Promise.resolve();
  });

  const [, , language] = generate.mock.calls[0]!;
  expect(["zh", "en"]).toContain(language);
  await view.unmount();
});

/** A refused generation reports, and leaves what the user typed alone. */
it("keeps the field when the host cannot name it", async () => {
  generate.mockRejectedValue(new Error("empty_title"));
  const view = await mount();
  await view.setValue("my own name");

  await act(async () => view.button("Auto-generate")!.click());
  await act(async () => {
    for (let i = 0; i < 4; i += 1)
      await Promise.resolve();
  });

  expect(view.text()).toContain("empty_title");
  expect((view.input as HTMLInputElement).value).toBe("my own name");
  await view.unmount();
});

/**
 * Enter while a title is being generated must not save.
 *
 * The Save button is disabled during generation, but Enter reaches `submit`
 * directly — and saving here would store whatever the field held *before* the
 * generated title arrived.
 */
it("refuses to save while a title is still being generated", async () => {
  let settle: ((title: string) => void) | null = null;
  generate.mockImplementation(() => new Promise((resolve) => {
    settle = resolve;
  }));
  const view = await mount();

  await act(async () => view.button("Auto-generate")!.click());
  await view.pressEnter();

  expect(rename).not.toHaveBeenCalled();

  await act(async () => settle!("A generated name"));
  await view.unmount();
});
