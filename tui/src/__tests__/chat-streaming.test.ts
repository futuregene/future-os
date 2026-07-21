/**
 * Streaming render tests for ChatArea.
 *
 * Guards the two streaming optimizations:
 *  1. Deferred re-render: deltas only mark the message dirty; the markdown
 *     pipeline runs at flush time (render/renderAll), once per frame.
 *  2. Prefix cache: while pending, blocks closed by a blank line outside
 *     code fences are rendered once and cached; only the tail re-renders.
 *     The incremental output must equal a fresh full render at every step.
 *
 * Run with: bun test
 */
import { describe, test, expect } from "bun:test";
import { ChatArea, type ChatMessage } from "../components/chat-area.js";

const W = 120;

/** Reach into privates for white-box assertions (compile-time only). */
function setMessages(chat: ChatArea, messages: ChatMessage[]): void {
  (chat as any).messages = messages;
  (chat as any).rerender();
}

function eagerLines(content: string, pending: boolean, width = W): string[] {
  const chat = new ChatArea(width);
  chat.render(width);
  setMessages(chat, [{ id: "m", role: "assistant", content, pending }]);
  return chat.renderAll(width);
}

/** Stream `full` into `chat` in random-ish chunks, checking every frame. */
function expectStreamingMatchesFullRender(full: string, width = W): number {
  const chat = new ChatArea(width);
  chat.render(width);
  chat.addMessage({ id: "m", role: "assistant", content: "" });

  let i = 0;
  let frames = 0;
  let n = 3; // deterministic varying chunk sizes
  while (i < full.length) {
    n = (n * 7 + 5) % 23 + 1;
    chat.appendToLastMessage(full.slice(i, i + n));
    i += n;
    const got = chat.renderAll(width);
    const want = eagerLines(full.slice(0, i), true, width);
    expect(got).toEqual(want);
    frames++;
  }
  return frames;
}

describe("ChatArea streaming render", () => {
  test("deferred: deltas are not rendered until flush", () => {
    const chat = new ChatArea(W);
    chat.render(W);
    chat.addMessage({ id: "m", role: "assistant", content: "" });
    const before = chat.renderAll(W);
    chat.appendToLastMessage("hello **world**");
    // Content mutated, but rendered lines stay stale until the next render.
    expect(chat.renderAll(W)).not.toEqual(before);
    expect(chat.renderAll(W)).toEqual(eagerLines("hello **world**", true));
  });

  test("incremental prefix cache matches full render at every frame", () => {
    const full = [
      "# Header\n\n",
      "para **one** with `code` and more text wrapping around here. ".repeat(8) + "\n\n",
      "```ts\nconst x = 1;\n// comment\n\nblank line inside fence\n```\n\n",
      "- item one\n- item two\n- item three\n\n",
      "| a | b |\n|---|---|\n| 1 | 2 |\n\n",
      "> a quote\n\n",
      "1. first\n2. second\n\n",
      "unclosed fence follows\n\n```python\nprint(1)\n",
    ].join("");
    const frames = expectStreamingMatchesFullRender(full);
    expect(frames).toBeGreaterThan(50);
  });

  test("link reference definitions disable prefix caching safely", () => {
    // Definition arrives AFTER the use — would retroactively change the
    // prefix render, so the cache must be bypassed for this message.
    const full = "see [the docs] for details\n\nmore text\n\n[the docs]: https://example.com\n";
    expectStreamingMatchesFullRender(full);
  });

  test("thinking deltas stream incrementally too", () => {
    const chat = new ChatArea(W);
    chat.render(W);
    chat.addMessage({ id: "m", role: "assistant", content: "", thinking: "" });
    const thinking = "reasoning **step** one\n\nreasoning step two\n\n";
    for (const chunk of thinking.match(/.{1,5}/gs) ?? []) {
      chat.appendThinkingDelta(chunk);
      chat.renderAll(W); // must not throw; output checked at completion
    }
    (chat as any).messages[0].pending = false;
    (chat as any).rerender();
    const got = chat.renderAll(W);

    const ref = new ChatArea(W);
    ref.render(W);
    setMessages(ref, [{ id: "m", role: "assistant", content: "", thinking }]);
    expect(got).toEqual(ref.renderAll(W));
  });

  test("width change mid-stream invalidates cached prefix", () => {
    const chat = new ChatArea(W);
    chat.render(W);
    chat.addMessage({ id: "m", role: "assistant", content: "" });
    const first = "para one wraps differently at another width. ".repeat(6) + "\n\n";
    chat.appendToLastMessage(first);
    chat.renderAll(W);

    const rest = "para two keeps streaming along here. ".repeat(6);
    chat.appendToLastMessage(rest);
    const narrow = chat.renderAll(60);
    expect(narrow).toEqual(eagerLines(first + rest, true, 60));
  });

  test("completion switches to a clean full render", () => {
    const chat = new ChatArea(W);
    chat.render(W);
    chat.addMessage({ id: "m", role: "assistant", content: "" });
    const full = "# Title\n\nbody **bold**\n\n- a\n- b\n";
    for (const chunk of full.match(/.{1,4}/gs) ?? []) {
      chat.appendToLastMessage(chunk);
      chat.renderAll(W);
    }
    chat.updateLastMessage(full);
    (chat as any).messages[0].pending = false;
    (chat as any).rerender();
    expect(chat.renderAll(W)).toEqual(eagerLines(full, false));
  });
});
