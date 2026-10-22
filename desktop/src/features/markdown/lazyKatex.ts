import type { KatexOptions } from "katex";
import { useEffect, useSyncExternalStore } from "react";

/** The default export of `katex`: the object that renders a formula. */
export type Katex = typeof import("katex").default;

let loaded: Katex | null = null;
let inFlight: Promise<Katex> | null = null;
const listeners = new Set<() => void>();

/**
 * Start (or join) the one loading of KaTeX.
 *
 * Every formula on the screen shares it: `inFlight` is the cache. A failed load
 * clears it, so the next formula retries instead of inheriting a dead promise.
 */
export function loadKatex(): Promise<Katex> {
  inFlight ??= import("katex").then(
    (module) => {
      loaded = module.default;
      for (const notify of [...listeners])
        notify();
      return loaded;
    },
    (error: unknown) => {
      inFlight = null;
      throw error;
    },
  );
  return inFlight;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function snapshot(): Katex | null {
  return loaded;
}

/**
 * KaTeX once it has arrived, `null` before that.
 *
 * KaTeX is close to a quarter of the desktop's main chunk (≈460 kB of source),
 * and only a conversation that actually shows a formula needs it, so it is a
 * chunk of its own that the first formula pulls in. A dynamic import only pays
 * off with no static import of the same module beside it — the composer's
 * `IN EFFECTIVE DYNAMIC IMPORT` warning was the other half of that rule. Until
 * the chunk lands the formula renders as its source (the text KaTeX's own
 * failure path shows), so nothing is blank and the height is already right.
 */
export function useKatex(): Katex | null {
  useEffect(() => {
    void loadKatex().catch(() => {
      // Unreachable chunk: the formula stays as source text rather than the
      // whole message failing to render.
    });
  }, []);
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/**
 * One formula as KaTeX HTML.
 *
 * `throwOnError: false` is KaTeX's own recovery (it paints the offending source
 * in red); the `catch` covers what it still throws, because a malformed formula
 * must cost its own line and not the message around it.
 */
export function mathHtml(katex: Katex, code: string, displayMode: boolean): string {
  const options: KatexOptions = { displayMode, throwOnError: false, trust: true, strict: false };
  try {
    return katex.renderToString(code, options);
  }
  catch {
    return `<span class="text-danger">${escapeHtml(code)}</span>`;
  }
}

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}
