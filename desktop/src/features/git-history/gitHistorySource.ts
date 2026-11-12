/**
 * One conversation's Git history, whichever machine holds it.
 *
 * A conversation shows the history of the workspace it runs in. That workspace
 * is on this machine for a local thread and on a paired desktop for a remote
 * one, so the *source* differs but the picture must not: both answers are the
 * same shape, from the same Rust module (`future_host::git_history`), decoded by
 * the same `@future-os/git-graph`. Callers therefore ask a `GitHistorySource`
 * rather than a Tauri command, and the view never learns which machine answered.
 */

import type { GitCommitDetail, GitGraphListing } from "./types";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { getRemoteGitCommit, listRemoteGitCommits } from "../remote-peer/remotePeerClient";

/** Where a history is read from. */
export type GitHistorySource
  /** The workspace on this machine. */
  = | { kind: "local"; path: string }
  /** A conversation on a paired desktop, addressed the way its other reads are. */
    | { kind: "remote"; desktopId: string; sessionId: string };

/** Whether a source can be read at all, so a view can say why it is empty. */
export function gitSourceReady(source: GitHistorySource | null): boolean {
  if (!source) {
    return false;
  }
  return source.kind === "local" ? source.path.length > 0 : true;
}

export async function listGitHistory(source: GitHistorySource): Promise<GitGraphListing> {
  if (source.kind === "local") {
    return invokeCommand<GitGraphListing>("get_git_history", { path: source.path });
  }
  return listRemoteGitCommits(source.desktopId, source.sessionId);
}

export async function getGitCommit(
  source: GitHistorySource,
  sha: string,
): Promise<GitCommitDetail> {
  if (source.kind === "local") {
    return invokeCommand<GitCommitDetail>("get_git_commit", { path: source.path, sha });
  }
  return getRemoteGitCommit(source.desktopId, source.sessionId, sha);
}

/**
 * A stable key for a source, for effect dependencies and cache identity.
 *
 * Never `JSON.stringify`: the property order of two literals for the same source
 * is not something a caller should have to keep, and a key that changed with the
 * spelling would re-read the history on every render.
 */
export function gitSourceKey(source: GitHistorySource | null): string {
  if (!source) {
    return "";
  }
  if (source.kind === "local") {
    return `local:${source.path}`;
  }
  return `remote:${source.desktopId}:${source.sessionId}`;
}
