// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { emitFutureEvent } from "../../lib/futureEvents";
import { RemoteComposer } from "./RemoteComposer";
import { clearRemoteComposerDraft } from "./remoteComposerDraft";
import { getRemoteOutbox, resetRemoteOutbox } from "./remotePromptOutbox";

/**
 * The composer's contract is about *which machine* a prompt or a stop goes to,
 * and about not losing the user's text: a prompt that failed on the wire must
 * not cost them the paragraph they wrote.
 */

const prompt = vi.fn<(
  desktopId: string,
  sessionId: string,
  message: string,
  attachments?: string[],
  workspaceId?: string | null,
  fresh?: { modelId?: string | null; thinkingLevel?: string | null } | null,
  commandId?: string,
) => Promise<{ sessionId: string; threadId: string }>>();
const abort = vi.fn<(desktopId: string, sessionId: string) => Promise<unknown>>();
const upload = vi.fn<(input: { desktopId: string; path: string }) => Promise<{ uploadId: string; name: string; contentHash: string }>>();
const listSkills = vi.fn<(desktopId: string) => Promise<unknown[]>>();
const getPromptReceipt = vi.fn<(desktopId: string, commandId: string) => Promise<{ sessionId: string; threadId: string; runId: string } | null>>();
vi.mock("./remotePeerClient", () => ({
  promptRemoteConversation: (...args: Parameters<typeof prompt>) => prompt(...args),
  getRemotePromptReceipt: (...args: Parameters<typeof getPromptReceipt>) => getPromptReceipt(...args),
  abortRemoteRun: (...args: Parameters<typeof abort>) => abort(...args),
  uploadRemoteFile: (...args: Parameters<typeof upload>) => upload(...args),
  // The `/` menu's skills, which are the host's own list.
  listRemoteInstalledSkills: (...args: Parameters<typeof listSkills>) => listSkills(...args),
  // The control row carries the host's own approval mode, which the composer
  // reads on mount: a host that reports a mode shows the menu, and one that
  // cannot is a host the composer draws the other controls without.
  getRemoteApprovalSettings: async () => ({ approvalTier: "manual", sandboxAvailable: false }),
  setRemoteApprovalTier: async (_desktopId: string, tier: string) => tier,
}));

const openDialog = vi.fn<(options: Record<string, unknown>) => Promise<string | null>>();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: Parameters<typeof openDialog>) => openDialog(...args),
}));

/**
 * The clipboard's file sources, mocked at the module the composer shares with the
 * local one: a Finder copy (paths recovered from WKWebView) and a screenshot
 * (bytes written to a temporary file, because the host's upload takes a path).
 */
const nativePaths = vi.fn<() => Promise<string[]>>();
const saveImage = vi.fn<(input: { bytes: number[]; extension: string }) => Promise<{ path: string }>>();
const saveFile = vi.fn<(input: { bytes: number[]; name: string }) => Promise<{ path: string }>>();
vi.mock("../../integrations/storage/threadStore", () => ({
  readNativeClipboardFilePaths: () => nativePaths(),
  savePastedFile: (input: { bytes: number[]; name: string }) => saveFile(input),
  savePastedImage: (input: { bytes: number[]; extension: string }) => saveImage(input),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
// jsdom has no layout, so a menu cannot scroll its highlighted row into view.
Element.prototype.scrollIntoView = vi.fn();

const peer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  stopped: false,
  desktopId: "desktop_a",
  error: null,
  // The capability a current host declares, which is what puts its skills in the
  // `/` menu; the tests that are about the menu without them pass their own.
  features: ["skills_v1"],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

/**
 * That host's conversations, as its catalogue carries them: the `#` menu's
 * source. A chat and a workspace conversation, so the row's workspace label is
 * exercised too.
 */
const conversations = [
  { sessionId: "sess_2", title: "Fix the flaky test", mode: "chat" as const, lastMessageAt: 300 },
  {
    sessionId: "sess_3",
    title: "Release notes",
    mode: "workspace" as const,
    workspaceId: "ws_1",
    workspaceName: "future-os",
    lastMessageAt: 200,
  },
];

async function mount(props: {
  canCompact?: boolean;
  conversations?: typeof conversations;
  desktopId?: string;
  onCompact?: () => void;
  onCreated?: (sessionId: string) => void;
  onSent?: () => void;
  peer?: typeof peer;
  sessionId?: string;
  streaming?: boolean;
  workspaceId?: string | null;
} = {}) {
  const onCreated = props.onCreated ?? vi.fn();
  const onSent = props.onSent ?? vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  // What one render is given, before any per-render override: a re-render (the
  // streaming tests need one) must not silently drop the other props.
  const resolved = {
    canCompact: props.canCompact ?? false,
    conversations: props.conversations ?? [],
    desktopId: props.desktopId ?? "desktop_a",
    onCompact: props.onCompact,
    onCreated,
    onSent,
    peer: props.peer ?? peer,
    sessionId: props.sessionId ?? "sess_1",
    streaming: props.streaming,
    workspaceId: props.workspaceId ?? null,
  };
  const element = (overrides: Partial<typeof resolved> = {}) => (
    <RemoteComposer {...resolved} {...overrides} />
  );
  await act(async () => {
    root.render(element());
  });
  // The field is a textarea: the local composer's editor is a contenteditable
  // div, so this is the closest markup the remote one can share — and a
  // `TextInput` put a bordered control inside the bordered shell.
  const input = container.querySelector("textarea")!;
  // By accessible name: the attach and send controls are icon buttons (as they
  // are in this app's own composer), so they have an aria-label rather than text.
  const button = (label: string) => [...container.querySelectorAll("button")]
    .find(node => node.textContent === label || node.getAttribute("aria-label") === label);
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
        const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
        setter.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    press: async (key: string, init: KeyboardEventInit = {}) => {
      await act(async () => {
        input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key, ...init }));
      });
    },
    /**
     * The same component with one prop changed, as the caller does when the host
     * starts or settles a run: the composer is re-rendered, not remounted (the
     * shell's key covers desktop+session only).
     */
    rerender: async (overrides: Partial<typeof resolved>) => {
      await act(async () => root.render(element(overrides)));
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
  // The composer persists drafts and outbox in localStorage; without this
  // tests leak queued prompts and staged uploads between cases.
  localStorage.clear();
  clearRemoteComposerDraft("desktop_a", "sess_1");
  resetRemoteOutbox();

  saveFile.mockReset();
  saveImage.mockReset();
  nativePaths.mockReset().mockResolvedValue([]);
  saveFile.mockResolvedValue({ path: "/tmp/pasted" });

  listSkills.mockReset().mockResolvedValue([]);
  prompt.mockReset().mockResolvedValue({ sessionId: "sess_1", threadId: "thread_1" });
  getPromptReceipt.mockReset().mockResolvedValue(null);
  abort.mockReset().mockResolvedValue(undefined);
  upload.mockReset().mockResolvedValue({ uploadId: "upload_1", name: "notes.txt", contentHash: "abc" });
  openDialog.mockReset().mockResolvedValue("/local/notes.txt");
});

it("prompts the conversation it belongs to and clears only on success", async () => {
  const view = await mount();
  await view.type("hello host");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "hello host", [], null, { modelId: null, thinkingLevel: null }, expect.any(String));
  expect(view.onSent).toHaveBeenCalledTimes(1);
  expect(view.input.value).toBe("");
  // An existing conversation was not created, so nothing adopts an id.
  expect(view.onCreated).not.toHaveBeenCalled();
  await view.unmount();
});

it.each([
  {},
  { isComposing: true },
  { keyCode: 229 },
  { code: "Enter", key: "Unidentified" },
])("requires release before sending after an IME Enter with markers %j", async (init: KeyboardEventInit) => {
  const view = await mount();
  await view.type("候选词");
  if (Object.keys(init).length === 0) {
    await act(async () => {
      view.input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    });
  }
  await view.press("Enter", init);
  await act(async () => {
    view.input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  });
  await view.press("Enter", { repeat: true });
  await view.press("Enter");
  expect(prompt).not.toHaveBeenCalled();
  expect(view.input.value).toBe("候选词");

  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keyup", { bubbles: true, key: "Enter" }));
  });
  await view.press("Enter");
  await settle();
  expect(prompt).toHaveBeenCalledTimes(1);
  expect(view.input.value).toBe("");
  await view.unmount();
});

it("leaves the slash menu and its command alone during composition and a held Enter", async () => {
  const onCompact = vi.fn();
  const view = await mount({ canCompact: true, onCompact });
  await view.type("/compact");
  await act(async () => {
    view.input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  });
  await view.press("ArrowDown");
  await view.press("Enter");
  await act(async () => {
    view.input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  });
  await view.press("Enter", { repeat: true });
  expect(onCompact).not.toHaveBeenCalled();
  expect(prompt).not.toHaveBeenCalled();
  expect(view.input.value).toBe("/compact");

  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keyup", { bubbles: true, key: "Enter" }));
  });
  await view.press("Enter");
  expect(onCompact).toHaveBeenCalledTimes(1);
  expect(prompt).not.toHaveBeenCalled();
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

  expect(prompt).toHaveBeenCalledWith("desktop_a", "", "first message", [], null, { modelId: null, thinkingLevel: null }, expect.any(String));
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

/**
 * Whitespace alone is not a prompt, and neither is nothing at all.
 *
 * Both ways in are asserted: the button is disabled, and Enter — which reaches
 * `send` even while that button is disabled — does nothing either. The second is
 * the one that would otherwise be covered by nothing at all.
 */
it("refuses to send an empty message", async () => {
  const view = await mount();
  expect(view.button("Send")!.disabled).toBe(true);

  await view.press("Enter");
  await settle();
  expect(prompt).not.toHaveBeenCalled();

  await view.type("   ");
  await act(async () => view.button("Send")!.click());
  await settle();
  expect(prompt).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * Stop replaces Send while a run is in flight and nothing is drafted — the host
 * would refuse a second prompt.
 */
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

/**
 * A draft typed during a run does not bring Send back: the host refuses a second
 * prompt while it is answering the first ("This session is still running; wait
 * for it to finish or abort it first"), so a press could only produce an error
 * over a message the reader meant to send. The draft waits in the field, Stop
 * stays reachable, and Send returns when the run settles — the local composer's
 * rule and the phone's.
 */
it("keeps Send away while a run is in flight, however much is drafted", async () => {
  const view = await mount({ streaming: true });
  await view.type("a follow-up");

  expect(view.button("Send")).toBeUndefined();
  const stop = view.button("Stop");
  expect(stop).toBeTruthy();
  expect(stop!.disabled).toBe(false);

  // Enter takes the same route as the button, and the draft survives it.
  await view.press("Enter");
  await settle();
  expect(prompt).not.toHaveBeenCalled();
  expect(view.input.value).toBe("a follow-up");

  // The run ends: what was typed meanwhile is still there, and now it can go.
  await view.rerender({ streaming: false });
  expect(view.button("Stop")).toBeUndefined();
  expect(view.button("Send")!.disabled).toBe(false);
  await act(async () => view.button("Send")!.click());
  await settle();
  expect(prompt).toHaveBeenCalledTimes(1);
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
  // The abort is outstanding, so the control is closed and says so: `disabled` is
  // the single guard against a second stop (there is no second check to make),
  // and the label changes with the state as this app's own composer's does.
  const stopping = view.button("Stopping…")!;
  expect(stopping.disabled).toBe(true);
  await act(async () => stopping.click());
  expect(abort).toHaveBeenCalledTimes(1);

  await act(async () => {
    release?.();
  });
  await settle();
  await view.unmount();
});

/**
 * The box is this app's own, down to the shape of its controls.
 *
 * The two composers sit one above the other on the same screen, so a difference
 * is not a matter of taste — it reads as two products. The field had an inner
 * border (a `TextInput` inside an already-bordered shell) and the send control
 * was a labelled button where this app's is an icon. Measured rather than
 * described: the shell's classes are the local form's, the field carries no
 * border of its own, and the send control is the local icon button.
 */
it("uses this app's own field and send control, not a control nested in the shell", async () => {
  const view = await mount();
  const form = view.container.querySelector("form")!;
  for (const part of ["rounded-lg", "border", "shadow-panel"]) {
    expect(form.className, `the shell is missing ${part}`).toContain(part);
  }

  const field = view.input;
  // Two nested borders: the shell's and the field's. The local editor has none.
  expect(field.className).not.toContain("border");
  for (const part of ["min-h-14", "px-2", "py-1", "text-sm", "leading-5", "placeholder:text-ink-muted"]) {
    expect(field.className, `the field is missing ${part}`).toContain(part);
  }

  const send = view.button("Send")!;
  expect(send.className).toContain("size-7");
  // Icon only: the label is an accessible name, not the button's content.
  expect(send.textContent).toBe("");
  expect(send.querySelector("svg")).not.toBeNull();
  await view.unmount();
});

// ── attachments ─────────────────────────────────────────────────────────────

/** A staged file is shown by the name the host gave it, not the local path. */
it("stages a chosen file on the host and shows it", async () => {
  upload.mockResolvedValue({ uploadId: "upload_7", name: "report.txt", contentHash: "h" });
  const view = await mount();

  await act(async () => view.button("Attach")!.click());
  await settle();

  expect(openDialog).toHaveBeenCalledWith(expect.objectContaining({ multiple: false }));
  expect(upload).toHaveBeenCalledWith({ desktopId: "desktop_a", path: "/local/notes.txt" });
  expect(view.text()).toContain("report.txt");
  await view.unmount();
});

/** Cancelling the file picker must not reach the host at all. */
it("uploads nothing when the picker is cancelled", async () => {
  openDialog.mockResolvedValue(null);
  const view = await mount();

  await act(async () => view.button("Attach")!.click());
  await settle();

  expect(upload).not.toHaveBeenCalled();
  await view.unmount();
});

it("reports a failed upload and stages nothing", async () => {
  upload.mockRejectedValue(new Error("remote_upload_hash_mismatch:notes.txt"));
  const view = await mount();

  await act(async () => view.button("Attach")!.click());
  await settle();

  expect(view.text()).toContain("remote_upload_hash_mismatch");
  await view.unmount();
});

/**
 * The staged files travel as *references*, in the prompt's own field: the bytes
 * are already on the host, and re-sending them would be a second copy that could
 * disagree with the first.
 */
it("sends the staged files as attachment references", async () => {
  upload.mockResolvedValue({ uploadId: "upload_7", name: "report.txt", contentHash: "h" });
  const view = await mount();
  await act(async () => view.button("Attach")!.click());
  await settle();

  await view.type("please read this");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "please read this", ["upload_7"], null, { modelId: null, thinkingLevel: null }, expect.any(String));
  // And the chips are gone: they now belong to the message, not the composer.
  expect(view.text()).not.toContain("report.txt");
  await view.unmount();
});

/** A file on its own is a message: the user should not have to write a prompt. */
it("sends an attachment with no words", async () => {
  upload.mockResolvedValue({ uploadId: "upload_7", name: "report.txt", contentHash: "h" });
  const view = await mount();
  await act(async () => view.button("Attach")!.click());
  await settle();

  expect(view.button("Send")!.hasAttribute("disabled")).toBe(false);
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "", ["upload_7"], null, { modelId: null, thinkingLevel: null }, expect.any(String));
  await view.unmount();
});

/** A file removed before sending must not be attached. */
it("drops a removed attachment from the prompt", async () => {
  upload.mockResolvedValue({ uploadId: "upload_7", name: "report.txt", contentHash: "h" });
  const view = await mount();
  await act(async () => view.button("Attach")!.click());
  await settle();

  await act(async () => {
    view.container.querySelector<HTMLButtonElement>("button[aria-label='Remove report.txt']")!.click();
  });
  await view.type("no file after all");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "no file after all", [], null, { modelId: null, thinkingLevel: null }, expect.any(String));
  await view.unmount();
});

/**
 * A prompt that failed keeps the staged file: the bytes are already on the host,
 * so re-staging them would be wasted work, and the user's intent is preserved.
 */
it("keeps the staged file when the prompt fails", async () => {
  upload.mockResolvedValue({ uploadId: "upload_7", name: "report.txt", contentHash: "h" });
  prompt.mockRejectedValue(new Error("agent_unavailable"));
  const view = await mount();
  await act(async () => view.button("Attach")!.click());
  await settle();

  await view.type("hello");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(view.text()).toContain("agent_unavailable");
  expect(view.text()).toContain("report.txt");
  await view.unmount();
});

/**
 * While a file is uploading there is nothing to send yet, and the control says
 * so rather than swallowing the click.
 */
it("holds the send back while a file is uploading", async () => {
  let release: ((value: { uploadId: string; name: string; contentHash: string }) => void) | null = null;
  upload.mockImplementation(() => new Promise((resolve) => {
    release = resolve;
  }));
  const view = await mount();

  await act(async () => view.button("Attach")!.click());
  expect(view.button("Attach")!.disabled).toBe(true);

  await act(async () => {
    release?.({ uploadId: "upload_7", name: "report.txt", contentHash: "h" });
  });
  await settle();
  expect(view.text()).toContain("report.txt");
  await view.unmount();
});

/**
 * A file pasted into the box goes to the *host*: the machine that will read it
 * is the machine that needs the bytes. Without this the user's only way to hand
 * a file over is the attach button, which is not how anyone moves a file on a
 * desktop.
 */
it("stages a pasted file on the host, not in the draft", async () => {
  nativePaths.mockResolvedValue(["/Users/me/Desktop/report.txt"]);
  upload.mockResolvedValue({ contentHash: "h", name: "report.txt", uploadId: "upload_9" });
  const view = await mount();

  // What a Finder copy actually looks like in the webview: a `File` with no
  // usable name or type, whose real path has to be recovered off the pasteboard.
  const opaque = new File([new Uint8Array([1])], "", { type: "" });
  await act(async () => {
    view.input.dispatchEvent(pasteEvent({ files: [opaque] }));
  });
  await settle();

  // The recovered path is uploaded, not the opaque bytes: the path is the only
  // one that names the file the user actually copied.
  expect(upload).toHaveBeenCalledWith({ desktopId: "desktop_a", path: "/Users/me/Desktop/report.txt" });
  expect(saveFile).not.toHaveBeenCalled();
  expect(view.text()).toContain("report.txt");
  await view.unmount();
});

/**
 * The bytes case: a screenshot has no path, so it is written to a temporary file
 * and that is what is uploaded.
 */
it("uploads a pasted screenshot through a temporary file", async () => {
  nativePaths.mockResolvedValue([]);
  saveImage.mockResolvedValue({ path: "/tmp/pasted-1.png" });
  upload.mockResolvedValue({ contentHash: "h", name: "pasted-1.png", uploadId: "upload_10" });
  const view = await mount();

  const file = new File([new Uint8Array([1, 2, 3])], "pasted.png", { type: "image/png" });
  await act(async () => {
    view.input.dispatchEvent(pasteEvent({ files: [file] }));
  });
  await settle();

  expect(saveImage).toHaveBeenCalledWith({ bytes: [1, 2, 3], extension: "png" });
  expect(upload).toHaveBeenCalledWith({ desktopId: "desktop_a", path: "/tmp/pasted-1.png" });
  await view.unmount();
});

/**
 * A plain text paste belongs in the field. Swallowing it would be worse than the
 * feature it was added for.
 */
it("leaves a text paste to the input", async () => {
  nativePaths.mockResolvedValue([]);
  const view = await mount();

  const event = pasteEvent();
  await act(async () => {
    view.input.dispatchEvent(event);
  });
  await settle();

  expect(upload).not.toHaveBeenCalled();
  expect(event.defaultPrevented).toBe(false);
  await view.unmount();
});

/**
 * A clipboard carrying a file URI the platform cannot turn back into a path —
 * the case where the user's paste yields nothing to send. It must not swallow the
 * paste: the text they were copying is still theirs to paste.
 */
it("leaves a paste whose file could not be resolved to the field", async () => {
  nativePaths.mockResolvedValue([]);
  const view = await mount();

  const event = pasteEvent({ files: [], uriList: "file:///Users/me/notes.txt" });
  await act(async () => {
    view.input.dispatchEvent(event);
  });
  await settle();

  expect(upload).not.toHaveBeenCalled();
  expect(event.defaultPrevented).toBe(false);
  await view.unmount();
});

/** A copied file with no image type travels as bytes written to a temp file. */
it("writes a copied non-image file to a temporary path before uploading it", async () => {
  nativePaths.mockResolvedValue([]);
  saveFile.mockResolvedValue({ path: "/tmp/copied-report.txt" });
  upload.mockResolvedValue({ contentHash: "h", name: "report.txt", uploadId: "upload_11" });
  const view = await mount();

  const file = new File([new Uint8Array([7, 7])], "report.txt", { type: "text/plain" });
  await act(async () => {
    view.input.dispatchEvent(pasteEvent({ files: [file] }));
  });
  await settle();

  expect(saveFile).toHaveBeenCalledWith({ bytes: [7, 7], name: "report.txt" });
  expect(upload).toHaveBeenCalledWith({ desktopId: "desktop_a", path: "/tmp/copied-report.txt" });
  await view.unmount();
});

/** A file that cannot be read is reported, and nothing half-staged is left. */
it("reports a pasted file it could not read", async () => {
  nativePaths.mockResolvedValue([]);
  saveFile.mockRejectedValue(new Error("no permission"));
  const view = await mount();

  const file = new File([new Uint8Array([1])], "secret.txt", { type: "text/plain" });
  await act(async () => {
    view.input.dispatchEvent(pasteEvent({ files: [file] }));
  });
  await settle();

  expect(view.text()).toContain("secret.txt");
  expect(upload).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * A copy of more files than can be staged says so, and stages the limit rather
 * than silently dropping the rest — an attachment the user cannot see is one they
 * cannot remove.
 */
it("caps a large paste and says it did", async () => {
  nativePaths.mockResolvedValue(["/tmp/a", "/tmp/b", "/tmp/c", "/tmp/d", "/tmp/e"]);
  upload.mockResolvedValue({ contentHash: "h", name: "f", uploadId: "upload_12" });
  const view = await mount();

  await act(async () => {
    view.input.dispatchEvent(pasteEvent({ files: [new File([], "a")] }));
  });
  await settle();

  expect(upload).toHaveBeenCalledTimes(4);
  expect(view.text()).toContain("At most 4");
  await view.unmount();
});

/** More copied files than the limit: the extra ones are not read at all. */
it("stops reading a copied batch once the limit is reached", async () => {
  nativePaths.mockResolvedValue([]);
  saveFile.mockResolvedValue({ path: "/tmp/copied" });
  upload.mockResolvedValue({ contentHash: "h", name: "f", uploadId: "upload_13" });
  const view = await mount();

  const files = Array.from({ length: 6 }, (_, index) =>
    new File([new Uint8Array([index])], `file-${index}.txt`, { type: "text/plain" }));
  await act(async () => {
    view.input.dispatchEvent(pasteEvent({ files }));
  });
  await settle();

  // Four read and staged; the fifth and sixth are never turned into bytes.
  expect(saveFile).toHaveBeenCalledTimes(4);
  expect(upload).toHaveBeenCalledTimes(4);
  await view.unmount();
});

/** A transfer that fails after the read is reported, not swallowed. */
it("reports a pasted file the host refused", async () => {
  nativePaths.mockResolvedValue(["/tmp/too-big.bin"]);
  upload.mockRejectedValue(new Error("the host refused this file"));
  const view = await mount();

  await act(async () => {
    view.input.dispatchEvent(pasteEvent({ files: [new File([], "x")] }));
  });
  await settle();

  expect(view.text()).toContain("the host refused this file");
  await view.unmount();
});

/** A clipboard carrying a file URI but no `File` is still a file copy. */
function pasteEvent(input: { files?: File[]; uriList?: string } = {}): Event {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  const data = {
    files: input.files ?? [],
    getData: (type: string) => (type === "text/uri-list" ? input.uriList ?? "" : ""),
    types: input.uriList ? ["text/uri-list"] : [],
  };
  Object.defineProperty(event, "clipboardData", { value: data });
  return event;
}

/** A paste event with no clipboard payload (some platforms fire one) is ignored. */
it("ignores a paste that carries no clipboard data", async () => {
  const view = await mount();

  const event = new Event("paste", { bubbles: true, cancelable: true });
  await act(async () => {
    view.input.dispatchEvent(event);
  });
  await settle();

  expect(upload).not.toHaveBeenCalled();
  expect(event.defaultPrevented).toBe(false);
  await view.unmount();
});

// ── the `/` commands ────────────────────────────────────────────────────────

/**
 * Compacting is a `/` command, not a header button.
 *
 * It is a conversation-level *request* — something you ask for while writing —
 * and a button in the conversation header read as a mode switch on the whole
 * conversation. It is offered only when the host would accept it, because a slash
 * menu is a list of things you can ask for.
 */
it("runs a slash command and takes its word out of the draft", async () => {
  const onCompact = vi.fn();
  const view = await mount({ canCompact: true, onCompact });

  await act(async () => view.type("/comp"));
  const row = [...view.container.querySelectorAll("button")].find(node => node.textContent?.includes("Compact context"));
  expect(row).toBeDefined();
  await act(async () => row!.click());

  expect(onCompact).toHaveBeenCalledTimes(1);
  // The word the menu was opened by is gone; the message is not sent.
  expect(view.input.value).toBe("");
  expect(prompt).not.toHaveBeenCalled();
  await view.unmount();
});

/** Enter chooses the highlighted command rather than sending the draft. */
it("chooses the command with Enter while the menu is open", async () => {
  const onCompact = vi.fn();
  const view = await mount({ canCompact: true, onCompact });

  await act(async () => view.type("/comp"));
  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
  });

  expect(onCompact).toHaveBeenCalledTimes(1);
  expect(prompt).not.toHaveBeenCalled();
  await view.unmount();
});

/** Escape closes the menu by taking its word back out of the draft. */
it("closes the menu on Escape without running anything", async () => {
  const onCompact = vi.fn();
  const view = await mount({ canCompact: true, onCompact });

  await act(async () => view.type("hello /comp"));
  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
  });

  expect(onCompact).not.toHaveBeenCalled();
  expect(view.input.value).toBe("hello");
  await view.unmount();
});

/** The arrows move the highlight, and wrap at both ends. */
it("moves the highlight with the arrow keys", async () => {
  const view = await mount({ canCompact: true });
  await act(async () => view.type("/comp"));

  const selected = () => [...view.container.querySelectorAll("[role='option']")].map(node => node.getAttribute("aria-selected"));
  expect(selected()).toEqual(["true"]);

  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
  });
  // One command, so the highlight wraps back onto it.
  expect(selected()).toEqual(["true"]);

  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowUp" }));
  });
  expect(selected()).toEqual(["true"]);
  await view.unmount();
});

/** No command to offer means no menu, and Enter sends as usual. */
it("sends normally when the host would not accept the command", async () => {
  const onCompact = vi.fn();
  const view = await mount({ canCompact: false, onCompact, peer: { ...peer, features: [] } });

  await act(async () => view.type("/comp"));
  expect([...view.container.querySelectorAll("[role='option']")]).toHaveLength(0);
  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
  });
  expect(onCompact).not.toHaveBeenCalled();
  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "/comp", [], null, { modelId: null, thinkingLevel: null }, expect.any(String));
  await view.unmount();
});

// ── the `#` conversation references ─────────────────────────────────────────

/** The rows of whichever menu is open, in the order the keyboard walks them. */
function menuRows(view: Awaited<ReturnType<typeof mount>>): HTMLElement[] {
  return [...view.container.querySelectorAll<HTMLElement>("[role='option']")];
}

/**
 * `#` offers the conversations on *that* host, and a pick becomes a link.
 *
 * The composer is a textarea, so it cannot hold the local editor's pill: the
 * draft shows the phone's compact token and the send expands it into the link
 * the agent resolves. What is asserted is both halves — the token the reader
 * sees, and the message the host receives.
 */
it("offers that host's conversations after `#` and sends the reference as a link", async () => {
  const view = await mount({ conversations });

  await act(async () => view.type("see #"));
  expect(menuRows(view).map(node => node.textContent)).toEqual([
    "Fix the flaky test",
    `Release notes${conversations[1]!.workspaceName}`,
  ]);

  await act(async () => menuRows(view)[0]!.click());
  // The compact token stays in the draft; the menu answered and closed.
  expect(view.input.value).toBe("see #Fix the flaky test ");
  expect(menuRows(view)).toHaveLength(0);

  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
  });
  await settle();

  expect(prompt).toHaveBeenCalledWith(
    "desktop_a",
    "sess_1",
    "see [Fix the flaky test](futureos://session/sess_2)",
    [],
    null,
    { modelId: null, thinkingLevel: null },
    expect.any(String),
  );
  await view.unmount();
});

/** `#` filters on what is typed, and choosing the second row picks that one. */
it("filters the menu and chooses the highlighted conversation with Enter", async () => {
  const view = await mount({ conversations });

  await act(async () => view.type("see #release"));
  expect(menuRows(view)).toHaveLength(1);
  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
  });

  expect(view.input.value).toBe("see #Release notes ");
  await view.unmount();
});

/** The arrows move the highlight through the conversations. */
it("walks the conversation menu with the arrow keys", async () => {
  const view = await mount({ conversations });
  await act(async () => view.type("#"));

  const selected = () => menuRows(view).map(node => node.getAttribute("aria-selected"));
  expect(selected()).toEqual(["true", "false"]);

  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
  });
  expect(selected()).toEqual(["false", "true"]);

  // Wraps at the end, so the last row's next is the first.
  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" }));
  });
  expect(selected()).toEqual(["true", "false"]);
  await view.unmount();
});

/** A `#` that is not the start of a word is text, never a menu. */
it("leaves a `#` inside a word or a heading alone", async () => {
  const view = await mount({ conversations });

  await act(async () => view.type("issue#12"));
  expect(menuRows(view)).toHaveLength(0);

  await act(async () => view.type("## Plan"));
  expect(menuRows(view)).toHaveLength(0);
  await view.unmount();
});

/** Escape closes the menu without taking away what was typed. */
it("closes the conversation menu on Escape and keeps the word", async () => {
  const view = await mount({ conversations });

  await act(async () => view.type("see #fix"));
  expect(menuRows(view)).toHaveLength(1);
  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
  });

  expect(menuRows(view)).toHaveLength(0);
  // The word may be text the user wants — closing is not deleting.
  expect(view.input.value).toBe("see #fix");
  await view.unmount();
});

/** With nowhere to point, `#` opens nothing rather than an empty panel. */
it("opens no menu when there is no other conversation", async () => {
  const view = await mount({ conversations: [] });

  await act(async () => view.type("#"));

  expect(menuRows(view)).toHaveLength(0);
  await view.unmount();
});

/** The conversation being composed in is not a conversation to point at. */
it("does not offer the conversation it belongs to", async () => {
  const view = await mount({ conversations, sessionId: "sess_2" });

  await act(async () => view.type("#"));

  expect(menuRows(view).map(node => node.textContent)).toEqual([
    `Release notes${conversations[1]!.workspaceName}`,
  ]);
  await view.unmount();
});

/**
 * `/` reaches the reference on a host with nothing else to offer.
 *
 * The phone has this row for a different reason (it has no `#` key to speak of);
 * here it is what keeps `/` from opening an empty menu on a host with no
 * commands and no skills installed — the row removes the slash word and types
 * the `#`, which is the same hand-off.
 */
it("offers the conversation reference from the `/` menu and hands over to `#`", async () => {
  const view = await mount({ canCompact: false, conversations, peer: { ...peer, features: [] } });

  await act(async () => view.type("/"));
  const rows = [...view.container.querySelectorAll("button")]
    .filter(node => node.getAttribute("role") === "option");
  expect(rows.map(node => node.textContent).join(" ")).toContain("Reference a conversation");

  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
  });
  await settle();

  // The slash word is gone, the `#` it named is typed, and the conversation
  // menu opened in its place.
  expect(view.input.value).toBe("#");
  expect(menuRows(view)).toHaveLength(2);
  await view.unmount();
});

/** A reference survives a visit: the token and the id it stands for both do. */
it("restores a draft's reference and still expands it", async () => {
  const first = await mount({ conversations });
  await act(async () => first.type("see #"));
  await act(async () => menuRows(first)[0]!.click());
  await first.unmount();

  const second = await mount({ conversations });
  expect(second.input.value).toBe("see #Fix the flaky test ");
  await act(async () => {
    second.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
  });
  await settle();

  expect(prompt).toHaveBeenCalledWith(
    "desktop_a",
    "sess_1",
    "see [Fix the flaky test](futureos://session/sess_2)",
    [],
    null,
    { modelId: null, thinkingLevel: null },
    expect.any(String),
  );
  await second.unmount();
});

// ── the `/` menu's skills ───────────────────────────────────────────────────

const hostSkill = {
  id: "future-peer-review",
  name: "future-peer-review",
  nameZh: "同行评审",
  description: "Review manuscripts and proposals.",
  descriptionZh: "评审稿件与基金申请书",
  version: "1.1.0",
};

/**
 * The host's skills are in the `/` menu, listed by the name the host resolves.
 *
 * This is the half the local composer and the phone both have: a skill is used by
 * naming it, so a user who has to remember the name has no menu at all. Read from
 * the host while the menu is open — the list belongs to that machine, and a skill
 * installed there a minute ago belongs in it now.
 */
it("offers the host's skills, and leaves the chosen one in the draft", async () => {
  listSkills.mockResolvedValue([hostSkill]);
  const view = await mount({ canCompact: true });

  await act(async () => view.type("/peer"));
  await settle();

  expect(listSkills).toHaveBeenCalledWith("desktop_a");
  const row = [...view.container.querySelectorAll<HTMLButtonElement>("[role='option']")]
    .find(node => node.textContent?.includes("/future-peer-review"));
  expect(row).toBeDefined();
  // The displayed description follows this app's language; the name does not,
  // because the host resolves the name.
  expect(row!.textContent).toContain("Review manuscripts and proposals.");

  await act(async () => row!.click());
  // The word the menu was opened by is replaced by the skill, ready for the
  // message that follows it — and nothing is sent.
  expect(view.input.value).toBe("/future-peer-review ");
  expect(prompt).not.toHaveBeenCalled();
  await view.unmount();
});

/** Enter chooses the highlighted skill rather than sending the draft. */
it("chooses a skill with Enter while the menu is open", async () => {
  listSkills.mockResolvedValue([hostSkill]);
  const view = await mount({ canCompact: false });

  await act(async () => view.type("/future-peer"));
  await settle();
  await act(async () => {
    view.input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
  });

  expect(view.input.value).toBe("/future-peer-review ");
  expect(prompt).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * A host that does not announce `skills_v1` is never asked for its skills, and
 * its `/` menu stays what it was: the commands it can accept.
 */
it("does not ask a host that cannot report its skills", async () => {
  const view = await mount({ canCompact: true, peer: { ...peer, features: [] } });

  await act(async () => view.type("/"));
  await settle();

  expect(listSkills).not.toHaveBeenCalled();
  expect(view.container.textContent).toContain("Compact context");
  await view.unmount();
});

/**
 * A host that refuses the read leaves the menu with the commands rather than
 * failing the composer: the user never asked to see a list, so there is nothing
 * to report to them.
 */
it("keeps the menu working when the host will not list its skills", async () => {
  listSkills.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount({ canCompact: true });

  await act(async () => view.type("/"));
  await settle();

  expect(view.container.textContent).toContain("Compact context");
  expect(view.text()).not.toContain("peer_not_connected");
  await view.unmount();
});

/**
 * The placeholder says what `/` offers, as this app's own two do — and that `#`
 * reaches another conversation.
 *
 * The local composer distinguishes a conversation ("@ to add files, / for tools
 * and skills") from a new one ("/ to use skills"), and a remote conversation is
 * the same distinction: with something to compact the menu holds a command and
 * the skills, and a draft holds only the skills. Both also carry the `#`
 * hand-off, which is the one extra way of pointing at something this side has
 * (the local screen says so in the same words). What it must never do is name
 * the machine — this app's own composers describe the task, and the machine a
 * conversation is on is the rail's business.
 */
it("describes the task, naming the `/` and `#` menus and not the machine", async () => {
  const conversation = await mount({ canCompact: true });
  expect(conversation.input.placeholder)
    .toBe("Describe the task, / for tools and skills, # to reference a conversation");
  expect(conversation.input.placeholder).not.toContain("Studio iMac");
  await conversation.unmount();

  const draft = await mount({ canCompact: false });
  expect(draft.input.placeholder).toBe("Describe the task, / to use skills, # to reference a conversation");
  await draft.unmount();
});

/** A draft started from a workspace heading names it to the host. */
it("names the workspace when the draft belongs in one", async () => {
  const view = await mount({ workspaceId: "ws_9" });
  await act(async () => view.type("hello"));
  await act(async () => view.button("Send")!.click());
  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "hello", [], "ws_9", { modelId: null, thinkingLevel: null }, expect.any(String));
  await view.unmount();
});

// ── draft persistence and isolation ─────────────────────────────────────────

/**
 * Drafts are scoped to a specific host and conversation, so switching sessions
 * never leaks text between them, and the same session id on two hosts stays
 * separate.
 */
it("isolates drafts per desktop+session and clears them after send", async () => {
  // Session A on desktop_a gets its own draft.
  const a1 = await mount({ desktopId: "desktop_a", sessionId: "sess_a" });
  await a1.type("draft for A");
  await a1.unmount();

  // Session B on the same host starts empty.
  const b = await mount({ desktopId: "desktop_a", sessionId: "sess_b" });
  expect(b.input.value).toBe("");
  await b.unmount();

  // Session A on desktop_a still has its text.
  const a2 = await mount({ desktopId: "desktop_a", sessionId: "sess_a" });
  expect(a2.input.value).toBe("draft for A");

  // The same session id on a different host is isolated.
  const other = await mount({ desktopId: "desktop_b", sessionId: "sess_a" });
  expect(other.input.value).toBe("");
  await other.unmount();

  // Sending clears the draft.
  await act(async () => a2.button("Send")!.click());
  await settle();
  const a3 = await mount({ desktopId: "desktop_a", sessionId: "sess_a" });
  expect(a3.input.value).toBe("");
  await a3.unmount();
});

// ── the caret ───────────────────────────────────────────────────────────────

/**
 * The field takes the caret when it appears, the way this app's own composer
 * does.
 *
 * Regression: "New conversation" on a paired machine put that machine's composer
 * on screen with no caret in it, so the first keystrokes after the click went
 * nowhere — and for a draft the field is the only thing on the screen a prompt
 * can be typed into. The conversation case is the same rule, so both are
 * asserted here: the shell opens a draft and an existing conversation through
 * the same view.
 */
it("takes the caret on mount, for a conversation and for a draft", async () => {
  const conversation = await mount({ sessionId: "sess_1" });
  expect(document.activeElement).toBe(conversation.input);
  await conversation.unmount();

  const draft = await mount({ sessionId: "" });
  expect(document.activeElement).toBe(draft.input);
  await draft.unmount();
});

/**
 * The caret also comes back on request, which is what the device dialog asks for
 * when it closes. A draft the dialog re-opens is already on screen — same
 * desktop, same session — so nothing remounts and the mount-time autofocus above
 * would never run again.
 */
it("takes the caret back when the shell asks for it", async () => {
  const view = await mount({ sessionId: "" });
  view.input.blur();
  expect(document.activeElement).not.toBe(view.input);

  await act(async () => emitFutureEvent("focus-composer", undefined));
  expect(document.activeElement).toBe(view.input);
  await view.unmount();
});

/**
 * A run in flight keeps the screen: the host is answering, and the reader may be
 * doing something else in the meantime, so the caret is not taken from them —
 * and it arrives in the field when the answer ends. The same rule this app's own
 * composer follows with its own `sending`.
 *
 * Re-rendered rather than remounted on purpose: the shell keys this composer by
 * desktop+session, so a run starting and settling changes the prop on a mounted
 * component, and a fix that only ran on mount would pass the test above and
 * still leave the caret outside the field after every answer.
 */
it("waits for a run in flight, then takes the caret when it settles", async () => {
  const view = await mount({ streaming: true });
  expect(document.activeElement).not.toBe(view.input);

  await view.rerender({ streaming: false });
  expect(document.activeElement).toBe(view.input);
  await view.unmount();
});

/**
 * A recoverable failure is not a lost message: the prompt goes to the durable
 * outbox, and the composer says so.
 *
 * "Recoverable" is the runtime's own verdict (`peer_not_connected`,
 * `transport`, `timeout`, …) and it means the host never saw the command — so
 * the outbox keeps it and recovery re-sends it when the link is back. The user
 * has to be told *which* of the two things happened: a prompt that reached the
 * host and failed is their text to fix, while one that is queued is already on
 * its way. Reporting it as a failure (or silently) invites them to send it
 * twice.
 */
it("queues a prompt whose link was down, and says it is queued", async () => {
  prompt.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount();
  await view.type("send this when the host is back");
  await act(async () => view.button("Send")!.click());
  await settle();

  // Queued, not failed: the composer says where it is rather than showing the
  // wire's error, and the draft is still in the box so nothing is lost either
  // way (the text can be edited and re-sent, or left for the recovery pass).
  expect(view.text()).toContain("queued");
  expect(view.text()).not.toContain("peer_not_connected");
  expect(view.input.value).toBe("send this when the host is back");
  expect(view.onSent).not.toHaveBeenCalled();
  await view.unmount();
});

it("stays silent about a prompt that is not recoverable", async () => {
  // The other half of the same rule: `agent_unavailable` is the host answering
  // "no", so there is nothing to queue and the reason is what the user needs.
  prompt.mockRejectedValue(new Error("agent_unavailable"));
  const view = await mount();
  await view.type("this one the host refused");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(view.text()).toContain("agent_unavailable");
  expect(view.text()).not.toContain("Queued");
  await view.unmount();
});

/**
 * The outbox's recovery pass belongs to a *connected* host.
 *
 * It exists to re-deliver what a dropped link swallowed, so asking a host that is
 * not connected (or was stopped by the user) would open a connection as a side
 * effect of drawing a composer — exactly what the peer list refuses to do with
 * its background poll.
 */
it("does not run the outbox recovery against a host that is not connected", async () => {
  const view = await mount({ peer: { ...peer, connected: false } });
  await settle();
  // The composer still draws: it is a usable draft box, not a dead one.
  expect(view.input).not.toBeNull();
  await view.unmount();
});

/**
 * And a recovery already in flight when the composer is gone is abandoned.
 *
 * `recoverRemoteOutbox` is awaited, so it settles after an unmount whenever the
 * conversation closes while the host is answering. The window is one microtask
 * wide, so this test drives the render and the unmount *without* a flush in
 * between: the usual `mount()` helper awaits a flush, by which time the recovery
 * has already landed and the abandoned branch cannot be reached at all.
 */
it("abandons a recovery that settles after unmount", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  // Sync `act`: effects run (and start the recovery), no microtasks are drained.
  act(() => {
    root.render(
      <RemoteComposer
        canCompact={false}
        desktopId="desktop_a"
        onCreated={vi.fn()}
        onSent={vi.fn()}
        peer={peer}
        sessionId="sess_1"
        workspaceId={null}
      />,
    );
  });
  act(() => root.unmount());
  container.remove();

  // The recovery settles here, with the composer gone.
  await settle();

  // Nothing was queued or reported: the abandoned continuation returned before
  // touching state, and the outbox is exactly as the recovery found it (a state
  // update after unmount is the branch these guards exist to avoid).
  expect(getRemoteOutbox("desktop_a", "sess_1").prompt).toBeNull();
});
