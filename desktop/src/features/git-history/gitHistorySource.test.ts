import { beforeEach, expect, it, vi } from "vitest";
import { getGitCommit, gitSourceKey, gitSourceReady, listGitHistory } from "./gitHistorySource";

/**
 * The one seam between a conversation's workspace and either machine.
 *
 * A local thread's workspace is on this machine; a remote thread's is on a
 * paired desktop. Both answer with the same shape from the same Rust module, so
 * this module's whole job is to route the question and to give the answer a name
 * an effect can depend on.
 */

const invoke = vi.fn<(command: string, args: Record<string, unknown>) => Promise<unknown>>();
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (...args: Parameters<typeof invoke>) => invoke(...args),
}));

const listRemote = vi.fn<(desktopId: string, sessionId: string) => Promise<unknown>>();
const getRemote = vi.fn<(desktopId: string, sessionId: string, sha: string) => Promise<unknown>>();
vi.mock("../remote-peer/remotePeerClient", () => ({
  listRemoteGitCommits: (...args: Parameters<typeof listRemote>) => listRemote(...args),
  getRemoteGitCommit: (...args: Parameters<typeof getRemote>) => getRemote(...args),
}));

const listing = { commits: [], isRepo: true, path: "/w", repoName: "w", truncated: false };
const detail = { patch: "", short: "abc123", truncated: false };

beforeEach(() => {
  invoke.mockReset().mockResolvedValue(listing);
  listRemote.mockReset().mockResolvedValue(listing);
  getRemote.mockReset().mockResolvedValue(detail);
});

it("asks this machine for a local workspace", async () => {
  await listGitHistory({ kind: "local", path: "/Users/reader/repo" });
  expect(invoke).toHaveBeenCalledWith("get_git_history", { path: "/Users/reader/repo" });
  expect(listRemote).not.toHaveBeenCalled();
});

it("asks the peer for a remote conversation", async () => {
  await listGitHistory({ desktopId: "desktop_a", kind: "remote", sessionId: "sess_1" });
  expect(listRemote).toHaveBeenCalledWith("desktop_a", "sess_1");
  expect(invoke).not.toHaveBeenCalled();
});

it("reads a local commit's patch from this machine", async () => {
  await getGitCommit({ kind: "local", path: "/Users/reader/repo" }, "abc123");
  expect(invoke).toHaveBeenCalledWith("get_git_commit", { path: "/Users/reader/repo", sha: "abc123" });
  expect(getRemote).not.toHaveBeenCalled();
});

it("reads a remote commit's patch from the peer", async () => {
  await getGitCommit({ desktopId: "desktop_a", kind: "remote", sessionId: "sess_1" }, "abc123");
  expect(getRemote).toHaveBeenCalledWith("desktop_a", "sess_1", "abc123");
  expect(invoke).not.toHaveBeenCalled();
});

it("gives every source a stable, distinct key", () => {
  const repo = { kind: "local", path: "/Users/reader/repo" } as const;
  // The same source spelled twice is the same key, which is what stops an
  // effect from re-reading the history on every render.
  expect(gitSourceKey({ ...repo })).toBe(gitSourceKey(repo));
  expect(gitSourceKey(repo)).not.toBe(gitSourceKey({ kind: "local", path: "/Users/reader/other" }));
  expect(gitSourceKey({ desktopId: "d", kind: "remote", sessionId: "s" }))
    .not
    .toBe(gitSourceKey({ desktopId: "d", kind: "remote", sessionId: "other" }));
  expect(gitSourceKey({ desktopId: "d", kind: "remote", sessionId: "s" })).not.toBe(gitSourceKey(repo));
  expect(gitSourceKey(null)).toBe("");
});

it("refuses to read a local workspace with no path, but allows a remote one", () => {
  expect(gitSourceReady(null)).toBe(false);
  expect(gitSourceReady({ kind: "local", path: "" })).toBe(false);
  expect(gitSourceReady({ kind: "local", path: "/w" })).toBe(true);
  // The peer read carries its own address; there is no path to be missing.
  expect(gitSourceReady({ desktopId: "d", kind: "remote", sessionId: "s" })).toBe(true);
});
