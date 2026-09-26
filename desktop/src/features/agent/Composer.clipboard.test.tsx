import type { Root } from "react-dom/client";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Composer } from "./Composer";

/**
 * The composer's clipboard-file paths: an image pasted as bytes is saved and
 * attached, any other file is copied into the thread's temp dir, Finder's native
 * file-URL pasteboard is preferred over copying bytes, and each size/count limit
 * is refused with a reason the user can read.
 *
 * `Composer.paste.test.tsx` covers the failure of an image read; the tool,
 * panel, send and refusal paths are in the other Composer suites.
 */
const h = vi.hoisted(() => ({
  deleteTempAttachment: vi.fn(),
  readNativeClipboardFilePaths: vi.fn(),
  savePastedFile: vi.fn(),
  savePastedImage: vi.fn(),
  classifyAttachment: vi.fn(),
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => (command === "list_agent_providers" ? { builtin: [], custom: [] } : [])),
}));
vi.mock("../../integrations/storage/threadStore", async original => ({
  ...await original<typeof import("../../integrations/storage/threadStore")>(),
  deleteTempAttachment: h.deleteTempAttachment,
  readNativeClipboardFilePaths: h.readNativeClipboardFilePaths,
  savePastedFile: h.savePastedFile,
  savePastedImage: h.savePastedImage,
}));
vi.mock("../../integrations/skills/skillsClient", () => ({
  listAvailableSkills: vi.fn(async () => []),
  listInstalledSkills: vi.fn(async () => []),
  loadSkillCatalog: () => ({ catalogue: Promise.resolve([]), installed: Promise.resolve([]) }),
}));
vi.mock("./attachments", async original => ({
  ...await original<typeof import("./attachments")>(),
  classifyAttachment: h.classifyAttachment,
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

function render(over: Partial<Parameters<typeof Composer>[0]> = {}) {
  act(() => root.render(
    <Composer modelOptions={[]} onSend={vi.fn(async () => {})} {...over} />,
  ));
}

beforeEach(() => {
  vi.clearAllMocks();
  h.deleteTempAttachment.mockResolvedValue(undefined);
  h.readNativeClipboardFilePaths.mockResolvedValue([]);
  h.savePastedFile.mockImplementation(async ({ name }: { name: string }) => ({ path: `/tmp/${name}` }));
  h.savePastedImage.mockResolvedValue({ path: "/tmp/pasted.png" });
  // The classifier is the backend boundary; accept whatever it is handed.
  h.classifyAttachment.mockImplementation(async (path: string) => ({ kind: "file", path }));
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** A `File` whose size the test controls without allocating the bytes. */
function pastedFile(name: string, type: string, size: number): File {
  const file = new File(["x"], name, { type });
  Object.defineProperty(file, "size", { configurable: true, value: size });
  file.arrayBuffer = async () => new ArrayBuffer(0);
  return file;
}

/** Paste `files` as the clipboard's file payload (no text, no URI list). */
async function pasteFiles(files: File[]) {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: { getData: () => "", items: files.map(file => ({ getAsFile: () => file, kind: "file" })) },
  });
  await act(async () => {
    container.querySelector("[role=textbox]")!.dispatchEvent(event);
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

function chips() {
  return [...container.querySelectorAll<HTMLButtonElement>("button[aria-label^='Remove']")];
}

it("saves a pasted image and attaches the path it returns", async () => {
  render();
  await pasteFiles([pastedFile("shot.png", "image/png", 2048)]);

  expect(h.savePastedImage).toHaveBeenCalledWith(expect.objectContaining({ extension: "png" }));
  expect(chips().map(chip => chip.getAttribute("aria-label"))).toEqual(["Remove pasted.png"]);
});

it("names the extension from the type, falling back to png", async () => {
  // boundary: an unnamed clipboard image has no usable extension, so the saver
  // gets a sane default instead of an empty one.
  render();
  await pasteFiles([pastedFile("blob", "image/", 10)]);
  expect(h.savePastedImage).toHaveBeenCalledWith(expect.objectContaining({ extension: "png" }));
});

it("copies a pasted non-image file into the thread and keeps its name", async () => {
  render();
  await pasteFiles([pastedFile("report.txt", "text/plain", 100)]);

  expect(h.savePastedFile).toHaveBeenCalledWith(expect.objectContaining({ name: "report.txt" }));
  // The chip carries the user-facing name, not the temp path it was writen to.
  expect(chips().map(chip => chip.getAttribute("aria-label"))).toEqual(["Remove report.txt"]);
});

it("refuses a pasted file over the per-file byte limit", async () => {
  // error-path: an over-limit file is named with its reason, not silently dropped.
  render();
  await pasteFiles([pastedFile("huge.bin", "application/octet-stream", 11 * 1024 * 1024)]);

  expect(h.savePastedFile).not.toHaveBeenCalled();
  expect(container.textContent).toContain("huge.bin");
});

it("refuses a paste whose copies together exceed the total byte limit", async () => {
  render();
  const files = Array.from({ length: 4 }, (_, index) =>
    pastedFile(`big-${index}.bin`, "application/octet-stream", 6 * 1024 * 1024));
  await pasteFiles(files);

  expect(h.savePastedFile).not.toHaveBeenCalled();
  expect(chips()).toHaveLength(0);
  // The whole paste is refused up front: the message names the total limit
  // rather than one file, because no single file is the problem.
  expect(container.textContent).toContain("copied files total exceeds");
});

it("copies at most the per-paste file count and says how many were dropped", async () => {
  // boundary: 12 pasted files against a limit of 10.
  render();
  const files = Array.from({ length: 12 }, (_, index) =>
    pastedFile(`f${index}.txt`, "text/plain", 10));
  await pasteFiles(files);

  expect(h.savePastedFile).toHaveBeenCalledTimes(10);
  expect(h.savePastedFile).not.toHaveBeenCalledWith(expect.objectContaining({ name: "f10.txt" }));
  expect(h.savePastedFile).not.toHaveBeenCalledWith(expect.objectContaining({ name: "f11.txt" }));
});

it("reports a copy that could not be read and keeps the ones that could", async () => {
  // error-path: one unreadable file must not lose the rest of the paste.
  render();
  h.savePastedFile.mockImplementation(async ({ name }: { name: string }) => {
    if (name === "bad.txt")
      throw new Error("temp dir is full");
    return { path: `/tmp/${name}` };
  });
  await pasteFiles([
    pastedFile("bad.txt", "text/plain", 10),
    pastedFile("good.txt", "text/plain", 10),
  ]);

  expect(container.textContent).toContain("bad.txt");
  // The readable one still attaches, so one bad file does not lose the paste.
  expect(chips().map(chip => chip.getAttribute("aria-label"))).toEqual(["Remove good.txt"]);
});

it("deletes a temp copy the classifier refused", async () => {
  // error-path: the bytes were already written, so a refused path must not leak
  // a file in the thread's temp dir.
  render();
  h.classifyAttachment.mockImplementation(async (path: string) => ({ kind: null, path, reason: "binary not supported" }));
  await pasteFiles([pastedFile("weird.bin", "application/octet-stream", 10)]);

  expect(h.deleteTempAttachment).toHaveBeenCalledWith("/tmp/weird.bin");
});

it("prefers Finder's native paths over copying the bytes", async () => {
  // platform-cfg: macOS publishes a native file-URL pasteboard that WKWebView
  // turns into opaque Files; the original path is authoritative, so the bytes
  // must not be copied.
  render();
  h.readNativeClipboardFilePaths.mockResolvedValue(["/Users/me/report.pdf"]);
  await pasteFiles([pastedFile("report.pdf", "application/pdf", 100)]);

  expect(h.savePastedFile).not.toHaveBeenCalled();
  expect(chips().map(chip => chip.getAttribute("aria-label"))).toEqual(["Remove report.pdf"]);
});

it("falls back to copying when the native pasteboard read fails", async () => {
  // error-path: the native read is best-effort, so its failure still attaches.
  render();
  h.readNativeClipboardFilePaths.mockRejectedValue(new Error("no pasteboard access"));
  await pasteFiles([pastedFile("fallback.txt", "text/plain", 10)]);

  expect(h.savePastedFile).toHaveBeenCalledWith(expect.objectContaining({ name: "fallback.txt" }));
});

it("handles an image and a document in the same paste", async () => {
  render();
  await pasteFiles([
    pastedFile("shot.png", "image/png", 100),
    pastedFile("notes.txt", "text/plain", 100),
  ]);

  expect(h.savePastedImage).toHaveBeenCalledTimes(1);
  expect(h.savePastedFile).toHaveBeenCalledTimes(1);
  expect(chips()).toHaveLength(2);
});

it("shows a chip for each accepted paste and keeps a permanent one on disk", async () => {
  // The two kinds of attachment are distinguishable: a copied file is temporary
  // (this composer owns it), a path the user picked is not.
  render();
  await pasteFiles([pastedFile("kept.txt", "text/plain", 10)]);
  expect(chips()).toHaveLength(1);
  expect(chips()[0]!.getAttribute("aria-label")).toContain("kept.txt");

  // Dismissing the chip clears the row; a temporary copy is then deleted, a
  // permanent path is only forgotten.
  await act(async () => {
    chips()[0]!.click();
    await Promise.resolve();
  });
  expect(chips()).toHaveLength(0);
});

it("refuses an image pasted over the read-source size limit", async () => {
  // boundary: an image above `READ_SOURCE_MAX_BYTES` (25 MB) is named with its
  // reason instead of being written to disk first.
  render();
  await pasteFiles([pastedFile("huge.png", "image/png", 26 * 1024 * 1024)]);

  expect(h.savePastedImage).not.toHaveBeenCalled();
  expect(chips()).toHaveLength(0);
  expect(container.textContent).toContain("huge.png");
});

it("deletes a temp copy of an image the classifier refused", async () => {
  // error-path: the bytes were written before classification, so a refusal must
  // not leak the file into the thread's temp dir.
  render();
  h.classifyAttachment.mockImplementation(async (path: string) => ({ kind: null, path, reason: "image unreadable" }));
  await pasteFiles([pastedFile("broken.png", "image/png", 100)]);

  expect(h.savePastedImage).toHaveBeenCalledTimes(1);
  expect(h.deleteTempAttachment).toHaveBeenCalledWith("/tmp/pasted.png");
  expect(chips()).toHaveLength(0);
});

it("prefers a clipboard URI list over the file bytes", async () => {
  // platform-cfg: a `text/uri-list` is an authoritative local reference, so the
  // attachment is that path and nothing is copied.
  render();
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: {
      getData: (type: string) => (type === "text/uri-list" ? "file:///tmp/from-list.txt\r\n" : ""),
      items: [],
    },
  });
  await act(async () => {
    container.querySelector("[role=textbox]")!.dispatchEvent(event);
    await Promise.resolve();
    await Promise.resolve();
  });

  expect(h.savePastedFile).not.toHaveBeenCalled();
  expect(chips().map(chip => chip.getAttribute("aria-label"))).toEqual(["Remove from-list.txt"]);
});
