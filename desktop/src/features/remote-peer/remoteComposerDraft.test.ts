// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";
import {
  clearRemoteComposerDraft,
  loadRemoteComposerDraft,
  saveRemoteComposerDraft,
} from "./remoteComposerDraft";

/**
 * The unsent input a remote conversation carries between visits.
 *
 * Two rules make this safe to keep across restarts: a draft belongs to one
 * host+conversation pair, and a draft this build cannot read is *discarded*
 * rather than migrated — a half-understood draft would put words back in the
 * composer that the user never typed in this version's shape.
 */

const KEY = "remote-composer-draft:desktop_a:sess_1";

beforeEach(() => {
  localStorage.clear();
});

describe("remoteComposerDraft", () => {
  it("keeps a draft per host and conversation", () => {
    saveRemoteComposerDraft("desktop_a", "sess_1", { text: "half a thought", uploads: [] });
    saveRemoteComposerDraft("desktop_b", "sess_1", { text: "the other machine's", uploads: [] });

    expect(loadRemoteComposerDraft("desktop_a", "sess_1")?.text).toBe("half a thought");
    expect(loadRemoteComposerDraft("desktop_b", "sess_1")?.text).toBe("the other machine's");
    // A conversation with no draft is not a conversation with an empty one.
    expect(loadRemoteComposerDraft("desktop_a", "sess_2")).toBeNull();

    clearRemoteComposerDraft("desktop_a", "sess_1");
    expect(loadRemoteComposerDraft("desktop_a", "sess_1")).toBeNull();
    expect(loadRemoteComposerDraft("desktop_b", "sess_1")?.text).toBe("the other machine's");
  });

  it("keeps the conversations a draft's `#` tokens point at", () => {
    // The token alone is display text: without the map a reopened draft would
    // send plain words where a link was meant.
    saveRemoteComposerDraft("desktop_a", "sess_1", {
      text: "see #Plan",
      uploads: [],
      references: { "#Plan": { sessionId: "sess_2", title: "Plan" } },
    });

    expect(loadRemoteComposerDraft("desktop_a", "sess_1")?.references)
      .toEqual({ "#Plan": { sessionId: "sess_2", title: "Plan" } });
  });

  it("discards a draft written by another version instead of guessing at it", () => {
    // The one field that makes a draft readable is its version: a shape this
    // build did not write is not one it may put back in the editor.
    localStorage.setItem(KEY, JSON.stringify({ text: "from the future", uploads: [], version: 99 }));

    expect(loadRemoteComposerDraft("desktop_a", "sess_1")).toBeNull();
  });

  it("discards a draft the tab died while writing", () => {
    // `localStorage` writes are not atomic across a crash, so a truncated value
    // is a real state — and it must read as "no draft" rather than throwing into
    // the composer's render.
    localStorage.setItem(KEY, "{\"text\":\"half a thou");

    expect(loadRemoteComposerDraft("desktop_a", "sess_1")).toBeNull();
  });
  it("keeps an answer on its own, and clears the slot only when it is gone too", () => {
    // A draft that is only an answer is exactly what a reader has after picking
    // an option and typing nothing. Treating it as empty would drop the pick on
    // the next visit while the chip was still what made Send live (design §5.4).
    const answer = {
      mode: "single" as const,
      options: [{ id: "forest", label: "重画森林图" }],
      selectedIds: ["forest"],
      title: "接下来做哪一件事？",
    };
    saveRemoteComposerDraft("desktop_a", "sess_1", { choice: answer });

    expect(loadRemoteComposerDraft("desktop_a", "sess_1")?.choice).toEqual(answer);

    saveRemoteComposerDraft("desktop_a", "sess_1", { text: "", uploads: [], choice: undefined });

    expect(loadRemoteComposerDraft("desktop_a", "sess_1")).toBeNull();
  });

  it("reads a draft written before the answer field existed", () => {
    // Exactly what this build wrote for a draft with words and one upload, back
    // when `choice` did not exist: `version` plus the two fields that were there.
    // It has to load, because the version is the only thing that makes a stored
    // draft unreadable and adding an optional field must not bump it — the pick a
    // reader had open across the upgrade is not worth discarding over a field
    // that is simply absent.
    const upload = {
      contentHash: "abc",
      id: "u1",
      kind: "image",
      name: "fig.png",
      path: "/tmp/fig.png",
      size: 12,
      uploadId: "upload-1",
    };
    saveRemoteComposerDraft("desktop_a", "sess_1", {
      text: "顺便把图注也改成中文",
      uploads: [upload],
    });
    // Strip the answer field to get the exact bytes this build wrote before it
    // existed — same version, one fewer key.
    const stored = JSON.parse(localStorage.getItem(KEY)!) as Record<string, unknown>;
    delete stored.choice;
    localStorage.setItem(KEY, JSON.stringify(stored));

    const draft = loadRemoteComposerDraft("desktop_a", "sess_1");

    expect(draft?.text).toBe("顺便把图注也改成中文");
    expect(draft?.uploads).toHaveLength(1);
    // Absent, not invented: an empty answer would draw a chip for a card this
    // draft never answered.
    expect(draft?.choice).toBeUndefined();
  });

  it("keeps reading an old draft after this build saves it again", () => {
    // The upgrade path in one go: a pre-`choice` draft, edited and saved by this
    // build without picking an answer, must stay loadable and stay answerless.
    saveRemoteComposerDraft("desktop_a", "sess_1", { text: "old words" });
    const stored = JSON.parse(localStorage.getItem(KEY)!) as Record<string, unknown>;
    delete stored.choice;
    localStorage.setItem(KEY, JSON.stringify(stored));
    const loaded = loadRemoteComposerDraft("desktop_a", "sess_1")!;

    saveRemoteComposerDraft("desktop_a", "sess_1", { ...loaded, text: "old words, edited" });

    const again = loadRemoteComposerDraft("desktop_a", "sess_1");
    expect(again?.text).toBe("old words, edited");
    expect(again?.choice).toBeUndefined();
  });
});
