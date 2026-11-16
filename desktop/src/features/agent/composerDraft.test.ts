import type { ChoiceAnswer } from "./composer/choiceAnswer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearComposerDraft, clearComposerDraftIfMatches, loadComposerDraft, saveComposerDraft } from "./composerDraft";

/** One answer, in the shape the editor's pill and the card both read. */
const ANSWER: ChoiceAnswer = {
  mode: "single",
  options: [
    { id: "forest", label: "重画森林图" },
    { id: "flow", label: "补一张实验流程图" },
  ],
  selectedIds: ["forest"],
  title: "接下来做哪一件？",
};

// Minimal in-memory sessionStorage: the default vitest env is "node", which has
// no Web Storage, so the module under test needs one supplied.
function installSessionStorage() {
  const map = new Map<string, string>();
  const storage: Pick<Storage, "getItem" | "setItem" | "removeItem"> = {
    getItem: key => (map.has(key) ? map.get(key)! : null),
    setItem: (key, value) => void map.set(key, String(value)),
    removeItem: key => void map.delete(key),
  };
  vi.stubGlobal("sessionStorage", storage);
  return map;
}

describe("composerDraft", () => {
  let store: Map<string, string>;

  beforeEach(() => {
    store = installSessionStorage();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("round-trips text and attachments scoped by conversation key", () => {
    saveComposerDraft("thread-a", {
      text: "hello [a.ts](./a.ts)",
      attachments: [{ name: "a.ts", path: "/a.ts" }],
    });

    const draft = loadComposerDraft("thread-a");
    expect(draft?.text).toBe("hello [a.ts](./a.ts)");
    expect(draft?.attachments).toEqual([{ name: "a.ts", path: "/a.ts" }]);
    // A different conversation shares nothing.
    expect(loadComposerDraft("thread-b")).toBeNull();
  });

  it("clears the slot instead of persisting a blank draft", () => {
    saveComposerDraft("t", { text: "seed", attachments: [] });
    expect(loadComposerDraft("t")).not.toBeNull();

    // Whitespace-only text with no attachments removes the entry.
    saveComposerDraft("t", { text: "   \n  ", attachments: [] });
    expect(loadComposerDraft("t")).toBeNull();
    expect(store.has("composer-draft:t")).toBe(false);
  });

  it("keeps a draft that has only a mention (mention markdown counts as content)", () => {
    saveComposerDraft("t", { text: "[a.ts](./a.ts)", attachments: [] });
    expect(loadComposerDraft("t")?.text).toBe("[a.ts](./a.ts)");
  });

  it("keeps a draft that has attachments but no text", () => {
    saveComposerDraft("t", { text: "", attachments: [{ name: "img.png", path: "/img.png" }] });
    expect(loadComposerDraft("t")?.attachments).toHaveLength(1);
  });

  it("returns null for corrupt JSON and version mismatches", () => {
    store.set("composer-draft:bad", "not json {{{");
    expect(loadComposerDraft("bad")).toBeNull();

    store.set("composer-draft:stale", JSON.stringify({ version: 999, text: "x" }));
    expect(loadComposerDraft("stale")).toBeNull();
  });

  it("clearComposerDraft removes a stored draft", () => {
    saveComposerDraft("t", { text: "text", attachments: [] });
    clearComposerDraft("t");
    expect(loadComposerDraft("t")).toBeNull();
  });

  it("treats an absent text or attachment list as empty", () => {
    // boundary: `draft.attachments?.length ?? 0` and `(draft.text ?? "")` are both
    // defensive against a partial call - a composer can persist a draft that has
    // only pills, or only attachments. Neither may throw, and a draft with nothing
    // at all in any field still counts as empty.
    saveComposerDraft("partial", { attachments: [{ name: "a.png", path: "/a.png" }] } as never);
    expect(loadComposerDraft("partial")?.attachments).toHaveLength(1);

    clearComposerDraft("absent");
    saveComposerDraft("absent", {} as never);
    expect(loadComposerDraft("absent")).toBeNull();
  });

  describe("the choice card's answer", () => {
    it("rides in the draft beside the text and survives a round trip", () => {
      saveComposerDraft("t", { attachments: [], choice: ANSWER, text: "顺便把图例挪到右下" });

      const draft = loadComposerDraft("t");
      expect(draft?.choice).toEqual(ANSWER);
      expect(draft?.text).toBe("顺便把图例挪到右下");
    });

    it("keeps the slot when the answer is the only thing in the draft", () => {
      // An answer is content: a draft the reader is halfway through must not be
      // thrown away because the box behind the pill happens to be empty.
      saveComposerDraft("t", { attachments: [], choice: ANSWER, text: "" });
      expect(loadComposerDraft("t")?.choice?.selectedIds).toEqual(["forest"]);
    });

    it("clears the slot once the answer is gone too", () => {
      saveComposerDraft("t", { attachments: [], choice: ANSWER, text: "" });
      saveComposerDraft("t", { attachments: [], choice: undefined, text: "" });
      expect(loadComposerDraft("t")).toBeNull();
    });

    it("still reads the version-1 drafts written before answers existed", () => {
      // The shape deliberately grows without a version bump: a stored draft from
      // before this field existed carries no `choice` and must load unchanged —
      // bumping DRAFT_VERSION here would have discarded every open draft.
      store.set("composer-draft:legacy", JSON.stringify({ version: 1, text: "旧草稿", attachments: [] }));
      expect(loadComposerDraft("legacy")?.text).toBe("旧草稿");
      expect(loadComposerDraft("legacy")?.choice).toBeUndefined();

      store.set("composer-draft:legacy-answer", JSON.stringify({ version: 1, text: "", choice: ANSWER }));
      expect(loadComposerDraft("legacy-answer")?.choice).toEqual(ANSWER);
    });
  });

  describe("clearComposerDraftIfMatches", () => {
    it("drops a restored rejected message once it has been accepted again", () => {
      saveComposerDraft("t", { text: "hello", attachments: [{ name: "a.png", path: "/a.png" }] });
      clearComposerDraftIfMatches("t", "hello", [{ name: "a.png", path: "/a.png" }]);
      expect(loadComposerDraft("t")).toBeNull();
    });

    it("keeps a draft the user has edited since the rejection", () => {
      saveComposerDraft("t", { text: "hello, edited", attachments: [] });
      clearComposerDraftIfMatches("t", "hello", []);
      expect(loadComposerDraft("t")?.text).toBe("hello, edited");
    });

    it("keeps a draft carrying different attachments", () => {
      saveComposerDraft("t", { text: "hello", attachments: [{ name: "b.png", path: "/b.png" }] });
      clearComposerDraftIfMatches("t", "hello", []);
      expect(loadComposerDraft("t")?.attachments).toHaveLength(1);
    });

    it("ignores a missing draft key and a missing draft", () => {
      expect(() => clearComposerDraftIfMatches(undefined, "hello")).not.toThrow();
      expect(() => clearComposerDraftIfMatches("missing", "hello")).not.toThrow();
    });
  });
});
