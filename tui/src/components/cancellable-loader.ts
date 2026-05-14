/**
 * CancellableLoader — loader that can be cancelled with Escape.
 * Extends Loader with an AbortSignal for cancelling async operations.
 *
 * Usage:
 *   const loader = new CancellableLoader(cyan, dim, "Working...");
 *   loader.onAbort = () => done(null);
 *   doWork(loader.signal).then(done);
 */

import { Loader, type LoaderIndicatorOptions } from "./loader.js";

export class CancellableLoader extends Loader {
  private abortController = new AbortController();

  /** Called when user presses Escape. */
  onAbort?: () => void;

  /** AbortSignal that is aborted when user presses Escape. */
  get signal(): AbortSignal {
    return this.abortController.signal;
  }

  /** Whether the loader was aborted. */
  get aborted(): boolean {
    return this.abortController.signal.aborted;
  }

  constructor(
    spinnerColorFn: (str: string) => string,
    messageColorFn: (str: string) => string,
    message = "Loading...",
    indicator?: LoaderIndicatorOptions,
  ) {
    super(spinnerColorFn, messageColorFn, message, indicator);
  }

  handleInput(data: string): void {
    if (data === "escape") {
      this.abortController.abort();
      this.onAbort?.();
    }
  }

  dispose(): void {
    this.stop();
  }
}
