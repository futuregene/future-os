// @vitest-environment jsdom
import type { Mock } from "vitest";
import type { StoredArtifact } from "../../integrations/storage/types";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ArtifactsPanel } from "./ArtifactsPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const openDialog = vi.fn<(options: { multiple?: boolean; title?: string }) => Promise<string | string[] | null>>();
const deleteArtifact = vi.fn<(id: string) => Promise<void>>();
const importAttachmentArtifact = vi.fn<(input: { threadId: string; path: string }) => Promise<unknown>>();
const inspectAttachment = vi.fn<(path: string) => Promise<{ isDir: boolean; size: number; isBinary: boolean }>>();
const openPath = vi.fn<(path: string) => Promise<void>>();

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (options: { multiple?: boolean; title?: string }) => openDialog(options),
  save: async () => null,
}));

vi.mock("../../integrations/storage/threadStore", () => ({
  deleteArtifact: (id: string) => deleteArtifact(id),
  importAttachmentArtifact: (input: { threadId: string; path: string }) => importAttachmentArtifact(input),
  inspectAttachment: (path: string) => inspectAttachment(path),
  openPath: (path: string) => openPath(path),
  storedTimeToIso: (value: number) => new Date(value).toISOString(),
}));

// The overlay belongs to the filepreview feature; here only the wiring matters.
vi.mock("../filepreview/FilePreviewOverlay", () => ({
  FilePreviewOverlay: (props: { kind: string; onClose?: () => void; onOpenExternal?: () => void; open: boolean; path: string }) =>
    createElement("div", {
      "data-kind": props.kind,
      "data-open": String(props.open),
      "data-path": props.path,
      "data-testid": "preview-overlay",
    }, [
      createElement("button", { "data-testid": "overlay-close", "key": "close", "onClick": () => props.onClose?.(), "type": "button" }, "close"),
      createElement("button", { "data-testid": "overlay-external", "key": "external", "onClick": () => props.onOpenExternal?.(), "type": "button" }, "external"),
    ]),
}));

function artifact(overrides: Partial<StoredArtifact> = {}): StoredArtifact {
  return {
    id: "a1",
    workspaceId: "w1",
    title: "report.md",
    artifactType: "document",
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let onChanged: Mock<() => void>;
let onSelectArtifact: Mock<(artifactId: string) => void>;

interface Props {
  artifacts?: StoredArtifact[];
  threadId?: string;
  workspacePath?: string | null;
}

async function mount(overrides: Props = {}) {
  const props = {
    artifacts: [],
    threadId: "thread-1",
    workspacePath: "/ws",
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(ArtifactsPanel, { ...props, onChanged, onSelectArtifact }));
  });
  return props;
}

function buttonByText(text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.textContent ?? "").trim() === text);
}

function buttonByLabelPrefix(prefix: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.getAttribute("aria-label") ?? "").startsWith(prefix));
}

async function openCardMenu(index = 0) {
  const cards = [...container.querySelectorAll("div.group.relative")];
  await act(async () => {
    cards[index]?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 40, clientY: 60 }));
  });
}

/**
 * Each card owns its own context menu, and opening one does not close the
 * others — so menu items are looked up inside the menu attached to a card
 * index, never across the whole container.
 */
function menuItemIn(cardIndex: number, label: string): HTMLButtonElement | undefined {
  const panel = container.querySelectorAll<HTMLElement>("div.z-50")[cardIndex];
  return [...(panel?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
    .find(button => (button.textContent ?? "").trim() === label);
}

function menuItem(label: string): HTMLButtonElement | undefined {
  const panels = container.querySelectorAll<HTMLElement>("div.z-50");
  const panel = panels[panels.length - 1];
  return [...(panel?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
    .find(button => (button.textContent ?? "").trim() === label);
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
  });
}

beforeEach(() => {
  openDialog.mockReset();
  deleteArtifact.mockReset();
  importAttachmentArtifact.mockReset();
  inspectAttachment.mockReset();
  openPath.mockReset();
  openDialog.mockResolvedValue(null);
  deleteArtifact.mockResolvedValue(undefined);
  importAttachmentArtifact.mockResolvedValue(undefined);
  inspectAttachment.mockResolvedValue({ isDir: false, size: 1024, isBinary: false });
  openPath.mockResolvedValue(undefined);
  onChanged = vi.fn<() => void>();
  onSelectArtifact = vi.fn<(artifactId: string) => void>();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("artifactsPanel list", () => {
  it("explains an empty panel and offers only the upload action", async () => {
    await mount();
    expect(container.textContent).toContain("No artifacts yet");
    expect(container.textContent).toContain("Generated reports, summaries, tables, and files will appear here.");
    expect(container.querySelector("input")).toBeNull();
    expect(buttonByText("Upload")).toBeTruthy();
  });

  it("shows a filter box once there is something to filter", async () => {
    await mount({ artifacts: [artifact()] });
    const input = container.querySelector("input");
    expect(input?.getAttribute("placeholder")).toBe("Filter artifacts…");
    expect(container.textContent).toContain("report.md");
  });

  it("filters by title case-insensitively and reports no match", async () => {
    await mount({
      artifacts: [
        artifact({ id: "a1", title: "Report Alpha" }),
        artifact({ id: "a2", title: "report beta" }),
      ],
    });
    await act(async () => {
      setInputValue("ALPHA");
    });
    expect(container.textContent).toContain("Report Alpha");
    expect(container.textContent).not.toContain("report beta");

    // Leading/trailing whitespace is not part of the query.
    await act(async () => {
      setInputValue("  beta  ");
    });
    expect(container.textContent).toContain("report beta");
    expect(container.textContent).not.toContain("Report Alpha");

    await act(async () => {
      setInputValue("nothing-matches");
    });
    expect(container.textContent).toContain("No matching artifacts.");

    await act(async () => {
      setInputValue("");
    });
    expect(container.textContent).toContain("Report Alpha");
    expect(container.textContent).toContain("report beta");
  });

  it("filters CJK titles and keeps a large list bounded", async () => {
    const artifacts = Array.from({ length: 120 }, (_, index) => artifact({ id: `a${index}`, title: `报告-${index}` }));
    artifacts.push(artifact({ id: "a-cjk", title: "工作区/模块/文件-テキスト.md" }));
    await mount({ artifacts });

    await act(async () => {
      setInputValue("テキスト");
    });
    expect(container.textContent).toContain("工作区/模块/文件-テキスト.md");
    expect(container.querySelectorAll("div.group.relative")).toHaveLength(1);

    await act(async () => {
      setInputValue("报告-2");
    });
    // 报告-2, 报告-20…报告-29 and 报告-2xx all match the substring.
    expect(container.querySelectorAll("div.group.relative").length).toBeGreaterThan(1);
    // A DOM-heavy boundary case: give it room to finish on a loaded machine.
  }, 20_000);

  function setInputValue(value: string) {
    const input = container.querySelector("input");
    if (!input)
      throw new Error("filter input missing");
    // React tracks the DOM value; set it through the native setter so onChange fires.
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }
});

describe("artifactsPanel upload", () => {
  it("imports the chosen file and reports the change", async () => {
    openDialog.mockResolvedValue("/ws/new.png");
    inspectAttachment.mockResolvedValue({ isDir: false, isBinary: true, size: 4096 });
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();

    expect(openDialog).toHaveBeenCalledWith({ multiple: false, title: "Upload file" });
    expect(inspectAttachment).toHaveBeenCalledWith("/ws/new.png");
    expect(importAttachmentArtifact).toHaveBeenCalledWith({ path: "/ws/new.png", threadId: "thread-1" });
    expect(onChanged).toHaveBeenCalledTimes(1);
    expect(buttonByText("Upload")?.disabled).toBe(false);
  });

  it("takes the first entry when the dialog answers with a list", async () => {
    openDialog.mockResolvedValue(["/ws/one.txt", "/ws/two.txt"]);
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(importAttachmentArtifact).toHaveBeenCalledWith({ path: "/ws/one.txt", threadId: "thread-1" });
  });

  it("does nothing when the dialog is cancelled", async () => {
    openDialog.mockResolvedValue(null);
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(inspectAttachment).not.toHaveBeenCalled();
    expect(importAttachmentArtifact).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain("Upload failed");
  });

  it("treats an empty selection list as a cancel", async () => {
    openDialog.mockResolvedValue([]);
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(importAttachmentArtifact).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain("Upload failed");
  });

  it("refuses a directory", async () => {
    openDialog.mockResolvedValue("/ws/folder");
    inspectAttachment.mockResolvedValue({ isDir: true, isBinary: false, size: 0 });
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(container.textContent).toContain("Folders are not supported.");
    expect(importAttachmentArtifact).not.toHaveBeenCalled();
  });

  it("refuses a file above the read bound and states the bound", async () => {
    openDialog.mockResolvedValue("/ws/huge.bin");
    inspectAttachment.mockResolvedValue({ isDir: false, isBinary: true, size: 26 * 1024 * 1024 });
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(container.textContent).toContain("File is too large. Maximum 25.0 MiB.");
    expect(importAttachmentArtifact).not.toHaveBeenCalled();
  });

  it("accepts a file exactly at the read bound", async () => {
    openDialog.mockResolvedValue("/ws/exact.bin");
    inspectAttachment.mockResolvedValue({ isDir: false, isBinary: true, size: 25 * 1024 * 1024 });
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(importAttachmentArtifact).toHaveBeenCalledTimes(1);
    expect(container.textContent).not.toContain("too large");
  });

  it("reports a failing import and re-enables the button", async () => {
    openDialog.mockResolvedValue("/ws/new.png");
    importAttachmentArtifact.mockRejectedValue(new Error("attachment store full"));
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(container.textContent).toContain("Upload failed: attachment store full");
    expect(buttonByText("Upload")?.disabled).toBe(false);
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("reports a failing dialog before anything is imported", async () => {
    openDialog.mockRejectedValue(new Error("dialog unavailable"));
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(container.textContent).toContain("Upload failed: dialog unavailable");
  });

  it("clears a previous upload error on the next attempt", async () => {
    openDialog.mockRejectedValue(new Error("dialog unavailable"));
    await mount();
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(container.textContent).toContain("Upload failed: dialog unavailable");

    openDialog.mockResolvedValue("/ws/ok.txt");
    await act(async () => {
      buttonByText("Upload")?.click();
    });
    await flush();
    expect(container.textContent).not.toContain("Upload failed");
    expect(importAttachmentArtifact).toHaveBeenCalledTimes(1);
  });

  it("shows the in-flight state and ignores a second click that slips through", async () => {
    // A double-click can deliver a second event before the disabled attribute
    // lands, so the handler carries its own re-entrancy guard.
    let resolveDialog!: (value: string | null) => void;
    openDialog.mockImplementation(() => new Promise((resolve) => {
      resolveDialog = resolve;
    }));
    await mount();
    const upload = buttonByText("Upload")!;

    await act(async () => {
      upload.click();
    });
    expect(buttonByText("Uploading…")?.disabled).toBe(true);
    expect(buttonByText("Uploading…")).toBeTruthy();

    await act(async () => {
      // dispatchEvent bypasses jsdom's `click()` disabled short-circuit, which
      // is exactly the "event already in flight" case the guard covers.
      upload.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(openDialog).toHaveBeenCalledTimes(1);

    await act(async () => {
      resolveDialog("/ws/ok.txt");
    });
    await flush();
    expect(importAttachmentArtifact).toHaveBeenCalledTimes(1);
    expect(buttonByText("Upload")?.disabled).toBe(false);
  });
});

describe("artifactsPanel card", () => {
  it("opens the artifact detail from the card body and the actions button", async () => {
    await mount({ artifacts: [artifact({ id: "a7", title: "report.md" })] });
    await act(async () => {
      buttonByLabelPrefix("View artifact")?.click();
    });
    expect(onSelectArtifact).toHaveBeenCalledWith("a7");

    await act(async () => {
      buttonByLabelPrefix("Artifact actions")?.click();
    });
    await act(async () => {
      menuItem("View details")?.click();
    });
    expect(onSelectArtifact).toHaveBeenCalledTimes(2);
  });

  it("renders stored inline content when there is no file path", async () => {
    await mount({ artifacts: [artifact({ artifactType: "text", content: "hello inline", title: "note" })] });
    const pre = container.querySelector("pre code");
    expect(pre?.textContent).toBe("hello inline");
  });

  it("offers attach/open/preview only for a file-backed, previewable artifact", async () => {
    await mount({
      artifacts: [
        artifact({ id: "a1", path: "/ws/docs/report.md", title: "report.md" }),
        artifact({ id: "a2", path: "/ws/data.bin", title: "data.bin" }),
        artifact({ id: "a3", title: "inline only", content: "x" }),
      ],
    });

    await openCardMenu(0);
    expect(menuItemIn(0, "Attach to context")).toBeTruthy();
    expect(menuItemIn(0, "Preview")).toBeTruthy();
    expect(menuItemIn(0, "Open")).toBeTruthy();
    expect(menuItemIn(0, "Delete")).toBeTruthy();

    // A non-previewable file: attach + open, but no inline preview.
    await openCardMenu(1);
    expect(menuItemIn(1, "Attach to context")).toBeTruthy();
    expect(menuItemIn(1, "Open")).toBeTruthy();
    expect(menuItemIn(1, "Preview")).toBeUndefined();
    // …and the earlier card's menu is untouched by that.
    expect(menuItemIn(0, "Preview")).toBeTruthy();

    // No path at all: details + delete only.
    await openCardMenu(2);
    expect(menuItemIn(2, "Attach to context")).toBeUndefined();
    expect(menuItemIn(2, "Open")).toBeUndefined();
    expect(menuItemIn(2, "Preview")).toBeUndefined();
    expect(menuItemIn(2, "View details")).toBeTruthy();
  });

  it("attaches the artifact as a workspace-relative POSIX path", async () => {
    const events: Array<{ name: string; path: string }> = [];
    const listener = (event: Event) => {
      events.push((event as CustomEvent<{ name: string; path: string }>).detail);
    };
    window.addEventListener("futureos:attach-file-to-context", listener);
    try {
      await mount({
        artifacts: [artifact({ path: "C:\\ws\\docs\\report.md", title: "report.md" })],
        workspacePath: "C:\\ws",
      });
      await openCardMenu();
      await act(async () => {
        menuItem("Attach to context")?.click();
      });
      expect(events).toEqual([{ name: "report.md", path: "docs/report.md" }]);
    }
    finally {
      window.removeEventListener("futureos:attach-file-to-context", listener);
    }
  });

  it("keeps an out-of-workspace path absolute when attaching", async () => {
    const events: Array<{ name: string; path: string }> = [];
    const listener = (event: Event) => {
      events.push((event as CustomEvent<{ name: string; path: string }>).detail);
    };
    window.addEventListener("futureos:attach-file-to-context", listener);
    try {
      await mount({ artifacts: [artifact({ path: "/elsewhere/report.md", title: "report.md" })], workspacePath: "/ws" });
      await openCardMenu();
      await act(async () => {
        menuItem("Attach to context")?.click();
      });
      expect(events).toEqual([{ name: "report.md", path: "/elsewhere/report.md" }]);
    }
    finally {
      window.removeEventListener("futureos:attach-file-to-context", listener);
    }
  });

  it("opens the file with the OS handler", async () => {
    await mount({ artifacts: [artifact({ path: "/ws/report.md" })] });
    await openCardMenu();
    await act(async () => {
      menuItem("Open")?.click();
    });
    await flush();
    expect(openPath).toHaveBeenCalledWith("/ws/report.md");
  });

  it("opens the preview overlay with the detected kind", async () => {
    await mount({ artifacts: [artifact({ path: "/ws/pic.PNG", title: "pic.PNG" })] });
    expect(container.querySelector("[data-testid='preview-overlay']")?.getAttribute("data-open")).toBe("false");

    await openCardMenu();
    await act(async () => {
      menuItem("Preview")?.click();
    });
    const overlay = container.querySelector("[data-testid='preview-overlay']");
    expect(overlay?.getAttribute("data-open")).toBe("true");
    expect(overlay?.getAttribute("data-kind")).toBe("image");
    expect(overlay?.getAttribute("data-path")).toBe("/ws/pic.PNG");

    // Closing the overlay from inside it returns the card to its normal state.
    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid='overlay-close']")?.click();
    });
    expect(container.querySelector("[data-testid='preview-overlay']")?.getAttribute("data-open")).toBe("false");
  });

  it("opens the original file from the overlay, tolerating a handler that rejects", async () => {
    openPath.mockRejectedValue(new Error("no OS handler"));
    await mount({ artifacts: [artifact({ path: "/ws/pic.png" })] });
    await openCardMenu();
    await act(async () => {
      menuItem("Preview")?.click();
    });
    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-testid='overlay-external']")?.click();
    });
    await flush();
    expect(openPath).toHaveBeenCalledWith("/ws/pic.png");
    // The rejection is swallowed: no toast, no error box in the panel.
    expect(container.textContent).not.toContain("no OS handler");
  });

  it.each([
    ["image", artifact({ artifactType: "image", path: "/ws/snapshot", title: "snapshot" }), "lucide-file-image"],
    ["pdf", artifact({ artifactType: "pdf", path: "/ws/spec", title: "spec" }), "lucide-book-text"],
    ["code with an unknown extension", artifact({ artifactType: "code", path: "/ws/tool.weird", title: "tool" }), "lucide-file-braces"],
    ["markdown path", artifact({ artifactType: "document", path: "/ws/notes.md", title: "notes" }), "lucide-file-down"],
    ["unknown type and extension", artifact({ artifactType: "document", path: "/ws/blob.weird", title: "blob" }), "lucide-file-text"],
  ])("badges %s artifacts with the matching glyph", async (_label, panel, glyph) => {
    await mount({ artifacts: [panel] });
    // The badge is the card's accent-coloured glyph, not the toolbar's Upload icon.
    const badge = [...container.querySelectorAll("svg")]
      .map(svg => svg.getAttribute("class") ?? "")
      .find(className => className.includes("text-accent"));
    expect(badge).toContain(glyph);
  });

  it("deletes an artifact and refreshes the list", async () => {
    await mount({ artifacts: [artifact({ id: "a9" })] });
    await openCardMenu();
    await act(async () => {
      menuItem("Delete")?.click();
    });
    await flush();
    expect(deleteArtifact).toHaveBeenCalledWith("a9");
    expect(onChanged).toHaveBeenCalledTimes(1);
  });

  it("raises a toast when the delete fails", async () => {
    deleteArtifact.mockRejectedValue(new Error("artifact not found"));
    const toasts: Array<{ message: string; tone?: string }> = [];
    const listener = (event: Event) => {
      toasts.push((event as CustomEvent<{ message: string; tone?: string }>).detail);
    };
    window.addEventListener("futureos:toast", listener);
    try {
      await mount({ artifacts: [artifact({ id: "a9" })] });
      await openCardMenu();
      await act(async () => {
        menuItem("Delete")?.click();
      });
      await flush();
      expect(toasts).toEqual([{ message: "Failed to delete artifact: artifact not found", tone: "error" }]);
      expect(onChanged).not.toHaveBeenCalled();
    }
    finally {
      window.removeEventListener("futureos:toast", listener);
    }
  });
});
