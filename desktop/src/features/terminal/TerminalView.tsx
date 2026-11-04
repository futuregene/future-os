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
import type { TerminalLink, TerminalTransport } from "./transport";
import type { TerminalFrameMeta } from "./types";
import { FitAddon } from "@xterm/addon-fit";
import { SerializeAddon } from "@xterm/addon-serialize";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef } from "react";
import { TerminalApiError } from "./client";
import { terminalKeyPolicy } from "./keyPolicy";
import { localTransport } from "./localTransport";
import { installMacInputWorkaround } from "./macInput";
import { currentTerminalTheme, useTerminalTheme } from "./theme";
import { AttachAborted, CLOSE_EXIT, CLOSE_LAGGED } from "./transport";
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

export interface TerminalViewProps {
  tab: TerminalTab;
  /**
   * How this view reaches its shell. A local tab uses the loopback WebSocket; a
   * paired desktop's tab polls the control plane. Everything below the seam —
   * cursor accounting, replay truncation, exit handling, retries — is shared.
   */
  transport?: TerminalTransport;
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
  const transport = props.transport ?? localTransport;
  const containerRef = useRef<HTMLDivElement | null>(null);
  // The xterm instance, so a theme switch can repaint an already-open terminal
  // without re-running the mount effect (which would drop the socket).
  const terminalRef = useRef<Terminal | null>(null);
  // Both theme axes (rendition and palette) feed the terminal's colors, so one
  // value tracks them together.
  const terminalTheme = useTerminalTheme();
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
    let link: TerminalLink | undefined;
    // An attach still waiting for its endpoint, ticket or first page is aborted
    // on unmount instead of opening a connection nobody will read.
    const abort = new AbortController();
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
    // True once this attachment's control frame has arrived. Everything before
    // it is replay, and its extent is what that frame reports; everything after
    // it is live output, which arrives as bare bytes and has to be counted here
    // (see the message handler).
    let replaying = true;
    const disposables: IDisposable[] = [];

    const terminal = new Terminal({
      cols: initial.cols,
      rows: initial.rows,
      cursorBlink: true,
      cursorStyle: "bar",
      fontSize: 13,
      scrollback: SCROLLBACK,
      allowTransparency: false,
      convertEol: false,
      theme: currentTerminalTheme(),
    });
    terminalRef.current = terminal;
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
    const removeInputWorkaround = installMacInputWorkaround(terminal);

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
        void transport.resize(id, cols, rows).catch(() => undefined);
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
        void transport.resize(id, lastSize.cols, lastSize.rows).catch(() => undefined);
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
      // The attachment's replay is over even when the payload cannot be read:
      // every later frame is live output and has to move the cursor.
      replaying = false;
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

    // Which attachment is current. A retry that lands after a newer one started
    // must not steal the link (or leave a second one open).
    let generation = 0;

    async function open(): Promise<void> {
      if (disposed) {
        return;
      }
      const mine = ++generation;
      // A new attachment starts with a replay again.
      replaying = true;
      const decoder = new TextDecoder();
      let next: TerminalLink;
      try {
        next = await transport.open(id, seek ?? -1, {
          onOpen: () => {
            if (disposed || mine !== generation) {
              return;
            }
            tries = 0;
            lastSize = undefined;
            pushSize(terminal.cols, terminal.rows);
            handlersRef.current.onConnect?.();
          },
          onBytes: (payload) => {
            if (disposed || mine !== generation) {
              return;
            }
            // Live output advances the stored cursor as it is applied: the host
            // counts the stream in bytes and reports that offset once per
            // attachment (and again on exit), so without this the cursor
            // persisted on unmount stays at the attach-time offset and the next
            // mount replays output the restored screen already shows.
            if (!replaying) {
              cursor = (cursor ?? 0) + payload.length;
            }
            push(decoder.decode(payload, { stream: true }));
          },
          onMeta: (meta) => {
            if (disposed || mine !== generation) {
              return;
            }
            applyMeta(meta);
          },
          onText: (text) => {
            if (disposed || mine !== generation) {
              return;
            }
            // A transport that handed over something it could not classify:
            // rendering it is strictly better than dropping output.
            push(text);
          },
          onMissing: () => {
            if (disposed || mine !== generation) {
              return;
            }
            // The shell is gone (the app restarted, or the session was
            // evicted). The tab keeps its final screen and offers a restart.
            handlersRef.current.onMissing?.();
          },
          onClose: (code) => {
            if (disposed || mine !== generation) {
              return;
            }
            if (code === CLOSE_EXIT) {
              // The shell exited: the final control frame already carried the
              // code.
              return;
            }
            if (code === CLOSE_LAGGED) {
              // The host dropped this viewer to bound memory, or the link
              // faltered. Come back with the cursor actually applied and replay
              // only the difference.
              seek = cursor;
              scheduleRetry(0);
              return;
            }
            retry(new Error(`terminal connection closed (${code})`));
          },
        }, abort.signal);
      }
      catch (cause) {
        if (disposed || mine !== generation || cause instanceof AttachAborted) {
          return;
        }
        if (cause instanceof TerminalApiError && cause.code === "TERMINAL_NOT_FOUND") {
          handlersRef.current.onMissing?.();
          return;
        }
        retry(cause);
        return;
      }
      if (disposed || mine !== generation) {
        next.close();
        return;
      }
      link = next;
    }

    const onData = terminal.onData((data) => {
      link?.write(data);
    });
    disposables.push(onData);
    const onResize = terminal.onResize(size => pushSize(size.cols, size.rows));
    disposables.push(onResize);
    const onWindowResize = () => scheduleFit();
    window.addEventListener("resize", onWindowResize);
    // xterm's fit addon does not observe anything itself: refit when the panel
    // (or the window) changes the container size.
    const observer = new ResizeObserver(() => scheduleFit());
    observer.observe(container);

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
      abort.abort();
      if (retryTimer !== undefined) {
        clearTimeout(retryTimer);
      }
      if (resizeTimer !== undefined) {
        clearTimeout(resizeTimer);
      }
      if (fitFrame !== undefined) {
        cancelAnimationFrame(fitFrame);
      }
      removeInputWorkaround();
      window.removeEventListener("resize", onWindowResize);
      observer.disconnect();
      for (const disposable of disposables.splice(0)) {
        disposable.dispose();
      }
      link?.close();
      // Persist the *applied* screen: draining first guarantees the last chunk
      // of output is part of what the next mount restores.
      flush(() => {
        persist();
        terminalRef.current = null;
        terminal.dispose();
      });
    };
    // Mount once per session id; every other prop is read through the captured
    // `initial` snapshot or the handlers ref.
    // eslint-disable-next-line react/exhaustive-deps
  }, [id]);

  // Repaint an open terminal when the app theme changes. xterm keeps its own
  // palette, so the CSS-variable flip does not reach it.
  useEffect(() => {
    const terminal = terminalRef.current;
    if (terminal) {
      terminal.options.theme = terminalTheme;
    }
  }, [terminalTheme]);

  return (
    <div
      ref={containerRef}
      data-component="terminal"
      data-terminal-root={id}
      dir="ltr"
      className="h-full w-full overflow-hidden bg-surface px-3 py-2"
    />
  );
}
