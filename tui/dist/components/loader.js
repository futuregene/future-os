/**
 * Loader component — animated braille spinner with message text.
 * Uses a callback for render requests instead of a TUI reference.
 */
import { Text } from "./text.js";
const DEFAULT_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const DEFAULT_INTERVAL_MS = 80;
export class Loader extends Text {
    spinnerColorFn;
    messageColorFn;
    message;
    frames = [...DEFAULT_FRAMES];
    intervalMs = DEFAULT_INTERVAL_MS;
    currentFrame = 0;
    intervalId = null;
    onUpdate = null;
    renderIndicatorVerbatim = false;
    constructor(spinnerColorFn, messageColorFn, message = "Loading...", indicator) {
        super("", 1, 0);
        this.spinnerColorFn = spinnerColorFn;
        this.messageColorFn = messageColorFn;
        this.message = message;
        this.setIndicator(indicator);
    }
    /** Attach a render callback (called after display updates). */
    attach(onUpdate) {
        this.onUpdate = onUpdate;
    }
    detach() {
        this.stop();
        this.onUpdate = null;
    }
    render(width) {
        return ["", ...super.render(width)];
    }
    start() {
        this.updateDisplay();
        this.restartAnimation();
    }
    stop() {
        if (this.intervalId) {
            clearInterval(this.intervalId);
            this.intervalId = null;
        }
    }
    setMessage(message) {
        this.message = message;
        this.updateDisplay();
    }
    setIndicator(indicator) {
        this.renderIndicatorVerbatim = indicator !== undefined;
        this.frames = indicator?.frames ? [...indicator.frames] : [...DEFAULT_FRAMES];
        this.intervalMs = indicator?.intervalMs && indicator.intervalMs > 0
            ? indicator.intervalMs
            : DEFAULT_INTERVAL_MS;
        this.currentFrame = 0;
        this.start();
    }
    restartAnimation() {
        this.stop();
        if (this.frames.length <= 1)
            return;
        this.intervalId = setInterval(() => {
            this.currentFrame = (this.currentFrame + 1) % this.frames.length;
            this.updateDisplay();
        }, this.intervalMs);
    }
    updateDisplay() {
        const frame = this.frames[this.currentFrame] ?? "";
        const renderedFrame = this.renderIndicatorVerbatim
            ? frame
            : this.spinnerColorFn(frame);
        const indicator = frame.length > 0 ? `${renderedFrame} ` : "";
        this.setText(`${indicator}${this.messageColorFn(this.message)}`);
        this.onUpdate?.();
    }
}
