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
];

/** `convertFileSrc` equivalent: map the demo dataset's file paths to URLs. */
export function convertFileSrc(path: string, _protocol?: string): string {
  return ASSETS.find(([pattern]) => pattern.test(path))?.[1] ?? path;
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

  return dispatch(command, args) as T;
}
