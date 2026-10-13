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

const prompt = vi.fn<(
  desktopId: string,
  sessionId: string,
  message: string,
  attachments?: string[],
) => Promise<{ sessionId: string; threadId: string }>>();
const abort = vi.fn<(desktopId: string, sessionId: string) => Promise<unknown>>();
const upload = vi.fn<(input: { desktopId: string; path: string }) => Promise<{ uploadId: string; name: string; contentHash: string }>>();
const listSkills = vi.fn<(desktopId: string) => Promise<unknown[]>>();
vi.mock("./remotePeerClient", () => ({
  promptRemoteConversation: (...args: Parameters<typeof prompt>) => prompt(...args),
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

const peer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  desktopId: "desktop_a",
  error: null,
  // The capability a current host declares, which is what puts its skills in the
  // `/` menu; the tests that are about the menu without them pass their own.
  features: ["skills_v1"],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

async function mount(props: {
  canCompact?: boolean;
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
  await act(async () => {
    root.render(
      <RemoteComposer
        canCompact={props.canCompact ?? false}
        desktopId="desktop_a"
        onCompact={props.onCompact}
        onCreated={onCreated}
        onSent={onSent}
        peer={props.peer ?? peer}
        sessionId={props.sessionId ?? "sess_1"}
        streaming={props.streaming}
        workspaceId={props.workspaceId ?? null}
      />,
    );
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
  saveFile.mockReset();
  saveImage.mockReset();
  nativePaths.mockReset().mockResolvedValue([]);
  saveFile.mockResolvedValue({ path: "/tmp/pasted" });

  listSkills.mockReset().mockResolvedValue([]);
  prompt.mockReset().mockResolvedValue({ sessionId: "sess_1", threadId: "thread_1" });
  abort.mockReset().mockResolvedValue(undefined);
  upload.mockReset().mockResolvedValue({ uploadId: "upload_1", name: "notes.txt", contentHash: "abc" });
  openDialog.mockReset().mockResolvedValue("/local/notes.txt");
});

it("prompts the conversation it belongs to and clears only on success", async () => {
  const view = await mount();
  await view.type("hello host");
  await act(async () => view.button("Send")!.click());
  await settle();

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "hello host", [], null);
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

  expect(prompt).toHaveBeenCalledWith("desktop_a", "", "first message", [], null);
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

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "please read this", ["upload_7"], null);
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

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "", ["upload_7"], null);
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

  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "no file after all", [], null);
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
  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "/comp", [], null);
  await view.unmount();
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
 * The placeholder says what `/` offers, as this app's own two do.
 *
 * The local composer distinguishes a conversation ("@ to add files, / for tools
 * and skills") from a new one ("/ to use skills"), and a remote conversation is
 * the same distinction: with something to compact the menu holds a command and
 * the skills, and a draft holds only the skills. What it must never do is name
 * the machine — this app's own composers describe the task, and the machine a
 * conversation is on is the rail's business.
 */
it("describes the task, naming the `/` menu and not the machine", async () => {
  const conversation = await mount({ canCompact: true });
  expect(conversation.input.placeholder).toBe("Describe the task, / for tools and skills");
  expect(conversation.input.placeholder).not.toContain("Studio iMac");
  await conversation.unmount();

  const draft = await mount({ canCompact: false });
  expect(draft.input.placeholder).toBe("Describe the task, / to use skills");
  await draft.unmount();
});

/** A draft started from a workspace heading names it to the host. */
it("names the workspace when the draft belongs in one", async () => {
  const view = await mount({ workspaceId: "ws_9" });
  await act(async () => view.type("hello"));
  await act(async () => view.button("Send")!.click());
  expect(prompt).toHaveBeenCalledWith("desktop_a", "sess_1", "hello", [], "ws_9");
  await view.unmount();
});
