import type { TerminalLink, TerminalTransport } from "./transport";
import type { CreateTerminalInput, TerminalInfo } from "./types";
import { requestRemotePeer } from "../remote-peer/remotePeerClient";
import { TerminalApiError } from "./client";
import { AttachAborted, CLOSE_EXIT } from "./transport";

/**
 * The embedded terminal as a *paired* desktop drives it.
 *
 * The host owns the shells — the same ones the person in front of that machine
 * sees — and this module is the client half of `host/src/remote_host/business/
 * terminal.rs`. Everything goes through the ordinary control-plane command lane,
 * which is why a desktop that has never heard of this module is unaffected: an
 * older host answers `Unsupported command`, and the capability it advertises
 * (`terminal_v1`) is what tells this client not to offer the panel at all.
 *
 * ## The one thing that is not free: response time
 *
 * There is no stream here. Output is *pulled* — `terminal_read` returns the
 * bytes after the cursor the view already applied — and input is *awaited*: one
 * `terminal_write` is in flight at a time, because commands are dispatched
 * concurrently on the host (a slow one must not block the others) and keystroke
 * order is a correctness requirement no batching may trade away. So a keystroke
 * reaches the shell one round trip later, and its echo one more. That is a
 * "watch the build" terminal, not a `vim` terminal, and the panel says nothing
 * it cannot do: see `docs/internals/desktop/embedded-terminal.md` §Remote.
 */

/** How long an idle shell waits before the next read. */
const IDLE_POLL_MS = 250;
/**
 * How soon a shell with output to give is read again. Not zero: the poll clock
 * is what bounds a chatty command's request rate, and 60 ms is under the
 * frame time of anything a human reads.
 */
const BUSY_POLL_MS = 16;
/**
 * Where output stops being worth a round trip per page and starts being worth a
 * pause. A command like `yes` produces far more than the wire can carry; the
 * view shows what it can and the rest is behind its cursor, exactly as a lagged
 * loopback viewer is.
 */
const MAX_PAGES_PER_WAKE = 4;
/** Consecutive failed reads before the attachment gives up and re-attaches. */
const READ_FAILURES = 3;

/** A read page, as `terminal_read` answers it. */
export interface RemoteTerminalPage {
  bytes: Uint8Array;
  cursor: number;
  start: number;
  status: "running" | "exited";
  exitCode: number | null;
}

function decodeBase64(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1)
    bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function encodeBase64(bytes: Uint8Array): string {
  let binary = "";
  // Chunked so a large paste cannot blow the argument limit of `String.fromCharCode`.
  for (let index = 0; index < bytes.length; index += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  return btoa(binary);
}

/**
 * The host's refusal, in the shape the local transport's failures have.
 *
 * A control-plane error arrives as one string (`"TERMINAL_NOT_FOUND: …"`), and
 * the code in front of the colon is what the UI branches on — the same codes
 * the loopback routes answer with, because both transports drive one registry.
 */
export function remoteTerminalError(cause: unknown): TerminalApiError {
  if (cause instanceof TerminalApiError)
    return cause;
  const message = cause instanceof Error ? cause.message : String(cause);
  // Linear on purpose: the message is a host's error text, and a backtracking
  // regex over attacker-influenced length is the one thing to avoid here.
  const trimmed = message.trim();
  const colon = trimmed.indexOf(":");
  const code = colon > 0 && /^[A-Z][A-Z0-9_]+$/.test(trimmed.slice(0, colon))
    ? trimmed.slice(0, colon)
    : null;
  if (code === null)
    return new TerminalApiError("REQUEST_FAILED", message, 0);
  return new TerminalApiError(code, trimmed.slice(colon + 1).trim(), 0);
}

function info(raw: unknown): TerminalInfo {
  const row = (typeof raw === "object" && raw !== null ? raw : {}) as Record<string, unknown>;
  const text = (key: string): string => (typeof row[key] === "string" ? row[key] as string : "");
  const size = (key: string): number => (typeof row[key] === "number" ? row[key] as number : 0);
  return {
    id: text("id"),
    threadId: text("threadId"),
    title: text("title"),
    // The wire deliberately does not carry the shell's program or pid: a paired
    // desktop has no use for either, and the local types keep them for the
    // panel's own tooltip.
    command: "",
    args: [],
    cwd: text("cwd"),
    status: row.status === "exited" ? "exited" : "running",
    exitCode: typeof row.exitCode === "number" ? row.exitCode : null,
    pid: null,
    cols: size("cols"),
    rows: size("rows"),
  };
}

/** Every shell this conversation has on the host. */
export async function listRemoteTerminals(
  desktopId: string,
  sessionId: string,
): Promise<TerminalInfo[]> {
  try {
    const reply = await requestRemotePeer<{ terminals?: unknown[] }>(
      desktopId,
      { type: "terminal_list", sessionId },
      sessionId,
    );
    const terminals = Array.isArray(reply?.terminals) ? reply.terminals : [];
    return terminals.map(info).filter(terminal => terminal.id);
  }
  catch (cause) {
    throw remoteTerminalError(cause);
  }
}

export async function createRemoteTerminal(
  desktopId: string,
  sessionId: string,
  input: Omit<CreateTerminalInput, "threadId">,
): Promise<TerminalInfo> {
  try {
    const reply = await requestRemotePeer<{ terminal?: unknown }>(
      desktopId,
      {
        type: "terminal_create",
        sessionId,
        ...(input.title === undefined ? {} : { title: input.title }),
        ...(input.cols === undefined ? {} : { cols: input.cols }),
        ...(input.rows === undefined ? {} : { rows: input.rows }),
      },
      sessionId,
    );
    return info(reply?.terminal);
  }
  catch (cause) {
    throw remoteTerminalError(cause);
  }
}

export async function updateRemoteTerminal(
  desktopId: string,
  terminalId: string,
  patch: { title?: string; cols?: number; rows?: number },
): Promise<void> {
  try {
    await requestRemotePeer(
      desktopId,
      {
        type: "terminal_update",
        terminalId,
        ...(patch.title === undefined ? {} : { title: patch.title }),
        ...(patch.cols === undefined ? {} : { cols: patch.cols }),
        ...(patch.rows === undefined ? {} : { rows: patch.rows }),
      },
      "list",
    );
  }
  catch (cause) {
    throw remoteTerminalError(cause);
  }
}

export async function removeRemoteTerminal(
  desktopId: string,
  terminalId: string,
): Promise<void> {
  try {
    await requestRemotePeer(desktopId, { type: "terminal_delete", terminalId }, "list");
  }
  catch (cause) {
    throw remoteTerminalError(cause);
  }
}

async function readRemoteTerminal(
  desktopId: string,
  terminalId: string,
  cursor: number,
): Promise<RemoteTerminalPage> {
  try {
    const reply = await requestRemotePeer<Record<string, unknown>>(
      desktopId,
      { type: "terminal_read", terminalId, terminalCursor: cursor },
      "list",
    );
    return {
      bytes: typeof reply?.data === "string" ? decodeBase64(reply.data) : new Uint8Array(),
      cursor: typeof reply?.cursor === "number" ? reply.cursor : cursor,
      start: typeof reply?.start === "number" ? reply.start : cursor,
      status: reply?.status === "exited" ? "exited" : "running",
      exitCode: typeof reply?.exitCode === "number" ? reply.exitCode : null,
    };
  }
  catch (cause) {
    throw remoteTerminalError(cause);
  }
}

async function writeRemoteTerminal(
  desktopId: string,
  terminalId: string,
  bytes: Uint8Array,
): Promise<void> {
  try {
    await requestRemotePeer(
      desktopId,
      { type: "terminal_write", terminalId, terminalData: encodeBase64(bytes) },
      "list",
    );
  }
  catch (cause) {
    throw remoteTerminalError(cause);
  }
}

/** Input batched for at most this long before it costs a round trip. */
const INPUT_BATCH_MS = 10;

/**
 * The polling transport for one remote terminal.
 *
 * Its contract is the loopback one: bytes in order, then the control frame that
 * says where the client is, then either more bytes or the shell's exit. Only the
 * clock differs, and that is architecture, not preference — see the module docs.
 */
export function remoteTerminalTransport(desktopId: string): TerminalTransport {
  return {
    async open(terminalId, cursor, handlers, signal): Promise<TerminalLink> {
      let stopped = false;
      let seek = cursor;
      let wake: ReturnType<typeof setTimeout> | undefined;
      let flushTimer: ReturnType<typeof setTimeout> | undefined;
      let failures = 0;
      let burst = 0;
      // Input is serialized: the next batch goes out only after the previous one
      // was acknowledged, because the host dispatches commands concurrently and
      // nothing else keeps keystrokes in order.
      let pending = "";
      let writing = false;

      const clearWake = () => {
        if (wake !== undefined) {
          clearTimeout(wake);
          wake = undefined;
        }
      };

      const stop = (closeCode: number) => {
        if (stopped)
          return;
        stopped = true;
        clearWake();
        if (flushTimer !== undefined) {
          clearTimeout(flushTimer);
          flushTimer = undefined;
        }
        handlers.onClose(closeCode);
      };

      const deliver = (page: RemoteTerminalPage) => {
        if (page.bytes.length > 0)
          handlers.onBytes(page.bytes);
        // The control frame follows the bytes it accounts for, exactly as the
        // loopback server orders them.
        handlers.onMeta({
          cursor: page.cursor,
          start: page.start,
          ...(page.exitCode === null ? {} : { exitCode: page.exitCode }),
        });
        seek = page.cursor;
        burst = page.bytes.length > 0 ? burst + 1 : 0;
      };

      const poll = async () => {
        if (stopped)
          return;
        let page: RemoteTerminalPage;
        try {
          page = await readRemoteTerminal(desktopId, terminalId, seek);
          failures = 0;
        }
        catch (cause) {
          if (stopped)
            return;
          const error = remoteTerminalError(cause);
          if (error.code === "TERMINAL_NOT_FOUND" || error.code === "TERMINALS_UNSUPPORTED") {
            handlers.onMissing();
            stop(CLOSE_EXIT);
            return;
          }
          failures += 1;
          if (failures >= READ_FAILURES) {
            // Lagged semantics: the view re-attaches from the cursor it applied.
            stop(4408);
            return;
          }
          wake = setTimeout(() => void poll(), IDLE_POLL_MS);
          return;
        }
        deliver(page);
        if (page.status === "exited") {
          stop(CLOSE_EXIT);
          return;
        }
        // A shell that answered keeps answering: drain a small burst now rather
        // than at the idle clock, but never let one runaway command own the
        // round trip — past that it is paced like any other busy shell.
        const more = page.bytes.length > 0 && burst < MAX_PAGES_PER_WAKE;
        wake = setTimeout(() => void poll(), more ? BUSY_POLL_MS : IDLE_POLL_MS);
      };

      const flush = async () => {
        if (writing || stopped || pending === "")
          return;
        const batch = pending;
        pending = "";
        writing = true;
        try {
          await writeRemoteTerminal(desktopId, terminalId, new TextEncoder().encode(batch));
        }
        catch (cause) {
          // A shell that exited is reported by the reader (it has the exit
          // code); anything else is retried with the next batch rather than
          // raised here, where no one is waiting for the answer.
          void remoteTerminalError(cause).code;
        }
        finally {
          writing = false;
        }
        if (pending !== "" && !stopped)
          void flush();
      };

      // The first read *is* the attach: a refusal here is the caller's to see
      // (a tab whose shell is gone must not spin), so it propagates.
      const first = await readRemoteTerminal(desktopId, terminalId, seek);
      if (signal.aborted) {
        // The view went away while the first page was in flight. It asked for
        // no attachment, so none is made.
        stopped = true;
        throw new AttachAborted();
      }
      deliver(first);
      handlers.onOpen();
      if (first.status === "exited")
        stop(CLOSE_EXIT);
      else
        void poll();

      return {
        write(data: string) {
          if (stopped || data === "")
            return;
          pending += data;
          flushTimer ??= setTimeout(() => {
            flushTimer = undefined;
            void flush();
          }, INPUT_BATCH_MS);
        },
        close() {
          if (stopped)
            return;
          stopped = true;
          clearWake();
          if (flushTimer !== undefined) {
            clearTimeout(flushTimer);
            flushTimer = undefined;
          }
        },
      };
    },

    async resize(terminalId, cols, rows) {
      // Best effort: a resize racing the shell's exit is not an error the user
      // should see, and the next poll reports the exit either way.
      await updateRemoteTerminal(desktopId, terminalId, { cols, rows }).catch(() => undefined);
    },
  };
}
