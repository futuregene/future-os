/**
 * CancellableLoader — loader that can be cancelled with Escape.
 * Extends Loader with an AbortSignal for cancelling async operations.
 *
 * Usage:
 *   const loader = new CancellableLoader(cyan, dim, "Working...");
 *   loader.onAbort = () => done(null);
 *   doWork(loader.signal).then(done);
 */
import { Loader } from "./loader.js";
export class CancellableLoader extends Loader {
    abortController = new AbortController();
    /** Called when user presses Escape. */
    onAbort;
    /** AbortSignal that is aborted when user presses Escape. */
    get signal() {
        return this.abortController.signal;
    }
    /** Whether the loader was aborted. */
    get aborted() {
        return this.abortController.signal.aborted;
    }
    constructor(spinnerColorFn, messageColorFn, message = "Loading...", indicator) {
        super(spinnerColorFn, messageColorFn, message, indicator);
    }
    handleInput(data) {
        if (data === "escape") {
            this.abortController.abort();
            this.onAbort?.();
        }
    }
    dispose() {
        this.stop();
    }
}
