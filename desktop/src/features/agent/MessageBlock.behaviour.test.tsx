// @vitest-environment jsdom
import type { AgentMessage, MessageAttachment, MessageSegment } from "@future-os/thread-projection";
import type { Root } from "react-dom/client";
import type { Mock } from "vitest";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { MessageBlock } from "./MessageBlock";

/**
 * MessageBlock's own wiring: the hover handlers, the retry/continue/fork
 * footer, the copy payload, the inline compaction divider, and the attachment
 * chip. The markdown renderer, the reconnecting status and the standalone
 * compaction divider have their own tests.
 */
const mocks = vi.hoisted(() => ({
  convertFileSrc: vi.fn((path: string) => `asset://${path}`),
  copyText: vi.fn(async () => {}),
  openPath: vi.fn(async () => {}),
  previews: [] as { kind: string; name: string; open: boolean; path: string }[],
}));

vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: mocks.convertFileSrc }));
vi.mock("../../integrations/storage/threadStore", () => ({ openPath: mocks.openPath }));
vi.mock("../../lib/clipboard", () => ({ copyText: mocks.copyText }));
vi.mock("../filepreview/FilePreviewOverlay", () => ({
  FilePreviewOverlay: (props: { kind: string; name: string; onClose: () => void; open: boolean; path: string }) => {
    mocks.previews.push({ kind: props.kind, name: props.name, open: props.open, path: props.path });
    return (
      <div data-kind={props.kind} data-open={String(props.open)} data-preview="">
        <button data-close-preview="" onClick={props.onClose} type="button" />
      </div>
    );
  },
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.previews.length = 0;
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: vi.fn(async () => {}) },
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function message(over: Partial<AgentMessage> = {}): AgentMessage {
  return {
    authorKey: "author.researchCopilot",
    content: "a reply",
    createdAt: new Date("2026-01-01T00:00:00Z").toISOString(),
    id: "m1",
    role: "assistant",
    status: "complete",
    ...over,
  } as AgentMessage;
}

function attachment(over: Partial<MessageAttachment> = {}): MessageAttachment {
  return { kind: "file", name: "notes.ts", path: "/work/notes.ts", ...over } as MessageAttachment;
}

interface Handlers {
  onContinue: Mock;
  onFork: Mock;
  onHover: Mock;
  onLeave: Mock;
  onRetry: Mock;
}

function render(msg: AgentMessage, over: Partial<Parameters<typeof MessageBlock>[0]> = {}, handlers?: Handlers) {
  const h: Handlers = handlers ?? {
    onContinue: vi.fn(),
    onFork: vi.fn(),
    onHover: vi.fn(),
    onLeave: vi.fn(),
    onRetry: vi.fn(),
  };
  act(() => root.render(
    <MessageBlock
      dataMessageId={msg.id}
      hovered
      isLast
      message={msg}
      onContinue={h.onContinue}
      onFork={h.onFork}
      onHover={h.onHover}
      onLeave={h.onLeave}
      onRetry={h.onRetry}
      {...over}
    />,
  ));
  return h;
}

function buttons() {
  return [...container.querySelectorAll<HTMLButtonElement>("button")];
}

function byTitle(title: string) {
  return container.querySelector<HTMLButtonElement>(`button[title="${title}"]`);
}

it("reports hover and leave with the message's own id", () => {
  const handlers = render(message({ id: "m7" }));
  const row = container.querySelector<HTMLElement>("[data-message-id='m7']")!;

  act(() => {
    row.dispatchEvent(new MouseEvent("pointerover", { bubbles: true }));
  });
  expect(handlers.onHover).toHaveBeenCalledWith("m7");

  act(() => {
    row.dispatchEvent(new MouseEvent("pointerout", { bubbles: true, relatedTarget: document.body }));
  });
  expect(handlers.onLeave).toHaveBeenCalledWith("m7");
});

it("offers retry and continue on the latest failed reply, with its recovery source", () => {
  // Only the last exchange may be recovered: retrying an older one would fork
  // the conversation behind the user's back.
  const source = message({ content: "the prompt", id: "u1", role: "user" });
  const handlers = render(
    message({ id: "a1", status: "failed" }),
    { isLast: true, recoverySource: source },
  );

  const retry = buttons().find(button => button.textContent?.includes("Retry"))!;
  const cont = buttons().find(button => button.textContent?.includes("Continue"))!;
  act(() => retry.click());
  act(() => cont.click());

  expect(handlers.onRetry).toHaveBeenCalledWith(expect.objectContaining({ id: "a1" }), source);
  expect(handlers.onContinue).toHaveBeenCalledWith(expect.objectContaining({ id: "a1" }));
});

it("withholds recovery for an interrupted run and for an earlier exchange", () => {
  // error-path: the agent may still be processing the original after a GUI
  // restart, so retrying would race the in-flight run.
  render(message({ id: "a1", status: "failed", stopped: true }), { isLast: true, recoverySource: message({ id: "u1", role: "user" }) });
  expect(buttons().some(button => button.textContent?.includes("Retry"))).toBe(false);

  act(() => root.unmount());
  root = createRoot(container);
  render(message({ id: "a1", status: "failed" }), { isLast: false, recoverySource: message({ id: "u1", role: "user" }) });
  expect(buttons().some(button => button.textContent?.includes("Retry"))).toBe(false);

  // A user row never offers its own recovery.
  act(() => root.unmount());
  root = createRoot(container);
  render(message({ id: "u1", role: "user", status: "failed" }), { isLast: true });
  expect(buttons().some(button => button.textContent?.includes("Retry"))).toBe(false);
});

it("copies the reply's text, joining segments and skipping activity lines", async () => {
  const segments: MessageSegment[] = [
    { id: "s1", kind: "text", text: "first part" },
    { id: "s2", kind: "activity", item: { id: "call", kind: "shell", status: "completed" } },
    { id: "s3", kind: "text", text: "second part" },
    { id: "s4", kind: "thinking", text: "hidden reasoning" },
  ] as MessageSegment[];
  render(message({ segments }));
  expect(container.textContent).toContain("first part");

  const copy = byTitle("Copy")!;
  await act(async () => {
    copy.click();
    await Promise.resolve();
  });

  expect(mocks.copyText).toHaveBeenCalledWith("first part\n\nsecond part");
});

it("copies the raw content, trimmed, when the reply is not segmented", async () => {
  render(message({ content: "  a single block  ", segments: undefined }));
  const copy = byTitle("Copy")!;
  await act(async () => {
    copy.click();
    await Promise.resolve();
  });
  expect(mocks.copyText).toHaveBeenCalledWith("a single block");
});

it("withholds the copy button while the reply is still streaming", () => {
  render(message({ status: "streaming" }));
  expect(byTitle("Copy")).toBeNull();
  // The live indicator replaces it.
  expect(container.querySelector("[role=status]")).not.toBeNull();
});

it("offers a fork for a settled reply, and hides it for a user turn or a streaming one", () => {
  render(message({ id: "a1" }));
  const fork = byTitle("Fork");
  expect(fork).not.toBeNull();
  const onFork = vi.fn();
  render(message({ id: "a1" }), { onFork });
  act(() => byTitle("Fork")!.click());
  expect(onFork).toHaveBeenCalledWith(expect.objectContaining({ id: "a1" }));

  act(() => root.unmount());
  root = createRoot(container);
  render(message({ id: "u1", role: "user" }));
  expect(byTitle("Fork")).toBeNull();

  act(() => root.unmount());
  root = createRoot(container);
  render(message({ id: "a1", status: "streaming" }));
  expect(byTitle("Fork")).toBeNull();
});

it("renders an inline compaction divider in the middle of a segmented reply", () => {
  const segments: MessageSegment[] = [
    { id: "s1", kind: "text", text: "before" },
    { id: "s2", kind: "compaction", status: "completed", tokensBefore: 1000, tokensAfter: 100, trigger: "manual" },
  ] as MessageSegment[];
  render(message({ segments }));

  expect(container.textContent).toContain("before");
  const dividers = container.querySelectorAll("[role=status]");
  expect(dividers.length).toBeGreaterThan(0);
  // The divider carries the token counts it was given.
  expect(container.textContent).toContain("100");
});

it("keeps a streaming reply's tail unhighlighted when only activity is growing", () => {
  // boundary: the live-marking loop walks back over activity/compaction
  // segments; with no text segment it must mark nothing (null) rather than
  // feeding an undefined id to the markdown renderer.
  const segments: MessageSegment[] = [
    { id: "a1", kind: "activity", item: { id: "call", kind: "shell", status: "running" } },
  ] as MessageSegment[];
  render(message({ segments, status: "streaming" }));

  expect(container.textContent).toContain("Running a command");
  expect(container.querySelector("[role=status]")).not.toBeNull();
});

it("shows the author and a formatted timestamp", () => {
  render(message({ createdAt: new Date("2026-03-04T05:06:07Z").toISOString() }));
  expect(container.textContent).toContain("03-04");
});

it("renders an attachment as a named pill and opens its in-app preview", async () => {
  render(message({ attachments: [attachment()], content: "see attached" }));

  const chip = buttons().find(button => button.textContent?.includes("notes.ts"))!;
  expect(chip).not.toBeNull();
  // A `kind: file` attachment with no thumbnail falls back to the named pill,
  // titled with the full path so a long name stays discoverable.
  expect(chip.getAttribute("title")).toBe("/work/notes.ts");

  await act(async () => {
    chip.click();
    await Promise.resolve();
  });
  // A `.ts` file has a preview kind, so the click opens the overlay rather than
  // shelling out to the OS handler.
  expect(mocks.openPath).not.toHaveBeenCalled();
  expect(mocks.previews[mocks.previews.length - 1]).toMatchObject({ kind: "text", open: true, path: "/work/notes.ts" });

  // Closing the overlay returns to the transcript.
  await act(async () => {
    container.querySelector<HTMLButtonElement>("[data-close-preview]")!.click();
    await Promise.resolve();
  });
  expect(mocks.previews[mocks.previews.length - 1]).toMatchObject({ kind: "text", open: false });
});

it("hands an attachment with no in-app preview to the OS handler", async () => {
  render(message({ attachments: [attachment({ kind: "file", name: "report.pdf", path: "/work/report.pdf" })] }));
  const chip = buttons().find(button => button.textContent?.includes("report.pdf"))!;

  await act(async () => {
    chip.click();
    await Promise.resolve();
  });

  expect(mocks.openPath).toHaveBeenCalledWith("/work/report.pdf");
  expect(mocks.previews).toEqual([]);
});

it("opens a previewable attachment's thumbnail overlay at full size", async () => {
  render(message({ attachments: [attachment({ name: "shot.png", path: "/work/shot.png", thumbnail: "/work/thumb.png" })] }));

  const thumb = buttons().find(button => button.getAttribute("title") === "shot.png")!;
  expect(thumb.querySelector("img")?.getAttribute("src")).toBe("asset:///work/thumb.png");
  expect(thumb.querySelector("img")?.getAttribute("alt")).toBe("shot.png");

  // The overlay is mounted closed and opened by the click.
  expect(mocks.previews[mocks.previews.length - 1]).toMatchObject({ kind: "image", open: false });
  await act(async () => {
    thumb.click();
    await Promise.resolve();
  });
  expect(mocks.previews[mocks.previews.length - 1]).toMatchObject({ kind: "image", open: true });

  // Closing the full-size preview returns to the thumbnail.
  await act(async () => {
    container.querySelector<HTMLButtonElement>("[data-close-preview]")!.click();
    await Promise.resolve();
  });
  expect(mocks.previews[mocks.previews.length - 1]).toMatchObject({ kind: "image", open: false });
});

it("falls back to the named pill when a thumbnail fails to load", () => {
  render(message({ attachments: [attachment({ name: "broken.png", path: "/work/broken.png", thumbnail: "/work/thumb.png" })] }));

  const img = container.querySelector("img")!;
  act(() => {
    img.dispatchEvent(new Event("error"));
  });

  // The broken preview is replaced by a pill carrying the file name, so the
  // attachment is still visible and still openable.
  expect(container.querySelector("img")).toBeNull();
  const pill = buttons().find(button => button.textContent?.includes("broken.png"))!;
  expect(pill).not.toBeNull();
});

it("toasts when an attachment that has no in-app preview is gone", async () => {
  // error-path: the file was moved or reclaimed; opening it must say so.
  const toasts: { message: string; tone?: string }[] = [];
  const off = onFutureEvent("toast", toast => void toasts.push(toast));
  mocks.openPath.mockRejectedValueOnce(new Error("no such file"));
  render(message({ attachments: [attachment({ name: "archive.zip", kind: "file", path: "/work/archive.zip" })] }));

  const chip = buttons().find(button => button.textContent?.includes("archive.zip"))!;
  await act(async () => {
    chip.click();
    await Promise.resolve();
  });

  expect(mocks.openPath).toHaveBeenCalledWith("/work/archive.zip");
  expect(toasts).toEqual([{ message: "File has been moved or deleted: archive.zip", tone: "error" }]);
  off();
});
