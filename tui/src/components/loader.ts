/**
 * Loader component — animated braille spinner with message text.
 * Uses a callback for render requests instead of a TUI reference.
 */

import { Text } from "./text.js";

export interface LoaderIndicatorOptions {
  frames?: string[];
  intervalMs?: number;
}

const DEFAULT_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const DEFAULT_INTERVAL_MS = 80;

export class Loader extends Text {
  private frames = [...DEFAULT_FRAMES];
  private intervalMs = DEFAULT_INTERVAL_MS;
  private currentFrame = 0;
  private intervalId: ReturnType<typeof setInterval> | null = null;
  private onUpdate: (() => void) | null = null;
  private renderIndicatorVerbatim = false;

  constructor(
    private spinnerColorFn: (str: string) => string,
    private messageColorFn: (str: string) => string,
    private message = "Loading...",
    indicator?: LoaderIndicatorOptions,
  ) {
    super("", 1, 0);
    this.setIndicator(indicator);
  }

  /** Attach a render callback (called after display updates). */
  attach(onUpdate: () => void): void {
    this.onUpdate = onUpdate;
  }

  detach(): void {
    this.stop();
    this.onUpdate = null;
  }

  render(width: number): string[] {
    return ["", ...super.render(width)];
  }

  start(): void {
    this.updateDisplay();
    this.restartAnimation();
  }

  stop(): void {
    if (this.intervalId) {
      clearInterval(this.intervalId);
      this.intervalId = null;
    }
  }

  setMessage(message: string): void {
    this.message = message;
    this.updateDisplay();
  }

  setIndicator(indicator?: LoaderIndicatorOptions): void {
    this.renderIndicatorVerbatim = indicator !== undefined;
    this.frames = indicator?.frames ? [...indicator.frames] : [...DEFAULT_FRAMES];
    this.intervalMs = indicator?.intervalMs && indicator.intervalMs > 0
      ? indicator.intervalMs
      : DEFAULT_INTERVAL_MS;
    this.currentFrame = 0;
    this.start();
  }

  private restartAnimation(): void {
    this.stop();
    if (this.frames.length <= 1) return;
    this.intervalId = setInterval(() => {
      this.currentFrame = (this.currentFrame + 1) % this.frames.length;
      this.updateDisplay();
    }, this.intervalMs);
  }

  private updateDisplay(): void {
    const frame = this.frames[this.currentFrame] ?? "";
    const renderedFrame = this.renderIndicatorVerbatim
      ? frame
      : this.spinnerColorFn(frame);
    const indicator = frame.length > 0 ? `${renderedFrame} ` : "";
    this.setText(`${indicator}${this.messageColorFn(this.message)}`);
    this.onUpdate?.();
  }
}
