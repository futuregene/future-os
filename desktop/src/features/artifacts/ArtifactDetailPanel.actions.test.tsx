// @vitest-environment jsdom
import type { Mock } from "vitest";
import type { StoredArtifact } from "../../integrations/storage/types";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ArtifactDetailPanel } from "./ArtifactDetailPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const convertFileSrc = vi.fn<(path: string) => string>();
const save = vi.fn<(options: { defaultPath?: string; title?: string }) => Promise<string | null>>();
const deleteArtifact = vi.fn<(id: string) => Promise<unknown>>();
const exportArtifactFile = vi.fn<(input: { content?: string | null; destinationPath: string; sourcePath?: string | null }) => Promise<void>>();
const inspectAttachment = vi.fn<(path: string) => Promise<{ isDir: boolean; isBinary: boolean; size: number }>>();
const openPath = vi.fn<(path: string) => Promise<void>>();
const readTextFilePreview = vi.fn<(input: { maxBytes?: number | null; path: string }) => Promise<{ content: string; size: number; truncated: boolean; validUtf8: boolean }>>();

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => convertFileSrc(path),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (options: { defaultPath?: string; title?: string }) => save(options),
}));

vi.mock("../../integrations/storage/threadStore", () => ({
  deleteArtifact: (id: string) => deleteArtifact(id),
  exportArtifactFile: (input: { content?: string | null; destinationPath: string; sourcePath?: string | null }) => exportArtifactFile(input),
  inspectAttachment: (path: string) => inspectAttachment(path),
  openPath: (path: string) => openPath(path),
  readTextFilePreview: (input: { maxBytes?: number | null; path: string }) => readTextFilePreview(input),
  storedTimeToIso: (value: number) => new Date(value).toISOString(),
}));

// The pdfjs-backed preview is exercised by its own suite; here only the lazy
// boundary and the branch that mounts it matter.
vi.mock("./PdfPreview", () => ({
  PdfPreview: (props: { path: string }) => createElement("div", { "data-path": props.path, "data-testid": "pdf-preview" }, "pdf"),
}));

vi.mock("../filepreview/FilePreviewOverlay", () => ({
  FilePreviewOverlay: (props: { kind: string; onClose?: () => void; onOpenExternal?: () => void; open: boolean; path: string }) =>
    createElement("div", {
      "data-kind": props.kind,
      "data-open": String(props.open),
      "data-path": props.path,
      "data-testid": "overlay",
    }, [
      createElement("button", { "data-testid": "overlay-close", "key": "close", "onClick": () => props.onClose?.(), "type": "button" }, "overlay-close"),
      createElement("button", { "data-testid": "overlay-external", "key": "external", "onClick": () => props.onOpenExternal?.(), "type": "button" }, "overlay-external"),
    ]),
}));

// Deterministic highlighting: the artifact's own branching is what is under test.
vi.mock("../markdown/useCodeHighlighter", async importOriginal => ({
  ...await importOriginal<typeof import("../markdown/useCodeHighlighter")>(),
  useCodeHighlighter: () => ({ highlight: () => null, isLoaded: false }),
}));

function artifact(overrides: Partial<StoredArtifact> = {}): StoredArtifact {
  return {
    id: "a1",
    workspaceId: "w1",
    title: "artifact",
    artifactType: "text",
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, reject, resolve };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let onBack: Mock<() => void>;
let onChanged: Mock<() => void>;

async function mount(panel: StoredArtifact, extra: { flush?: boolean } = {}) {
  await act(async () => {
    root.render(createElement(ArtifactDetailPanel, { artifact: panel, onBack, onChanged }));
  });
  if (extra.flush !== false)
    await flush();
}

async function rerender(panel: StoredArtifact) {
  await act(async () => {
    root.render(createElement(ArtifactDetailPanel, { artifact: panel, onBack, onChanged }));
  });
  await flush();
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function buttonByText(text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.textContent ?? "").trim() === text);
}

function buttonByLabel(label: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.getAttribute("aria-label") ?? "") === label);
}

function overlayState(): { kind: string | null; open: string | null } {
  const overlay = container.querySelector("[data-testid='overlay']");
  return { kind: overlay?.getAttribute("data-kind") ?? null, open: overlay?.getAttribute("data-open") ?? null };
}

beforeEach(() => {
  convertFileSrc.mockReset();
  save.mockReset();
  deleteArtifact.mockReset();
  exportArtifactFile.mockReset();
  inspectAttachment.mockReset();
  openPath.mockReset();
  readTextFilePreview.mockReset();

  convertFileSrc.mockImplementation(path => `asset:${path}`);
  save.mockResolvedValue("/out/destination.txt");
  deleteArtifact.mockResolvedValue(undefined);
  exportArtifactFile.mockResolvedValue(undefined);
  inspectAttachment.mockResolvedValue({ isBinary: false, isDir: false, size: 2048 });
  openPath.mockResolvedValue(undefined);
  readTextFilePreview.mockResolvedValue({ content: "# Title\n\nbody", size: 1024, truncated: false, validUtf8: true });

  // jsdom has no execCommand; copyText uses it first.
  (document as unknown as { execCommand: () => boolean }).execCommand = () => true;

  onBack = vi.fn<() => void>();
  onChanged = vi.fn<() => void>();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("artifactDetailPanel file state", () => {
  it("says the file was deleted or moved when the stat fails, and keeps Open/Export", async () => {
    inspectAttachment.mockRejectedValue(new Error("ENOENT: no such file"));
    await mount(artifact({ artifactType: "document", path: "/ws/gone.md" }));

    expect(container.textContent).toContain("File deleted or moved");
    expect(container.textContent).toContain("The original file is no longer at its location, so it can't be previewed. This isn't a system error — the file was likely deleted or moved.");
    // No preview of any kind is shown for a missing file…
    expect(container.querySelector("[data-testid='pdf-preview']")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    // …but the file can still be opened or exported.
    expect(buttonByText("Open")).toBeTruthy();
    expect(buttonByText("Export")).toBeTruthy();
  });

  it("does not stat or preview anything for a pathless artifact", async () => {
    await mount(artifact({ artifactType: "text", content: "inline body", title: "note" }));
    expect(inspectAttachment).not.toHaveBeenCalled();
    expect(readTextFilePreview).not.toHaveBeenCalled();
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toContain("inline body");
    expect(buttonByText("Open")).toBeUndefined();
  });
});

describe("artifactDetailPanel image preview", () => {
  it("renders the image through the asset protocol and enlarges it", async () => {
    await mount(artifact({ artifactType: "image", path: "/ws/pic.png", title: "pic" }));
    expect(convertFileSrc).toHaveBeenCalledWith("/ws/pic.png");
    const img = container.querySelector("img");
    expect(img?.getAttribute("src")).toBe("asset:/ws/pic.png");
    expect(img?.getAttribute("alt")).toBe("pic");

    expect(overlayState()).toEqual({ kind: "image", open: "false" });
    await act(async () => {
      buttonByLabel("Enlarge preview")?.click();
    });
    expect(overlayState()).toEqual({ kind: "image", open: "true" });

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid='overlay-close']")?.click();
    });
    expect(overlayState().open).toBe("false");
  });

  it("treats an extensionless artifact as an image when the type says so", async () => {
    await mount(artifact({ artifactType: "image", path: "/ws/snapshot", title: "snapshot" }));
    expect(convertFileSrc).toHaveBeenCalledWith("/ws/snapshot");
    expect(container.querySelector("img")).not.toBeNull();
  });

  it("falls back to a stated failure when the image cannot be decoded", async () => {
    await mount(artifact({ artifactType: "image", path: "/ws/pic.svg", title: "pic" }));
    const img = container.querySelector("img")!;
    await act(async () => {
      img.dispatchEvent(new Event("error"));
    });
    expect(container.textContent).toContain("Image preview unavailable");
    expect(container.textContent).toContain("The image could not be loaded. You can still open or export the original file.");
    expect(container.querySelector("img")).toBeNull();
  });

  it("clears a previous image failure when a different file is previewed", async () => {
    await mount(artifact({ id: "a1", artifactType: "image", path: "/ws/one.png", title: "one" }));
    await act(async () => {
      container.querySelector("img")!.dispatchEvent(new Event("error"));
    });
    expect(container.textContent).toContain("Image preview unavailable");

    await rerender(artifact({ id: "a2", artifactType: "image", path: "/ws/two.png", title: "two" }));
    expect(container.textContent).not.toContain("Image preview unavailable");
    expect(container.querySelector("img")?.getAttribute("src")).toBe("asset:/ws/two.png");
  });
});

describe("artifactDetailPanel text preview", () => {
  it("reads a markdown file, renders it, and reports the size", async () => {
    readTextFilePreview.mockResolvedValue({
      content: "# Title\n\nbody text",
      size: 1234567,
      truncated: false,
      validUtf8: true,
    });
    await mount(artifact({ artifactType: "document", path: "/ws/docs/note.md", title: "note" }));

    expect(readTextFilePreview).toHaveBeenCalledWith({ maxBytes: 200 * 1024, path: "/ws/docs/note.md" });
    expect(container.textContent).toContain("Title");
    expect(container.textContent).toContain("body text");
    expect(container.textContent).toContain("1,234,567 bytes");
    expect(container.textContent).not.toContain("preview truncated");
    // Markdown can be enlarged like an image.
    expect(overlayState()).toEqual({ kind: "markdown", open: "false" });
    await act(async () => {
      buttonByLabel("Enlarge preview")?.click();
    });
    expect(overlayState()).toEqual({ kind: "markdown", open: "true" });
  });

  it("flags a truncated preview", async () => {
    readTextFilePreview.mockResolvedValue({ content: "partial", size: 9_000_000, truncated: true, validUtf8: true });
    await mount(artifact({ artifactType: "document", path: "/ws/big.md", title: "big" }));
    expect(container.textContent).toContain("9,000,000 bytes · preview truncated");
  });

  it("renders a code file as monospace text and copies it", async () => {
    readTextFilePreview.mockResolvedValue({
      content: "fn main() {}\n",
      size: 12,
      truncated: false,
      validUtf8: true,
    });
    await mount(artifact({ artifactType: "code", path: "/ws/main.rs", title: "main.rs" }));
    const pre = container.querySelector("pre");
    expect(pre?.textContent).toBe("fn main() {}\n");
    // A code file still gets the fullscreen text overlay (kind "text").
    expect(overlayState()).toEqual({ kind: "text", open: "false" });

    const copy = buttonByLabel("Copy preview")!;
    await act(async () => {
      copy.click();
    });
    expect(buttonByLabel("Copy preview")).toBeTruthy();
  });

  it("surfaces a text-preview read failure", async () => {
    readTextFilePreview.mockRejectedValue(new Error("READ_FAILED: not utf8"));
    await mount(artifact({ artifactType: "document", path: "/ws/bad.md", title: "bad" }));
    expect(container.textContent).toContain("READ_FAILED: not utf8");
  });

  it("offers no preview for a file type it cannot render", async () => {
    await mount(artifact({ artifactType: "archive", path: "/ws/archive.bin", title: "archive" }));
    expect(readTextFilePreview).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Preview unavailable");
    expect(container.textContent).toContain("This file type does not have an inline preview yet. Use Open or Export to inspect the original file.");
    expect(overlayState()).toEqual({ kind: null, open: null });
  });

  it("still shows stored content when the file type has no preview of its own", async () => {
    await mount(artifact({ artifactType: "archive", path: "/ws/archive.zip", content: "stored bytes", title: "archive" }));
    expect(container.textContent).toContain("stored bytes");
    expect(container.textContent).not.toContain("Preview unavailable");
  });

  it("mounts the PDF preview behind the lazy boundary", async () => {
    await mount(artifact({ artifactType: "pdf", path: "/ws/spec.pdf", title: "spec" }));
    const pdf = container.querySelector("[data-testid='pdf-preview']");
    expect(pdf?.getAttribute("data-path")).toBe("/ws/spec.pdf");
  });
});

describe("artifactDetailPanel actions", () => {
  it("opens the file without reporting a data change, and shows the busy label", async () => {
    const pending = deferred<void>();
    openPath.mockReturnValue(pending.promise);
    await mount(artifact({ artifactType: "document", path: "/ws/note.md", title: "note" }));

    await act(async () => {
      buttonByText("Open")?.click();
    });
    expect(openPath).toHaveBeenCalledWith("/ws/note.md");
    expect(buttonByText("Opening")).toBeTruthy();
    expect(buttonByText("Export")?.disabled).toBe(true);
    expect(onChanged).not.toHaveBeenCalled();

    await act(async () => {
      pending.resolve();
    });
    await flush();
    expect(buttonByText("Open")).toBeTruthy();
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("reports an open failure", async () => {
    openPath.mockRejectedValue(new Error("no handler for .md"));
    await mount(artifact({ artifactType: "document", path: "/ws/note.md", title: "note" }));
    await act(async () => {
      buttonByText("Open")?.click();
    });
    await flush();
    expect(container.textContent).toContain("no handler for .md");
    expect(buttonByText("Open")?.disabled).toBe(false);
  });

  it("clears a previous action error on the next action", async () => {
    openPath.mockRejectedValueOnce(new Error("first failure"));
    await mount(artifact({ artifactType: "document", path: "/ws/note.md", title: "note" }));
    await act(async () => {
      buttonByText("Open")?.click();
    });
    await flush();
    expect(container.textContent).toContain("first failure");

    await act(async () => {
      buttonByText("Open")?.click();
    });
    await flush();
    expect(container.textContent).not.toContain("first failure");
  });

  it("exports a file-backed artifact by source path", async () => {
    await mount(artifact({ artifactType: "document", path: "/ws/docs/note.md", title: "note" }));
    await act(async () => {
      buttonByText("Export")?.click();
    });
    await flush();

    expect(save).toHaveBeenCalledWith({ defaultPath: "note.md", title: "Export artifact" });
    expect(exportArtifactFile).toHaveBeenCalledWith({
      content: null,
      destinationPath: "/out/destination.txt",
      sourcePath: "/ws/docs/note.md",
    });
    expect(onChanged).toHaveBeenCalledTimes(1);
  });

  it("exports inline content with a sanitized fallback file name", async () => {
    await mount(artifact({ artifactType: "data", content: "{\"a\":1}", title: "my/report: v2" }));
    await act(async () => {
      buttonByText("Export")?.click();
    });
    await flush();

    expect(save).toHaveBeenCalledWith({ defaultPath: "my_report_ v2.json", title: "Export artifact" });
    expect(exportArtifactFile).toHaveBeenCalledWith({
      content: "{\"a\":1}",
      destinationPath: "/out/destination.txt",
      sourcePath: null,
    });
  });

  it("falls back to the artifact title when the path has no basename", async () => {
    readTextFilePreview.mockResolvedValue({ content: "x", size: 1, truncated: false, validUtf8: true });
    await mount(artifact({ artifactType: "text", path: "/", title: "roaming" }));
    await act(async () => {
      buttonByText("Export")?.click();
    });
    await flush();
    expect(save).toHaveBeenCalledWith({ defaultPath: "roaming", title: "Export artifact" });
  });

  it("does nothing when the export dialog is cancelled", async () => {
    save.mockResolvedValue(null);
    await mount(artifact({ artifactType: "text", content: "inline", title: "note" }));
    await act(async () => {
      buttonByText("Export")?.click();
    });
    await flush();
    expect(exportArtifactFile).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain("Failed");
  });

  it("reports an export failure without reporting a change", async () => {
    exportArtifactFile.mockRejectedValue(new Error("EXPORT_FAILED: disk full"));
    await mount(artifact({ artifactType: "text", content: "inline", title: "note" }));
    await act(async () => {
      buttonByText("Export")?.click();
    });
    await flush();
    expect(container.textContent).toContain("EXPORT_FAILED: disk full");
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("reports a failing export dialog", async () => {
    save.mockRejectedValue(new Error("save dialog unavailable"));
    await mount(artifact({ artifactType: "text", content: "inline", title: "note" }));
    await act(async () => {
      buttonByText("Export")?.click();
    });
    await flush();
    expect(container.textContent).toContain("save dialog unavailable");
  });

  it("deletes the artifact, reports the change and goes back", async () => {
    const pending = deferred<unknown>();
    deleteArtifact.mockReturnValue(pending.promise);
    await mount(artifact({ id: "a5", artifactType: "text", content: "inline", title: "note" }));

    await act(async () => {
      buttonByText("Delete")?.click();
    });
    expect(deleteArtifact).toHaveBeenCalledWith("a5");
    expect(buttonByText("Deleting")).toBeTruthy();

    await act(async () => {
      pending.resolve(undefined);
    });
    await flush();
    expect(onChanged).toHaveBeenCalledTimes(1);
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("reports a delete failure and stays on the detail", async () => {
    deleteArtifact.mockRejectedValue(new Error("artifact not found"));
    await mount(artifact({ id: "a5", artifactType: "text", content: "inline", title: "note" }));
    await act(async () => {
      buttonByText("Delete")?.click();
    });
    await flush();
    expect(container.textContent).toContain("artifact not found");
    expect(onBack).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
    expect(buttonByText("Delete")?.disabled).toBe(false);
  });

  it("matches the Export affordance to what there is to export", async () => {
    // Nothing at all: no path, no stored content.
    await mount(artifact({ artifactType: "text", title: "empty" }));
    expect(buttonByText("Export")?.disabled).toBe(true);

    // A file-backed artifact can always be exported by source path, even with
    // no inline preview for its type.
    act(() => root.unmount());
    root = createRoot(container);
    await mount(artifact({ artifactType: "archive", path: "/ws/archive.bin", title: "archive" }));
    expect(container.textContent).toContain("Preview unavailable");
    expect(buttonByText("Export")?.disabled).toBe(false);

    // Stored content alone is enough too.
    act(() => root.unmount());
    root = createRoot(container);
    await mount(artifact({ artifactType: "archive", content: "stored", title: "archive" }));
    expect(buttonByText("Export")?.disabled).toBe(false);
  });

  it("copies the path and the inline content", async () => {
    const copyText = vi.spyOn(document, "execCommand");
    await mount(artifact({ artifactType: "text", content: "inline body", path: "/ws/note.txt", title: "note" }));

    // A text path shows the file preview, so the inline copy button is the
    // preview one; the path row always has its own.
    const pathCopy = buttonByLabel("Copy path")!;
    await act(async () => {
      pathCopy.click();
    });
    await flush();
    expect(copyText).toHaveBeenCalledWith("copy");

    // Force the inline-content branch (no file preview) and copy that.
    act(() => root.unmount());
    root = createRoot(container);
    await mount(artifact({ artifactType: "text", content: "inline body", title: "note" }));
    const contentCopy = buttonByLabel("Copy content")!;
    await act(async () => {
      contentCopy.click();
    });
    await flush();
    expect(container.querySelector("svg.lucide-check")).toBeTruthy();
    copyText.mockRestore();
  });

  it("hands back to the list", async () => {
    await mount(artifact({ artifactType: "text", content: "x", title: "note" }));
    await act(async () => {
      buttonByText("Artifacts")?.click();
    });
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});

describe("artifactDetailPanel overlay wiring", () => {
  it("opens the file externally from the overlay", async () => {
    await mount(artifact({ artifactType: "image", path: "/ws/pic.png", title: "pic" }));
    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid='overlay-external']")?.click();
    });
    await flush();
    expect(openPath).toHaveBeenCalledWith("/ws/pic.png");
  });

  it("closes the enlarged preview from the overlay", async () => {
    await mount(artifact({ artifactType: "image", path: "/ws/pic.png", title: "pic" }));
    await act(async () => {
      buttonByLabel("Enlarge preview")?.click();
    });
    expect(overlayState().open).toBe("true");

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid='overlay-close']")?.click();
    });
    expect(overlayState().open).toBe("false");
  });
});
