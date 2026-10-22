// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  defaultNewConversationTarget,
  readNewConversationTarget,
  writeNewConversationTarget,
} from "./newConversationTarget";

const STORAGE_KEY = "future.newConversationTarget";

function store(raw: unknown) {
  localStorage.setItem(STORAGE_KEY, typeof raw === "string" ? raw : JSON.stringify(raw));
}

describe("newConversationTarget", () => {
  beforeEach(() => localStorage.clear());

  it("round-trips a workspace target and a plain chat target", () => {
    writeNewConversationTarget({ mode: "workspace", workspaceId: "w-1" });
    expect(readNewConversationTarget()).toEqual({ mode: "workspace", workspaceId: "w-1" });

    // A plain chat is a choice too: it has to overwrite the remembered
    // workspace, or the user could never get back to chatting.
    writeNewConversationTarget({ mode: "chat", workspaceId: null });
    expect(readNewConversationTarget()).toEqual({ mode: "chat", workspaceId: null });
  });

  it("never stores a workspace mode without the workspace it means", () => {
    writeNewConversationTarget({ mode: "workspace", workspaceId: null });
    expect(readNewConversationTarget()).toEqual({ mode: "chat", workspaceId: null });
  });

  it("treats a value it cannot trust as nothing remembered", () => {
    store("{not json");
    expect(readNewConversationTarget()).toBeNull();

    // Unknown modes, missing ids and non-objects are all stale or foreign
    // writes; the screen then opens on its old default instead of guessing.
    store({ mode: "drawing", workspaceId: "w-1" });
    expect(readNewConversationTarget()).toBeNull();

    store({ mode: "workspace" });
    expect(readNewConversationTarget()).toBeNull();

    store({ mode: "workspace", workspaceId: "" });
    expect(readNewConversationTarget()).toBeNull();

    store([]);
    expect(readNewConversationTarget()).toBeNull();
  });

  it("reads and writes without a usable storage", () => {
    vi.spyOn(localStorage, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    expect(readNewConversationTarget()).toBeNull();

    vi.spyOn(localStorage, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    expect(() => writeNewConversationTarget({ mode: "chat", workspaceId: null })).not.toThrow();
  });

  it("opens on the remembered workspace only while it still exists", () => {
    const stored = { mode: "workspace", workspaceId: "w-1" } as const;
    expect(defaultNewConversationTarget(stored, id => id === "w-1")).toEqual(stored);

    // Deleted, or the store has not loaded it: a composer bound to a folder
    // that is gone is not what the user picked last time.
    expect(defaultNewConversationTarget(stored, () => false)).toEqual({ mode: "chat", workspaceId: null });
    expect(defaultNewConversationTarget({ mode: "chat", workspaceId: null }, () => true))
      .toEqual({ mode: "chat", workspaceId: null });
    expect(defaultNewConversationTarget(null, () => true)).toEqual({ mode: "chat", workspaceId: null });
  });
});
