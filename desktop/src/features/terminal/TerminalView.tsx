/**
 * The mounted terminal for one tab.
 *
 * Mirrors opencode's `Terminal`: xterm owns the screen, the Rust side owns the
 * shell, and the WebSocket carries raw bytes between them. On mount the
 * component restores the screen it last serialized and resumes from that
 * cursor, so collapsing the panel, reloading the webview or remounting a tab
 * never re-renders history and never loses a running shell.
 *
 * This module is loaded lazily by the panel (`React.lazy`), which keeps xterm
 * out of the main bundle.
 */
import type { IDisposable } from "@xterm/xterm";
import type { TerminalTab } from "./tabs";
import type { TerminalFrameMeta } from "./types";
import { FitAddon } from "@xterm/addon-fit";
import { SerializeAddon } from "@xterm/addon-serialize";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef } from "react";
import { connectTicket, connectUrl, TerminalApiError, terminalServer, updateTerminal } from "./client";
import { terminalKeyPolicy } from "./keyPolicy";
import { TERMINAL_THEME } from "./theme";
import "@xterm/xterm/css/xterm.css";

/** How long a resize waits for the drag to settle before telling the server. */
const RESIZE_DEBOUNCE_MS = 100;
/** Reconnect backoff bounds. */
const RETRY_MIN_MS = 250;
const RETRY_MAX_MS = 4000;
/** Scrollback kept in the view; the server keeps its own bounded tail. */
const SCROLLBACK = 10_000;
/**
 * Lines serialized into the persisted view state — bounded so a long session
 * cannot exhaust the storage quota.
 */
const PERSIST_SCROLLBACK = 1_000;
/** Close code the server sends when a viewer fell too far behind. */
const CLOSE_LAGGED = 4408;

export interface TerminalViewProps {
  tab: TerminalTab;
  autoFocus?: boolean;
  /** Attached and streaming. */
  onConnect?: () => void;
  /** The server no longer has this session; the tab needs a restart. */
  onMissing?: () => void;
  /** The shell exited, with its code when the server could report one. */
  onExit: (exitCode: number | null) => void;
  /** Persist the screen before this tab unmounts. */
  onPersist: (patch: Partial<TerminalTab>) => void;
  /** The stream dropped; the view is retrying. */
  onDisconnected?: (reason: string) => void;
}

export function TerminalView(props: TerminalViewProps) {
  const { tab, autoFocus } = props;
  const containerRef = useRef<HTMLDivElement | null>(null);
  // Latest callbacks without re-running the mount effect, which would tear the
  // socket down and reconnect on every parent render.
  const handlersRef = useRef(props);
  handlersRef.current = props;

  const id = tab.id;
  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const initial = tab;
    let disposed = false;
    let socket: WebSocket | undefined;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    let fitFrame: number | undefined;
    let tries = 0;
    let lastSize: { cols: number; rows: number } | undefined;
    let cursor: number | undefined = initial.cursor;
    // A restored screen already contains everything up to `cursor`. Without a
    // stored cursor we only know what the screen shows, so tail the stream
    // rather than replaying (which would duplicate the restored output).
    let seek: number | undefined = initial.cursor ?? (initial.buffer ? -1 : 0);
    const disposables: IDisposable[] = [];
    const listeners: Array<() => void> = [];

    const terminal = new Terminal({
      cols: initial.cols,
      rows: initial.rows,
      cursorBlink: true,
      cursorStyle: "bar",
      fontSize: 13,
      scrollback: SCROLLBACK,
      allowTransparency: false,
      convertEol: false,
      theme: TERMINAL_THEME,
    });
    disposables.push(terminal);
    // Two keys belong to someone else. The panel shortcut is the app's (see
    // `useTerminalPanel`), and while an IME is composing every key belongs to
    // the IME: xterm's own composition heuristic only knows Chromium's `229`, so
    // on WebKit it committed the half-typed composition when the user pressed
    // Backspace to fix a letter, typing the text twice. Returning false stands
    // xterm down and lets the event reach its owner.
    // Details and the reproduction: `./keyPolicy`.
    terminal.attachCustomKeyEventHandler(event => terminalKeyPolicy(event));
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    const serializer = new SerializeAddon();
    terminal.loadAddon(serializer);
    terminal.open(container);

    // Output is queued and applied in arrival order. xterm serialises its own
    // writes, but `serialize()` reads the *applied* buffer, so persistence
    // waits for the queue to drain (see the unmount path below). The queue also
    // keeps control frames ordered with the bytes around them.
    let queue: string[] = [];
    let writing = false;
    let flushWaiters: Array<() => void> = [];
    const drain = () => {
      if (writing) {
        return;
      }
      const data = queue.join("");
      queue = [];
      if (!data) {
        const waiters = flushWaiters;
        flushWaiters = [];
        for (const waiter of waiters) {
          waiter();
        }
        return;
      }
      writing = true;
      terminal.write(data, () => {
        writing = false;
        drain();
      });
    };
    const push = (data: string) => {
      if (!data) {
        return;
      }
      queue.push(data);
      drain();
    };
    const flush = (done: () => void) => {
      if (!writing && queue.length === 0) {
        done();
        return;
      }
      flushWaiters.push(done);
      drain();
    };

    const scheduleFit = () => {
      if (disposed || fitFrame !== undefined) {
        return;
      }
      fitFrame = requestAnimationFrame(() => {
        fitFrame = undefined;
        if (!disposed) {
          fit.fit();
        }
      });
    };

    const pushSize = (cols: number, rows: number) => {
      if (lastSize?.cols === cols && lastSize?.rows === rows) {
        return;
      }
      const previous = lastSize;
      lastSize = { cols, rows };
      if (!previous) {
        // The first measurement goes out immediately so the shell starts with
        // the geometry the user actually sees.
        void updateTerminal(id, { cols, rows }).catch(() => undefined);
        return;
      }
      if (resizeTimer !== undefined) {
        return;
      }
      resizeTimer = setTimeout(() => {
        resizeTimer = undefined;
        if (disposed || !lastSize) {
          return;
        }
        void updateTerminal(id, lastSize).catch(() => undefined);
      }, RESIZE_DEBOUNCE_MS);
    };

    const persist = () => {
      const buffer = (() => {
        try {
          return serializer.serialize({ scrollback: PERSIST_SCROLLBACK });
        }
        catch {
          return undefined;
        }
      })();
      handlersRef.current.onPersist({
        ...(buffer === undefined ? {} : { buffer }),
        ...(cursor === undefined ? {} : { cursor }),
        rows: terminal.rows,
        cols: terminal.cols,
        scrollY: terminal.buffer.active.viewportY,
      });
    };

    const applyMeta = (meta: TerminalFrameMeta | undefined) => {
      if (!meta) {
        return;
      }
      // A replay that starts after the cursor we asked for means the server no
      // longer retains what we missed. Say so instead of showing a hole.
      if (typeof seek === "number" && seek >= 0 && meta.start > seek) {
        push("\r\n\x1B[33m[output was truncated while this view was detached]\x1B[0m\r\n");
      }
      cursor = meta.cursor;
      seek = meta.cursor;
      if (meta.exitCode !== undefined) {
        handlersRef.current.onExit(meta.exitCode ?? null);
      }
    };

    const scheduleRetry = (delay: number) => {
      if (disposed || retryTimer !== undefined) {
        return;
      }
      retryTimer = setTimeout(() => {
        retryTimer = undefined;
        if (disposed) {
          return;
        }
        tries += 1;
        void open();
      }, delay);
    };

    const retry = (cause: unknown) => {
      if (disposed) {
        return;
      }
      handlersRef.current.onDisconnected?.(cause instanceof Error ? cause.message : String(cause));
      scheduleRetry(Math.min(RETRY_MIN_MS * 2 ** Math.min(tries, 4), RETRY_MAX_MS));
    };

    async function open(): Promise<void> {
      if (disposed) {
        return;
      }
      let ticket: string;
      try {
        ticket = await connectTicket(id);
      }
      catch (cause) {
        if (disposed) {
          return;
        }
        if (cause instanceof TerminalApiError && cause.code === "TERMINAL_NOT_FOUND") {
          // The shell is gone (app restarted, or the session was evicted). The
          // tab keeps its final screen and offers a restart.
          handlersRef.current.onMissing?.();
          return;
        }
        retry(cause);
        return;
      }
      if (disposed) {
        return;
      }

      let ws: WebSocket;
      try {
        const server = await terminalServer();
        if (disposed) {
          return;
        }
        ws = new WebSocket(connectUrl(server, id, seek, ticket));
      }
      catch (cause) {
        retry(cause);
        return;
      }
      ws.binaryType = "arraybuffer";
      socket = ws;
      const decoder = new TextDecoder();

      // These listeners belong to this attachment's socket, which the effect
      // closes on unmount, and every handler re-checks `disposed` — the rule
      // cannot see either fact.
      // eslint-disable-next-line react/web-api-no-leaked-event-listener
      ws.addEventListener("open", () => {
        if (disposed) {
          ws.close();
          return;
        }
        tries = 0;
        lastSize = undefined;
        pushSize(terminal.cols, terminal.rows);
        handlersRef.current.onConnect?.();
      });
      // These listeners belong to this attachment's socket, which the effect
      // closes on unmount, and every handler re-checks `disposed` — the rule
      // cannot see either fact.
      // eslint-disable-next-line react/web-api-no-leaked-event-listener
      ws.addEventListener("message", (event: MessageEvent) => {
        if (disposed) {
          return;
        }
        if (typeof event.data === "string") {
          // The server sends binary frames; rendering an unexpected text frame
          // is strictly better than dropping output.
          push(event.data);
          return;
        }
        const bytes = new Uint8Array(event.data as ArrayBuffer);
        if (bytes.length > 0 && bytes[0] === 0) {
          applyMeta(decodeMeta(bytes.subarray(1)));
          return;
        }
        push(decoder.decode(bytes, { stream: true }));
      });
      // These listeners belong to this attachment's socket, which the effect
      // closes on unmount, and every handler re-checks `disposed` — the rule
      // cannot see either fact.
      // eslint-disable-next-line react/web-api-no-leaked-event-listener
      ws.addEventListener("close", (event: CloseEvent) => {
        if (socket === ws) {
          socket = undefined;
        }
        if (disposed) {
          return;
        }
        if (event.code === 1000) {
          // The shell exited: the final control frame already carried the code.
          return;
        }
        if (event.code === CLOSE_LAGGED) {
          // The server dropped this viewer to bound memory. Come back with the
          // cursor actually applied and replay only the difference.
          seek = cursor;
          scheduleRetry(0);
          return;
        }
        retry(new Error(`terminal connection closed (${event.code})`));
      });
    }

    const onData = terminal.onData((data) => {
      if (socket?.readyState === WebSocket.OPEN) {
        socket.send(data);
      }
    });
    disposables.push(onData);
    const onResize = terminal.onResize(size => pushSize(size.cols, size.rows));
    disposables.push(onResize);
    const onWindowResize = () => scheduleFit();
    // Registered here and torn down through `listeners` below (the rule only
    // recognises a cleanup returned directly from the effect).
    // eslint-disable-next-line react/web-api-no-leaked-event-listener
    window.addEventListener("resize", onWindowResize);
    listeners.push(() => window.removeEventListener("resize", onWindowResize));
    // xterm's fit addon does not observe anything itself: refit when the panel
    // (or the window) changes the container size.
    const observer = new ResizeObserver(() => scheduleFit());
    observer.observe(container);
    listeners.push(() => observer.disconnect());

    // Restore before fitting: a wide stored buffer must reflow to the real
    // geometry, not the other way round.
    if (initial.buffer) {
      terminal.write(initial.buffer);
    }
    fit.fit();
    if (initial.scrollY !== undefined) {
      terminal.scrollToLine(initial.scrollY);
    }
    pushSize(terminal.cols, terminal.rows);
    if (autoFocus) {
      terminal.focus();
    }
    void open();

    return () => {
      disposed = true;
      if (retryTimer !== undefined) {
        clearTimeout(retryTimer);
      }
      if (resizeTimer !== undefined) {
        clearTimeout(resizeTimer);
      }
      if (fitFrame !== undefined) {
        cancelAnimationFrame(fitFrame);
      }
      for (const remove of listeners.splice(0)) {
        remove();
      }
      for (const disposable of disposables.splice(0)) {
        disposable.dispose();
      }
      if (socket && socket.readyState !== WebSocket.CLOSED && socket.readyState !== WebSocket.CLOSING) {
        socket.close(1000);
      }
      // Persist the *applied* screen: draining first guarantees the last chunk
      // of output is part of what the next mount restores.
      flush(() => {
        persist();
        terminal.dispose();
      });
    };
    // Mount once per session id; every other prop is read through the captured
    // `initial` snapshot or the handlers ref.
    // eslint-disable-next-line react/exhaustive-deps
  }, [id]);

  return (
    <div
      ref={containerRef}
      data-component="terminal"
      data-terminal-root={id}
      dir="ltr"
      className="h-full w-full overflow-hidden bg-white px-3 py-2"
    />
  );
}

/**
 * Decode a control frame.
 *
 * A decoder of its own, deliberately: the streaming decoder that decodes PTY
 * output holds the bytes of a character split across two frames, and a
 * non-streaming `decode()` on the same instance flushes those bytes out as
 * U+FFFD — which used to eat the end of a replay (and with it the control frame
 * that reports the cursor) whenever the 64 KiB replay chunk boundary fell inside
 * a multi-byte character. Control payloads are ASCII JSON.
 */
function decodeMeta(payload: Uint8Array): TerminalFrameMeta | undefined {
  try {
    return JSON.parse(new TextDecoder().decode(payload)) as TerminalFrameMeta;
  }
  catch {
    return undefined;
  }
}
