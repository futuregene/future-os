import { act } from "react";
// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";
import { renderHook } from "../../test/renderHook";
import { loadComposerDraft, saveComposerDraft } from "../agent/composerDraft";
import { useVibeQueue } from "./vibeQueue";

/**
 * The queue as the panel uses it: a hook whose answers are read synchronously.
 *
 * `enqueue` decides whether the composer may clear its box and `cancel` decides
 * whether the held text can go back into it — both are answers to a key press, so
 * they must be right in the same tick rather than after a state update. The pure
 * rules are covered in `vibeQueue.test.ts`; what is left here is the wiring: the live
 * queue the two synchronous answers read, and the hand-back when a run stops.
 */
const prompt = (text: string) => ({ attachments: [], text });

beforeEach(() => {
  // Drafts live in `sessionStorage`; the queue's own state is React state, so both
  // stores have to start empty for a case to mean what it says.
  localStorage.clear();
  sessionStorage.clear();
});

describe("useVibeQueue", () => {
  it("answers `none` for a cancel with nothing held, and leaves the draft alone", () => {
    // The cell's own cancel affordance is only drawn when something is queued, but a
    // key press can still land after the queue emptied: there must be nothing to hand
    // back, and nothing may be written into the box.
    const hook = renderHook(() => useVibeQueue(() => undefined, ""));
    expect(hook.current.cancel("t1")).toBe("none");
    expect(loadComposerDraft("t1")).toBeNull();
    expect(hook.current.queued.size).toBe(0);
    hook.unmount();
  });

  it("holds one sentence per conversation and refuses a second", () => {
    const hook = renderHook(() => useVibeQueue(() => "running", "running"));
    let first = "";
    let second = "";
    act(() => {
      first = hook.current.enqueue("t1", prompt("first"));
      second = hook.current.enqueue("t1", prompt("second"));
    });
    expect(first).toBe("queued");
    expect(second).toBe("refused");
    expect(hook.current.queued.get("t1")?.text).toBe("first");
    hook.unmount();
  });

  it("hands a held sentence back into an empty box", () => {
    const hook = renderHook(() => useVibeQueue(() => "running", "running"));
    act(() => {
      hook.current.enqueue("t1", prompt("held"));
    });
    let outcome = "";
    act(() => {
      outcome = hook.current.cancel("t1");
    });
    expect(outcome).toBe("cancelled");
    expect(loadComposerDraft("t1")?.text).toBe("held");
    expect(hook.current.queued.size).toBe(0);
    hook.unmount();
  });

  it("keeps the entry when the reader has started typing", () => {
    const hook = renderHook(() => useVibeQueue(() => "running", "running"));
    act(() => {
      hook.current.enqueue("t1", prompt("held"));
    });
    saveComposerDraft("t1", { attachments: [], text: "something new" });
    let outcome = "";
    act(() => {
      outcome = hook.current.cancel("t1");
    });
    expect(outcome).toBe("busy");
    expect(hook.current.queued.get("t1")?.text).toBe("held");
    expect(loadComposerDraft("t1")?.text).toBe("something new");
    hook.unmount();
  });

  it("hands a sentence back by itself once its run stops", () => {
    // The panel passes the joined run statuses as the signature, so this is the same
    // effect that runs when a run ends elsewhere.
    let running = true;
    const hook = renderHook(() => useVibeQueue(() => (running ? "running" : "completed"), String(running)));
    act(() => {
      hook.current.enqueue("t1", prompt("held"));
    });
    expect(hook.current.queued.get("t1")?.text).toBe("held");

    running = false;
    act(() => {
      hook.rerender();
    });

    expect(hook.current.queued.size).toBe(0);
    expect(loadComposerDraft("t1")?.text).toBe("held");
    expect(hook.current.returned.has("t1")).toBe(true);
  });

  it("stops marking a cell once the reader has seen the returned text", () => {
    let running = true;
    const hook = renderHook(() => useVibeQueue(() => (running ? "running" : "completed"), String(running)));
    act(() => {
      hook.current.enqueue("t1", prompt("held"));
    });
    running = false;
    act(() => {
      hook.rerender();
    });
    expect(hook.current.returned.has("t1")).toBe(true);

    act(() => {
      hook.current.acknowledge("t1");
    });

    expect(hook.current.returned.has("t1")).toBe(false);
  });

  it("drops the returned mark when the sentence is queued again", () => {
    // The mark says "this came back to your box"; a new queued sentence makes the cell
    // say what is actually true again.
    let running = true;
    const hook = renderHook(() => useVibeQueue(() => (running ? "running" : "completed"), String(running)));
    act(() => {
      hook.current.enqueue("t1", prompt("held"));
    });
    running = false;
    act(() => {
      hook.rerender();
    });
    expect(hook.current.returned.has("t1")).toBe(true);

    running = true;
    act(() => {
      hook.current.enqueue("t1", prompt("next"));
      hook.rerender();
    });

    expect(hook.current.returned.has("t1")).toBe(false);
    expect(hook.current.queued.get("t1")?.text).toBe("next");
  });
});
