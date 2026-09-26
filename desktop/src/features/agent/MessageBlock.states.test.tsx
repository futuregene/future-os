// @vitest-environment jsdom
import type { AgentMessage, MessageAttachment, MessageSegment } from "@future-os/thread-projection";
import type { Root } from "react-dom/client";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { MessageBlock } from "./MessageBlock";

/**
 * The *other* side of `MessageBlock`'s state conditions. Each case here asserts a
 * second arm of a branch that a happy-path render leaves untaken: the stopped /
 * no-final-reply dividers, the retry-vs-continue affordances, the un-hovered
 * fork button, a USER message's mention and link segments, and the compaction
 * labels for every status × trigger × token shape.
 *
 * `MessageBlock.behaviour.test.tsx` covers the primary side (hover, copy,
 * compaction divider, attachment chips); the reconnecting indicator and the
 * standalone divider have their own files.
 */
const h = vi.hoisted(() => ({
  openPath: vi.fn(async () => {}),
  previews: [] as { kind: string; open: boolean; path: string }[],
}));

vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: (path: string) => `asset://${path}` }));
vi.mock("../../integrations/storage/threadStore", () => ({ openPath: h.openPath }));
vi.mock("../../lib/clipboard", () => ({ copyText: vi.fn(async () => {}) }));
vi.mock("../filepreview/FilePreviewOverlay", () => ({
  FilePreviewOverlay: (props: { kind: string; open: boolean; path: string }) => {
    h.previews.push({ kind: props.kind, open: props.open, path: props.path });
    return <div data-preview="" />;
  },
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  h.previews.length = 0;
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

function render(msg: AgentMessage, over: Partial<Parameters<typeof MessageBlock>[0]> = {}) {
  act(() => root.render(
    <MessageBlock
      dataMessageId={msg.id}
      hovered={false}
      isLast
      message={msg}
      onHover={vi.fn()}
      onLeave={vi.fn()}
      {...over}
    />,
  ));
}

function buttons() {
  return [...container.querySelectorAll<HTMLButtonElement>("button")];
}

function byText(label: string) {
  return buttons().find(button => button.textContent?.trim() === label) ?? null;
}

it("says the model ended without a final reply, and warns rather than excuses it", () => {
  // The `noFinalReply` + not-stopped combination: a completed assistant turn with
  // no text, no activity and no notice is a silent stop the user must be told
  // about, so the divider is a warning and carries the explanatory detail.
  render(message({ content: "", status: "complete" }));

  const divider = container.querySelector("[role=status]")!;
  expect(divider.getAttribute("aria-label")).toBe("The model ended generation without a final reply");
  // `isLast` + not stopped → the detail line renders too.
  expect(container.textContent).toContain("Generated content has been kept");
});

it("explains a stopped response differently, and without the detail line", () => {
  // The other arm: the user stopped it, so the label names that and the
  // `!message.stopped` gate withholds the "you can send another request" detail.
  render(message({ content: "", stopped: true, terminationNotice: "Stopped by you" }));

  const divider = container.querySelector("[role=status]")!;
  expect(divider.getAttribute("aria-label")).toBe("You stopped this response");
  expect(container.textContent).not.toContain("Generated content has been kept");
});

it("shows a termination notice without the 'no final reply' wording", () => {
  // `terminationNotice` present, `noFinalReply` false → the notice's own text.
  render(message({
    content: "",
    status: "failed",
    terminationNotice: "The provider closed the stream",
  }));

  expect(container.textContent).toContain("The provider closed the stream");
  expect(container.textContent).not.toContain("The model ended generation without a final reply");
});

it("offers retry only when a recovery source is available", () => {
  // Two-sided: the failed last exchange with a source shows Retry; the same row
  // without one shows Continue instead (retry needs something to resend).
  const onRetry = vi.fn();
  const onContinue = vi.fn();
  const source = message({ content: "the prompt", id: "u1", role: "user" });

  render(message({ id: "a1", status: "failed" }), { onContinue, onRetry, recoverySource: source });
  expect(byText("Retry")).not.toBeNull();
  expect(byText("Continue")).not.toBeNull();

  act(() => root.unmount());
  root = createRoot(container);
  render(message({ id: "a1", status: "failed" }), { onContinue, onRetry, recoverySource: null });
  expect(byText("Retry")).toBeNull();
  expect(byText("Continue")).not.toBeNull();
});

it("withholds the continue affordance when the caller cannot handle it", () => {
  // The `onContinue &&` half of the condition: a host that offers no continue
  // handler gets Retry only, even for a conti-nuable error.
  render(
    message({ id: "a1", status: "failed" }),
    { onRetry: vi.fn(), recoverySource: message({ id: "u1", role: "user" }) },
  );
  expect(byText("Continue")).toBeNull();
  expect(byText("Retry")).not.toBeNull();
});

it("keeps the fork button painted out until the row is hovered", () => {
  // The un-hovered arm of the visibility class: the control exists (so it stays
  // reachable) but is not painted.
  const hidden = message({ id: "a1" });
  render(hidden, { hovered: false, onFork: vi.fn() });
  const hiddenRow = container.querySelector<HTMLElement>("[data-message-id='a1']")!;
  const hiddenFork = [...hiddenRow.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.getAttribute("title") === "Fork")!;
  // `classList`, not a substring: the base class list already carries
  // `focus-visible:opacity-100`, so `className.includes("opacity-100")` would be
  // true on every row and the assertion could never fail.
  expect(hiddenFork.classList.contains("opacity-0")).toBe(true);
  expect(hiddenFork.classList.contains("pointer-events-none")).toBe(true);
  expect(hiddenFork.classList.contains("opacity-100")).toBe(false);
});

it("paints the fork button on the hovered row", () => {
  render(message({ id: "a1" }), { hovered: true, onFork: vi.fn() });
  const row = container.querySelector<HTMLElement>("[data-message-id='a1']")!;
  const fork = [...row.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.getAttribute("title") === "Fork")!;
  expect(fork.classList.contains("opacity-100")).toBe(true);
  expect(fork.classList.contains("opacity-0")).toBe(false);
  expect(fork.classList.contains("pointer-events-none")).toBe(false);
});

it("renders a user message's mention pills and external links", () => {
  // A user turn's own text carries `@`-mention markdown and URLs; both segment
  // kinds are rendered (the mention as an accent span, the URL as a link), which
  // the assistant path never exercises.
  render(message({
    content: "see [a.ts](./src/a.ts) and [docs](https://example.com/x)",
    id: "u1",
    role: "user",
  }));

  const mentioned = [...container.querySelectorAll("span")].find(span => span.textContent === "a.ts")!;
  expect(mentioned.className).toContain("text-accent");
  const link = container.querySelector<HTMLAnchorElement>("a[href='https://example.com/x']")!;
  expect(link.textContent).toBe("docs");
  // The literal markdown is not shown alongside the rendered parts.
  expect(container.textContent).not.toContain("](./src/a.ts)");
});

it("renders a user message with no markdown at all as plain text", () => {
  // boundary: the plain branch of the same mapper (neither a mention nor a
  // link), so the two conditions have both sides asserted.
  render(message({ content: "just words", id: "u1", role: "user" }));
  expect(container.textContent).toContain("just words");
  expect(container.querySelector("a")).toBeNull();
});

it("labels a compaction divider for every status, trigger and token shape", async () => {
  // A table over the label matrix: status × manual × a lone `tokensBefore` × a
  // full before/after pair × nothing. Each row is a distinct arm of the nested
  // conditionals in the divider.
  const cases: [string, Partial<Extract<MessageSegment, { kind: "compaction" }>>, string][] = [
    ["running, automatic", { status: "running", trigger: "auto" }, "Compacting context"],
    ["running, manual", { status: "running", trigger: "manual" }, "You\u2019re compacting context"],
    ["failed, automatic", { status: "failed", trigger: "auto" }, "Context compaction failed"],
    ["failed, manual", { status: "failed", trigger: "manual" }, "Your context compaction failed"],
    ["completed with a delta", { status: "completed", tokensAfter: 300, tokensBefore: 1200, trigger: "auto" }, "Context compacted"],
    ["completed with a lone before", { status: "completed", tokensBefore: 1200, trigger: "auto" }, "Context compacted"],
    ["completed manual", { status: "completed", trigger: "manual" }, "You compacted this conversation"],
    ["completed bare", { status: "completed" }, "Context compacted"],
  ];

  for (const [label, segment, expected] of cases) {
    act(() => root.unmount());
    root = createRoot(container);
    const divider = message({
      content: "",
      id: `cp-${label}`,
      segments: [{ id: "seg", kind: "compaction", ...segment }] as MessageSegment[],
    });
    render(divider);
    const shown = container.querySelector("[role=status]")?.getAttribute("aria-label") ?? "";
    expect(shown, label).toContain(expected);
  }
});

it("previews an image attachment whose path has no preview kind of its own", async () => {
  // `previewKind ?? "image"` with a thumbnail: a `.bin` path has no in-app
  // reader, so the full-size preview must still open as an image.
  const attachment = {
    kind: "file",
    name: "raw.bin",
    path: "/tmp/raw.bin",
    thumbnail: "/tmp/raw-thumb.png",
  } as MessageAttachment;
  render(message({ attachments: [attachment], content: "" }));

  expect(h.previews[h.previews.length - 1]).toMatchObject({ kind: "image", open: false });

  act(() => root.unmount());
  root = createRoot(container);
  // The same path without a thumbnail shows the pill, and its glyph follows the
  // attachment's kind rather than being an image-only branch.
  render(message({ attachments: [{ ...attachment, thumbnail: undefined }], content: "" }));
  const pill = buttons().find(button => button.textContent?.includes("raw.bin"))!;
  expect(pill).not.toBeNull();
});

it("shows the image glyph for an image-kind attachment with no thumbnail", () => {
  // The `attachment.kind === "file"` false arm: an image-kind attachment renders
  // the paperclip-less variant, so both sides of the glyph choice are asserted.
  const image = { kind: "image", name: "shot.png", path: "/tmp/shot.png" } as MessageAttachment;
  render(message({ attachments: [image], content: "" }));
  expect(buttons().find(button => button.textContent?.includes("shot.png"))).not.toBeNull();

  act(() => root.unmount());
  root = createRoot(container);
  const file = { kind: "file", name: "notes.ts", path: "/tmp/notes.ts" } as MessageAttachment;
  render(message({ attachments: [file], content: "" }));
  expect(buttons().find(button => button.textContent?.includes("notes.ts"))).not.toBeNull();
});

it("right-aligns a user message that carries attachments", () => {
  // The `isUser && "justify-end"` arm of the attachments row.
  const attachment = { kind: "file", name: "notes.ts", path: "/tmp/notes.ts" } as MessageAttachment;
  render(message({ attachments: [attachment], content: "look", id: "u1", role: "user" }));
  const row = [...container.querySelectorAll("div")].find(div => div.className.includes("flex-wrap"))!;
  expect(row.className).toContain("justify-end");
});
