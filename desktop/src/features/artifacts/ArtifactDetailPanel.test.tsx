import type { StoredArtifact } from "../../integrations/storage/types";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync } from "../../test/renderHook";
import { ArtifactDetailPanel } from "./ArtifactDetailPanel";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(() => Promise.resolve(null));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `asset:${path}`,
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: async () => null,
  save: async () => null,
}));

// Deterministic highlighting; the real module keeps the production bound.
let highlighterState: {
  highlight: (code: string, language?: string) => unknown;
  isLoaded: boolean;
};

vi.mock("../markdown/useCodeHighlighter", async importOriginal => ({
  ...await importOriginal<typeof import("../markdown/useCodeHighlighter")>(),
  useCodeHighlighter: () => highlighterState,
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const source = "fn main() {\n    let x = 1;\n}\n";

function artifact(path: string, artifactType = "code"): StoredArtifact {
  return { id: "a1", workspaceId: "w1", title: "main.rs", createdAt: 1, updatedAt: 1, artifactType, path };
}

function mount(panel: StoredArtifact) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => {
    root.render(createElement(ArtifactDetailPanel, { artifact: panel, onBack: () => {}, onChanged: () => {} }));
  });
  return {
    container,
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

beforeEach(() => {
  highlighterState = { highlight: () => null, isLoaded: false };
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd) => {
    if (cmd === "inspect_attachment")
      return Promise.resolve({ isDir: false, size: source.length, isBinary: false });
    if (cmd === "read_text_file_preview")
      return Promise.resolve({ content: source, size: source.length, truncated: false, validUtf8: true });
    return Promise.resolve(null);
  });
});

describe("artifact code preview", () => {
  it("colors a code file the same way the fullscreen preview does", async () => {
    const highlight = vi.fn(() => ({
      fgColor: "#24292e",
      lines: source.split("\n").map(content => ({
        tokens: content ? [{ content, color: "#d73a49" }] : [],
      })),
    }));
    highlighterState = { highlight, isLoaded: true };
    const { container, cleanup } = mount(artifact("/w/main.rs"));
    await flushAsync();
    expect(highlight).toHaveBeenCalledWith(source, "rust");
    const pre = container.querySelector("pre") as HTMLPreElement;
    expect(pre.textContent).toBe(source);
    expect((pre.querySelector("span span") as HTMLSpanElement).style.color).toBe("rgb(215, 58, 73)");
    cleanup();
  });

  it.each([
    ["a file with no grammar", artifact("/w/notes.txt", "text")],
    ["a pathless artifact", { ...artifact(""), path: undefined, content: source } as StoredArtifact],
    ["a file past the highlight bound", artifact("/w/huge.rs")],
  ])("leaves %s as plain monospace", async (_case, panel) => {
    const content = panel.path === "/w/huge.rs" ? "x".repeat(100_001) : source;
    invokeMock.mockImplementation((cmd) => {
      if (cmd === "inspect_attachment")
        return Promise.resolve({ isDir: false, size: content.length, isBinary: false });
      if (cmd === "read_text_file_preview")
        return Promise.resolve({ content, size: content.length, truncated: true, validUtf8: true });
      return Promise.resolve(null);
    });
    const highlight = vi.fn(() => ({ fgColor: "#24292e", lines: [{ tokens: [{ content, color: "#111" }] }] }));
    highlighterState = { highlight, isLoaded: true };
    const { container, cleanup } = mount(panel);
    await flushAsync();
    expect(highlight).not.toHaveBeenCalled();
    const pre = container.querySelector("pre") as HTMLPreElement;
    expect(pre.textContent).toBe(content);
    expect(pre.querySelector("span")).toBeNull();
    cleanup();
  });
});
