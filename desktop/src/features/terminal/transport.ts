import type { TerminalFrameMeta } from "./types";

/**
 * How a mounted terminal reaches its shell.
 *
 * Two transports exist and they are not interchangeable in cost, only in
 * contract: the loopback WebSocket the local webview uses (bytes on one
 * socket), and the paired desktop's control-plane poll (a round trip per page).
 * Everything above this seam — cursor accounting, replay truncation notices,
 * exit handling, the retry policy — is the view's and is written once.
 *
 * Handlers receive *parsed* frames. Which side of the seam parses them depends
 * on what the wire carries: the loopback socket frames bytes with a type byte,
 * a control-plane page arrives as base64 plus the cursor fields it belongs to.
 * The view must not care.
 */

/** Frames a transport delivers, in the order the shell produced them. */
export interface TerminalTransportHandlers {
  /** Attached and streaming: the shell is reachable. */
  onOpen: () => void;
  /**
   * Output bytes, already stripped of framing. A transport must never reorder
   * them against each other or against the control frame that follows.
   */
  onBytes: (bytes: Uint8Array) => void;
  /**
   * The attachment's control frame — where this attachment started and the
   * absolute cursor after everything delivered so far. `undefined` when a
   * control payload could not be read: the attachment is still over.
   */
  onMeta: (meta: TerminalFrameMeta | undefined) => void;
  /**
   * A payload the transport could not classify, rendered verbatim rather than
   * dropped (an unexpected text frame is noise, not an attack).
   */
  onText: (text: string) => void;
  /** The attachment ended. `1000` exit, `4408` lagged, anything else retries. */
  onClose: (code: number) => void;
  /**
   * The shell is gone from the host (evicted, or the conversation deleted
   * there). The tab keeps its screen and offers a restart — the same outcome
   * the local transport reaches by rejecting the attach.
   */
  onMissing: () => void;
}

/** One attachment. Closed on unmount, on a tab switch, or by the transport. */
export interface TerminalLink {
  /** Keyboard input as xterm produced it (a string of UTF-16). */
  write: (data: string) => void;
  close: () => void;
}

export interface TerminalTransport {
  /**
   * Attach at `cursor` (`-1` tails from the current end). Rejects with a
   * `TerminalApiError` carrying `TERMINAL_NOT_FOUND` when the shell is gone —
   * the view turns that into "this tab needs a restart" instead of a retry.
   *
   * `signal` aborts an attach that has not finished: the panel closed, the tab
   * switched or the webview went away while the endpoint or ticket was still in
   * flight, and opening the connection anyway would leave a spurious attachment
   * (and, for the loopback server, a live viewer holding a buffer). A transport
   * that has already produced a link responds by closing it.
   */
  open: (
    id: string,
    cursor: number,
    handlers: TerminalTransportHandlers,
    signal: AbortSignal,
  ) => Promise<TerminalLink>;
  /** Tell the shell its geometry changed. Best effort. */
  resize: (id: string, cols: number, rows: number) => Promise<void>;
}

/** The shell exited on its own; the final control frame carried the code. */
export const CLOSE_EXIT = 1000;
/** The viewer fell too far behind and must re-attach from its own cursor. */
export const CLOSE_LAGGED = 4408;

/**
 * Thrown by an attach whose view went away before it finished. Not an error to
 * show: the view is gone, and nobody is waiting for the answer.
 */
export class AttachAborted extends Error {
  constructor() {
    super("attach_aborted");
    this.name = "AttachAborted";
  }
}
