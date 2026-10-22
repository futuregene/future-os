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
});
