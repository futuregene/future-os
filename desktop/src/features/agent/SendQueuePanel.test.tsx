// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { SendQueuePanel } from "./SendQueuePanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * Mount the panel. The rows are one press away from the header (the panel opens
 * closed, like the phone's), so a test that reads rows opens it first — unless it
 * is the test about the closed state itself.
 */
function renderPanel(overrides: Partial<Parameters<typeof SendQueuePanel>[0]> = {}, options: { expand?: boolean } = {}) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const props = {
    items: [
      { attachments: [], content: "first message", id: "a" },
      {
        attachments: [
          { kind: "image", name: "shot.png", path: "/tmp/shot.png", size: 4096 },
          { kind: "file", name: "doc.pdf", path: "/tmp/doc.pdf", size: 2048 },
        ],
        content: "",
        id: "b",
      },
    ],
    onRemove: vi.fn(),
    onRetry: vi.fn(),
    onUpdate: vi.fn(),
    ...overrides,
  };
  act(() => root.render(<SendQueuePanel {...props} />));
  if (options.expand !== false)
    act(() => host.querySelector<HTMLButtonElement>("[data-queue-toggle]")!.click());
  return {
    host,
    props,
    cleanup: () => {
      act(() => root.unmount());
      host.remove();
    },
  };
}

/**
 * A queue nobody asked to see is a queue that does not cover the reply.
 *
 * Every row the panel draws is a row of the conversation pushed further up, and
 * the header already says how many are waiting and what will become of them — so
 * the panel opens as that line, exactly as the phone's does, and the rows are one
 * press away.
 */
it("opens as its header line, with the rows one press away", () => {
  const panel = renderPanel({}, { expand: false });
  try {
    const header = panel.host.querySelector("[data-send-queue]")!;
    expect(header.textContent).toContain("Send queue");
    expect(header.textContent).toContain("2 waiting");
    expect(header.textContent).toContain("Sent as one message when this reply ends");
    // Nothing of the rows is drawn while it is closed…
    expect(panel.host.querySelectorAll("[data-queue-item]")).toHaveLength(0);
    expect(panel.host.querySelector("[data-queue-list]")).toBeNull();
    const toggle = panel.host.querySelector<HTMLButtonElement>("[data-queue-toggle]")!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(toggle.getAttribute("aria-label")).toBe("Show the queued messages");

    // …and the whole header is the control that opens them.
    act(() => toggle.click());
    expect(panel.host.querySelectorAll("[data-queue-item]")).toHaveLength(2);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(toggle.getAttribute("aria-label")).toBe("Hide the queued messages");

    act(() => toggle.click());
    expect(panel.host.querySelectorAll("[data-queue-item]")).toHaveLength(0);
  }
  finally {
    panel.cleanup();
  }
});

it("folds an edit that was open away with the rows", () => {
  // The editor lives inside the rows. A reader who folded the queue with one open
  // walked away from those words; drawing the field again, half-typed, on the
  // next open would put them back in an edit they thought they had left.
  const panel = renderPanel();
  try {
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-edit]")!.click());
    expect(panel.host.querySelector("textarea")).not.toBeNull();
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-toggle]")!.click());
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-toggle]")!.click());
    expect(panel.host.querySelectorAll("[data-queue-item]")).toHaveLength(2);
    expect(panel.host.querySelector("textarea")).toBeNull();
    expect(panel.props.onUpdate).not.toHaveBeenCalled();
  }
  finally {
    panel.cleanup();
  }
});

it("lists what is waiting, with an attachment-only row said in words", () => {
  const panel = renderPanel();
  try {
    expect(panel.host.querySelector("[data-send-queue]")?.textContent).toContain("Send queue");
    expect(panel.host.querySelectorAll("[data-queue-item]")).toHaveLength(2);
    expect(panel.host.textContent).toContain("first message");
    // An entry whose words are gone still travels while it has files.
    expect(panel.host.textContent).toContain("Attachments only");
    // …and the files are named, not counted: a chip is what can be looked at
    // before the run takes it.
    expect(panel.host.textContent).toContain("shot.png");
    expect(panel.host.textContent).toContain("doc.pdf");
    expect(panel.host.querySelectorAll("[data-queue-attachment]")).toHaveLength(2);
  }
  finally {
    panel.cleanup();
  }
});

it("edits a queued message in place and saves the new text", () => {
  const panel = renderPanel();
  try {
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-edit]")!.click());
    const field = panel.host.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(field.value).toBe("first message");
    // A controlled field only updates through React's own value setter.
    const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    act(() => {
      setValue.call(field, "second thoughts");
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-save]")!.click());
    expect(panel.props.onUpdate).toHaveBeenCalledWith("a", "second thoughts");
    expect(panel.host.querySelector("textarea")).toBeNull();
  }
  finally {
    panel.cleanup();
  }
});

it("takes a message back out of the queue", () => {
  const panel = renderPanel();
  try {
    act(() => panel.host.querySelectorAll<HTMLButtonElement>("[data-queue-remove]")[1]!.click());
    expect(panel.props.onRemove).toHaveBeenCalledWith("b");
    expect(panel.props.onUpdate).not.toHaveBeenCalled();
  }
  finally {
    panel.cleanup();
  }
});

it("stops promising the merge once a row is parked", () => {
  // A parked row never runs when the reply ends — it waits for the person reading
  // it — so the header that says the queue is merged and sent at the boundary is
  // simply wrong about it, and leaves the reader wondering why nothing went out.
  const fresh = renderPanel({
    items: [
      { content: "will run by itself", id: "a" },
      { content: "and so will this one", id: "c" },
    ],
  });
  try {
    expect(fresh.host.textContent).toContain("Sent as one message when this reply ends");
  }
  finally {
    fresh.cleanup();
  }
  const parked = renderPanel({
    items: [
      { content: "will run by itself", id: "a" },
      { content: "held row", id: "b", parked: true },
    ],
  });
  try {
    expect(parked.host.textContent).not.toContain("Sent as one message when this reply ends");
    expect(parked.host.textContent).toContain("press “Send this one now”");
  }
  finally {
    parked.cleanup();
  }
});

/**
 * One waiting message is not merged with anything.
 *
 * The Agent folds the coalescing requests queued *behind* the front one into
 * it (`drain_coalescing_after_first`); a lone row waits and then runs as its
 * own message, with its own reply. Promising a merge there described something
 * the queue never does, which is exactly what a report of "the header says
 * merged but the second message ran on its own" sees.
 */
it("describes a lone waiting message as its own message, not a merge", () => {
  const panel = renderPanel({ items: [{ content: "the only one", id: "a" }] });
  try {
    expect(panel.host.textContent).toContain("Sent as its own message when this reply ends");
    expect(panel.host.textContent).not.toContain("Sent as one message when this reply ends");
  }
  finally {
    panel.cleanup();
  }
});

it("offers a retry instead of the hint after a failed automatic send", () => {
  const panel = renderPanel({ failed: true });
  try {
    expect(panel.host.textContent).not.toContain("Sent as one message when this reply ends");
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-retry]")!.click());
    expect(panel.props.onRetry).toHaveBeenCalledTimes(1);
  }
  finally {
    panel.cleanup();
  }
});

it("offers a parked row a start, and only a parked one", () => {
  // The Agent runs a `queued` row by itself at the turn boundary; a `stale` one
  // waits for a person, so that row — and only that row — carries the control.
  const onRelease = vi.fn();
  const panel = renderPanel({
    items: [
      { attachments: [], content: "will run by itself", id: "a" },
      { attachments: [], content: "waited too long", id: "b", parked: true },
    ],
    onRelease,
  });
  try {
    const releases = panel.host.querySelectorAll("[data-queue-release]");
    expect(releases).toHaveLength(1);
    act(() => (releases[0] as HTMLButtonElement).click());
    expect(onRelease).toHaveBeenCalledWith("b");
  }
  finally {
    panel.cleanup();
  }
});

it("hides the start control when the queue cannot be started by hand", () => {
  // A client-side queue (a paired computer's, or the phone's) has no Agent row
  // behind a parked flag, and must not show a button that does nothing.
  const panel = renderPanel({
    items: [{ attachments: [], content: "waiting", id: "a", parked: true }],
  });
  try {
    expect(panel.host.querySelector("[data-queue-release]")).toBeNull();
    // ...and the header names the parked state instead of a send it will not make.
    expect(panel.host.textContent).not.toContain("Sent as its own message");
  }
  finally {
    panel.cleanup();
  }
});

/**
 * A row read back from a session's queue carries text and an acceptance time,
 * never the files it was sent with. Claiming "0" there told the reader a message
 * was text-only when it may have carried files, and an edit — which is a removal
 * and a re-send, the only kind there is — then dropped them with nobody warned.
 */
it("says what an edit costs when the queue does not report a message's files", () => {
  const panel = renderPanel({
    attachmentsUnreported: true,
    items: [{ content: "look at this", id: "a" }],
  });
  try {
    // Nothing is claimed about files the panel cannot see…
    expect(panel.host.querySelector("[data-queue-attachments]")).toBeNull();
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-edit]")!.click());
    // …and the trade is named where it is made, before the save.
    expect(panel.host.textContent).toContain("Saving sends the new text only");
    expect(panel.props.onUpdate).not.toHaveBeenCalled();
  }
  finally {
    panel.cleanup();
  }
});

it("does not offer an edit for a message that is only files", () => {
  // There is no text to correct, and an empty save is a removal: the bin beside
  // it already says that plainly.
  const panel = renderPanel({ items: [{ attachments: [], content: "", id: "files-only" }] });
  try {
    expect(panel.host.querySelector("[data-queue-edit]")).toBeNull();
    expect(panel.host.querySelector("[data-queue-remove]")).not.toBeNull();
    expect(panel.host.textContent).toContain("Attachments only");
  }
  finally {
    panel.cleanup();
  }
});

/**
 * The files are named, so they can be looked at before the run takes them.
 *
 * The panel used to depend on a count from a queue that never reported one, so a
 * waiting message with photos drew as text and nothing else — the reader could
 * not see what was about to be sent, let alone open it.
 */
it("names each file a waiting message carries and opens the one that is clicked", () => {
  const onPreviewAttachment = vi.fn();
  const panel = renderPanel({ onPreviewAttachment });
  try {
    const chips = panel.host.querySelectorAll<HTMLButtonElement>("[data-queue-attachment]");
    expect(chips).toHaveLength(2);
    expect(chips[0]!.textContent).toContain("shot.png");
    expect(chips[0]!.disabled).toBe(false);
    act(() => chips[0]!.click());
    expect(onPreviewAttachment).toHaveBeenCalledWith(
      { kind: "image", name: "shot.png", path: "/tmp/shot.png", size: 4096 },
    );
  }
  finally {
    panel.cleanup();
  }
});

it("names the files even when nothing can open them", () => {
  // The client that knows how to open one gives the handler; without it the chips
  // are still named, because what a message carries is worth reading even when
  // this window cannot show it.
  const panel = renderPanel();
  try {
    const chips = panel.host.querySelectorAll<HTMLButtonElement>("[data-queue-attachment]");
    expect(chips).toHaveLength(2);
    expect(chips[0]!.disabled).toBe(true);
  }
  finally {
    panel.cleanup();
  }
});

/**
 * A row that reports its files does not warn about losing them.
 *
 * The warning used to be a property of the whole panel, so a queue that had
 * started reporting files kept telling the reader an edit would drop them — a
 * cost that no longer existed, said at the moment of saving.
 */
it("does not warn about dropped files on a row whose files it can see", () => {
  const panel = renderPanel({
    attachmentsUnreported: true,
    items: [{ attachments: [{ kind: "file", name: "notes.md", path: "/tmp/notes.md", size: 10 }], content: "read this", id: "a" }],
  });
  try {
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-edit]")!.click());
    expect(panel.host.textContent).not.toContain("Saving sends the new text only");
  }
  finally {
    panel.cleanup();
  }
});

/**
 * The pick a queued message carries (§5.8.2).
 *
 * The row already has a pencil for its words. A chip carrying a second one a
 * few pixels away — meaning "change the answer" while the first means "change
 * the sentence" — is the ambiguity this whole design exists to remove, so the
 * chip *is* the button and wears a chevron instead.
 */
const ANSWER = {
  mode: "single" as const,
  options: [{ id: "forest", label: "重画森林图" }, { id: "flow", label: "补一张流程图" }],
  selectedIds: ["forest"],
  title: "接下来做哪一件？",
};

it("offers a queued answer as a button that opens the other options", () => {
  const onUpdateChoice = vi.fn();
  const panel = renderPanel({
    items: [{ choice: ANSWER, content: "[接下来做哪一件？ → 重画森林图]", id: "a" }],
    onUpdateChoice,
  });
  try {
    const chip = panel.host.querySelector<HTMLButtonElement>("[data-queue-choice-action=change]")!;
    expect(chip).not.toBeNull();
    expect(chip.tagName).toBe("BUTTON");
    expect(chip.getAttribute("aria-label")).toContain("接下来做哪一件？");
    expect(chip.getAttribute("aria-label")).toContain("重画森林图");
    expect(chip.disabled).toBe(false);

    act(() => chip.click());

    // The card's own rows, offered again without asking the transcript for it.
    // The menu is portalled out of the row, so it is found from the document.
    const rows = [...document.querySelectorAll("[role=menuitem], button, li")]
      .filter(node => node.textContent === "补一张流程图" || node.textContent === "重画森林图");
    expect(rows.map(node => node.textContent)).toEqual(["重画森林图", "补一张流程图"]);

    act(() => (rows[1] as HTMLElement).click());
    // The whole answer comes back, not just the id: the card's question and its
    // other options have to survive a change, or the chip could offer them once
    // and never again. This is the signature the phone's handler matches, and it
    // is the one that can grow a field without changing shape.
    const [rowId, answer] = onUpdateChoice.mock.calls[0]!;
    expect(rowId).toBe("a");
    expect(Array.isArray(answer)).toBe(false);
    expect(answer).toEqual({ ...ANSWER, selectedIds: ["flow"] });
  }
  finally {
    panel.cleanup();
  }
});

it("leaves the row's own pencil and bin doing what they always did", () => {
  // The constraint that keeps the two controls from meaning the same thing:
  // adding the chip must not remove, move or duplicate them.
  const onRemove = vi.fn();
  const onUpdate = vi.fn();
  const panel = renderPanel({
    items: [{ choice: ANSWER, content: "[接下来做哪一件？ → 重画森林图]", id: "a" }],
    onRemove,
    onUpdate,
    onUpdateChoice: vi.fn(),
  });
  try {
    const row = panel.host.querySelector("[data-queue-item]")!;
    expect(row.querySelectorAll("[data-queue-edit]")).toHaveLength(1);
    expect(row.querySelectorAll("[data-queue-remove]")).toHaveLength(1);
    // One icon, and it is not a pencil: the chip opens something, it does not edit.
    const chip = row.querySelector("[data-queue-choice-action=change]")!;
    expect(chip.querySelectorAll("svg")).toHaveLength(1);
    expect(chip.querySelector("svg")?.getAttribute("class") ?? "").not.toContain("lucide-pencil");
  }
  finally {
    panel.cleanup();
  }
});

it("draws no chip for a row that answered no card, or when nobody can act on it", () => {
  const plain = renderPanel();
  try {
    expect(plain.host.querySelectorAll("[data-queue-choice]")).toHaveLength(0);
  }
  finally {
    plain.cleanup();
  }
  // A handler-less panel (an older surface, or a test) draws the row it always
  // drew rather than a control that would do nothing.
  const readOnly = renderPanel({
    items: [{ choice: ANSWER, content: "x", id: "a" }],
  });
  try {
    const chip = readOnly.host.querySelector<HTMLButtonElement>("[data-queue-choice-action=change]");
    expect(chip?.disabled).toBe(true);
  }
  finally {
    readOnly.cleanup();
  }
});
