/**
 * Chromium screenshot — captures via Page.captureScreenshot.
 *
 * Protocol-specific. screenshot-writer.ts handles path/retry/output formatting.
 */
import type { CdpSession } from "./cdp-connection.js";
import type { CaptureScreenshotOptions } from "../backend.js";

/**
 * Capture a screenshot using CDP Page.captureScreenshot.
 * Returns raw image bytes (decoded from base64).
 */
export async function captureScreenshot(
  session: CdpSession,
  options: CaptureScreenshotOptions,
): Promise<Uint8Array> {
  const params: Record<string, unknown> = {
    format: options.format,
  };

  if (options.quality !== undefined) {
    params.quality = options.quality;
  }

  if (options.fullPage) {
    params.captureBeyondViewport = true;

    // Get full page dimensions
    const metrics = await session.send("Page.getLayoutMetrics") as {
      cssContentSize: { width: number; height: number };
    };

    // Set clip to full content size
    params.clip = {
      x: 0,
      y: 0,
      width: metrics.cssContentSize.width,
      height: metrics.cssContentSize.height,
      scale: 1,
    };
  }

  const result = await session.send("Page.captureScreenshot", params) as {
    data: string;
  };

  // Decode base64 to Uint8Array
  const binary = atob(result.data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}
