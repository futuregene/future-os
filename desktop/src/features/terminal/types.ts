/**
 * Wire types for the embedded terminal.
 *
 * These mirror `desktop/src-tauri/src/terminal/*` exactly. Terminal *output*
 * never appears here: it travels over the WebSocket, not through these types.
 */

/** What the client knows about a shell. Mirrors Rust `session::Info`. */
export interface TerminalInfo {
  id: string;
  threadId: string;
  title: string;
  command: string;
  args: string[];
  cwd: string;
  status: "running" | "exited";
  exitCode: number | null;
  pid: number | null;
  cols: number;
  rows: number;
}

/** One entry of `GET /terminal/shells`. */
export interface TerminalShell {
  path: string;
  name: string;
  acceptable: boolean;
}

/** The loopback endpoint the webview talks to. */
export interface TerminalServerHandle {
  url: string;
  token: string;
  port: number;
  maxSessions: number;
}

/** Control-frame payload sent after a replay (`0x00` + JSON on the wire). */
export interface TerminalFrameMeta {
  /** Absolute output cursor the client must store and resume from. */
  cursor: number;
  /**
   * Absolute offset of the first replayed byte. `start > requested` means the
   * retained buffer no longer covers what the client asked for.
   */
  start: number;
  /** Present on the final frame, after the shell exited. */
  exitCode?: number;
}

export interface CreateTerminalInput {
  threadId: string;
  title?: string;
  cols?: number;
  rows?: number;
}

export interface UpdateTerminalInput {
  title?: string;
  cols?: number;
  rows?: number;
}
