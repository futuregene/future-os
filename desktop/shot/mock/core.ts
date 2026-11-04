/**
 * Mock of `@tauri-apps/api/core` — the single IPC entry point.
 *
 * Hands every command to the in-page backend (`./backend`), which answers with
 * the same payload shapes the Rust side returns. Unknown commands are logged so
 * the console output doubles as a worklist when the app asks for something new.
 */
import { dispatch } from "./backend";

export interface InvokeArgs {
  [key: string]: unknown;
}

export interface InvokeOptions {
  headers?: Record<string, string>;
}

/** Mirrors the real one-shot event channel the agent prompt uses. */
export class Channel<T = unknown> {
  onmessage: (message: T) => void;

  constructor(onmessage?: (message: T) => void) {
    this.onmessage = onmessage ?? (() => {});
  }

  toJSON(): string {
    return "__CHANNEL__";
  }
}

/**
 * Demo files that exist as real PNGs under `shot/assets/`, so a local-path
 * image inside a message renders an actual picture in the browser instead of a
 * Tauri asset URL the browser cannot resolve.
 */
const ASSETS: Array<[RegExp, string]> = [
  [/effect-size\.png$/, "/shot/assets/effect-size.png"],
  [/forest-plot\.png$/, "/shot/assets/forest-plot.png"],
  // The parser session's benchmark figure; the reply embeds it with a path in
  // the demo checkout, and this is what turns that path into a real PNG.
  [/parser-throughput\.png$/, "/shot/assets/parser-throughput.png"],
];

/** `convertFileSrc` equivalent: map the demo dataset's file paths to URLs. */
export function convertFileSrc(path: string, _protocol?: string): string {
  return ASSETS.find(([pattern]) => pattern.test(path))?.[1] ?? path;
}

/**
 * `?fixture=http://127.0.0.1:PORT` routes commands to the UI test harness's
 * fixture server (see `harness/lib/fixture-server.mjs`) instead of the in-page
 * demo dataset.
 *
 * The server answers from a copy of a real `app.db`, so lists carry the real
 * row counts and titles and writes (create/delete) persist for the case. A
 * command it does not implement answers `{handled:false}` and falls through to
 * the demo dataset below — non-data surfaces (remote peers, task list) keep
 * working while the data surface is real. This exists only in the shot harness;
 * it is not part of the shipped app.
 */
const FIXTURE = typeof location === "undefined" ? null : new URLSearchParams(location.search).get("fixture");

/**
 * IPC args can carry `Channel` instances and callbacks the Rust side never
 * sees as JSON. Send the data half only; `toJSON` already marks channels.
 */
function fixtureArgs(args?: InvokeArgs): unknown {
  if (!args)
    return null;
  const seen = new WeakSet<object>();
  return JSON.parse(JSON.stringify(args, (_key, value) => {
    if (typeof value === "function")
      return undefined;
    if (value && typeof value === "object") {
      if (seen.has(value))
        return undefined;
      seen.add(value);
    }
    return value;
  }));
}

export async function invoke<T = unknown>(
  command: string,
  args?: InvokeArgs,
  _options?: InvokeOptions,
): Promise<T> {
  // The real `agent_prompt` reports acceptance through a Channel before it
  // resolves; replay that so the streaming-acceptance path stays exercised.
  if (args?.onAccepted instanceof Channel)
    setTimeout(() => (args.onAccepted as Channel<null>).onmessage(null), 10);

  if (FIXTURE) {
    try {
      const response = await fetch(`${FIXTURE}/invoke`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ command, args: fixtureArgs(args) }),
      });
      const payload = await response.json() as { handled?: boolean; value?: unknown; error?: string | null };
      if (payload.handled) {
        if (payload.error)
          throw new Error(payload.error);
        return payload.value as T;
      }
    }
    catch (error) {
      // A command the fixture server *did* handle and rejected must surface, or
      // the UI would silently render a successful write that never happened.
      if (error instanceof Error && !/fetch|ECONNREFUSED|Failed to fetch/i.test(error.message))
        throw error;
      console.warn(`[fixture] unreachable for ${command}; falling back to demo data`, error);
    }
  }

  return dispatch(command, args) as T;
}
