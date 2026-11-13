/**
 * The history the host answers with.
 *
 * These mirror `future_host::git_history::{GraphListing, GraphRow, CommitDetail}`
 * field for field. They are written out rather than imported from Rust because
 * the same shapes arrive from two paths — a local Tauri command and a remote
 * peer reply — and a shared hand-written type is what keeps those two answers
 * interchangeable at the view.
 */

export interface GitGraphRow {
  /** The abbreviated id: the row's key, and what opening the patch asks for. */
  short: string;
  when: string;
  who: string;
  /** Ref names on this commit (`HEAD -> main, origin/main, tag: v1`), or "". */
  decorations: string;
  subject: string;
  /**
   * The row's gutter, as box-drawing characters. A *description* of the picture,
   * not the picture: `@future-os/git-graph` turns it into strokes, and nothing
   * renders these characters.
   */
  graph: string;
}

export interface GitGraphListing {
  /**
   * Whether the conversation's workspace is inside a Git repository at all.
   * Answered by the same read the list uses, so a plain directory costs no extra
   * round trip to hide the feature for.
   */
  isRepo: boolean;
  /** The repository's directory name; "" when the workspace is not one. */
  repoName: string;
  /**
   * The workspace the history was read in — a workspace can hold several
   * checkouts, so the path is worth showing.
   */
  path: string;
  commits: GitGraphRow[];
  /** The log was cut at the host's page size; the oldest row is not the root. */
  truncated: boolean;
}

export interface GitCommitDetail {
  short: string;
  /** The patch *with* the stat, as `git show --stat --patch` prints it. */
  patch: string;
  truncated: boolean;
}
