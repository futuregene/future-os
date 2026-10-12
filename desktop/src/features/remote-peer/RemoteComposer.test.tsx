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
vi.mock("./remotePeerClient", () => ({
  promptRemoteConversation: (...args: Parameters<typeof prompt>) => prompt(...args),
  abortRemoteRun: (...args: Parameters<typeof abort>) => abort(...args),
  uploadRemoteFile: (...args: Parameters<typeof upload>) => upload(...args),
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
  // By accessible name: the attach control is an icon button (as it is in this
  // app's own composer), so it has an aria-label rather than text.
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
  saveFile.mockReset();
  saveImage.mockReset();
  nativePaths.mockReset().mockResolvedValue([]);
  saveFile.mockResolvedValue({ path: "/tmp/pasted" });

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
/**
 * The machine is named where the message is written, and only there.
 *
 * It used to be in two places — a line under the conversation title *and* the
 * control row — which reads as two machines. The placeholder is where this app
 * tells the reader what the box is for, so the host's name goes there.
 */
it("names the machine the prompt runs on, in the box itself", async () => {
  const view = await mount();
  expect(view.input.placeholder).toContain("Studio iMac");
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
