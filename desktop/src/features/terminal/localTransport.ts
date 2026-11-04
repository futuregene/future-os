import type { TerminalLink, TerminalTransport, TerminalTransportHandlers } from "./transport";
import type { TerminalFrameMeta } from "./types";
import { connectTicket, connectUrl, terminalServer, updateTerminal } from "./client";
import { AttachAborted, CLOSE_EXIT } from "./transport";

/**
 * The loopback transport: the app's own webview talking to the terminal server
 * in this same process (see `docs/internals/desktop/embedded-terminal.md`).
 *
 * One HTTP call for a single-use ticket, then one WebSocket carrying raw bytes
 * in both directions. That is why every local tab is cheap: no polling, no
 * base64, no relay.
 *
 * Frame layout (the server's own, `terminal/protocol.rs`): a type byte then its
 * payload — `0x00` + UTF-8 JSON control, `0x01` + PTY bytes verbatim. The type
 * byte decides, never the payload: output legitimately starts with NUL.
 */

/** The server's frame types. */
const FRAME_CONTROL = 0;
const FRAME_DATA = 1;

export const localTransport: TerminalTransport = {
  async open(id, cursor, handlers, signal): Promise<TerminalLink> {
    const ticket = await connectTicket(id);
    if (signal.aborted)
      throw new AttachAborted();
    const server = await terminalServer();
    if (signal.aborted)
      throw new AttachAborted();
    const socket = new WebSocket(connectUrl(server, id, cursor, ticket));
    socket.binaryType = "arraybuffer";
    attach(socket, handlers);
    return {
      write(data) {
        if (socket.readyState === WebSocket.OPEN) {
          socket.send(data);
        }
      },
      close() {
        if (socket.readyState !== WebSocket.CLOSED && socket.readyState !== WebSocket.CLOSING) {
          socket.close(CLOSE_EXIT);
        }
      },
    };
  },

  async resize(id, cols, rows) {
    await updateTerminal(id, { cols, rows });
  },
};

function attach(socket: WebSocket, handlers: TerminalTransportHandlers): void {
  // These listeners belong to one attachment's socket, which the view closes on
  // unmount (the same rule the view documented when it owned the socket).

  socket.addEventListener("open", () => handlers.onOpen());

  socket.addEventListener("message", (event: MessageEvent) => {
    if (typeof event.data === "string") {
      handlers.onText(event.data);
      return;
    }
    const bytes = new Uint8Array(event.data as ArrayBuffer);
    if (bytes.length > 0 && bytes[0] === FRAME_CONTROL) {
      handlers.onMeta(decodeMeta(bytes.subarray(1)));
      return;
    }
    if (bytes.length > 0 && bytes[0] !== FRAME_DATA) {
      // A frame kind this client does not know: dropping it is the only reading
      // that cannot type its header into the user's shell.
      return;
    }
    handlers.onBytes(bytes.subarray(1));
  });

  socket.addEventListener("close", (event: CloseEvent) => handlers.onClose(event.code));
}

/**
 * Decode a control frame.
 *
 * A decoder of its own, deliberately: the streaming decoder that decodes PTY
 * output holds the bytes of a character split across two frames, and a
 * non-streaming `decode()` on the same instance flushes those bytes out as
 * U+FFFD — which eats the end of a replay (with it the control frame reporting
 * the cursor) whenever the replay chunk boundary falls inside a multi-byte
 * character. Control payloads are ASCII JSON.
 */
function decodeMeta(payload: Uint8Array): TerminalFrameMeta | undefined {
  try {
    return JSON.parse(new TextDecoder().decode(payload)) as TerminalFrameMeta;
  }
  catch {
    return undefined;
  }
}
