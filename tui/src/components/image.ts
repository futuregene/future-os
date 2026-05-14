/**
 * Image component - renders images in the terminal via Kitty/iTerm2 protocol.
 * Falls back to text placeholder when terminal lacks image support.
 */

import {
  allocateImageId,
  getCapabilities,
  getImageDimensions,
  type ImageDimensions,
  imageFallback,
  renderImage,
} from "../terminal-image.js";
import type { Component } from "../tui.js";

export interface ImageTheme {
  fallbackColor: (str: string) => string;
}

export interface ImageOptions {
  maxWidthCells?: number;
  maxHeightCells?: number;
  filename?: string;
  imageId?: number;
}

export class Image implements Component {
  private base64Data: string;
  private mimeType: string;
  private dimensions: ImageDimensions;
  private theme: ImageTheme;
  private options: ImageOptions;
  private imageId?: number;

  private cachedLines?: string[];
  private cachedWidth?: number;

  constructor(
    base64Data: string,
    mimeType: string,
    theme: ImageTheme,
    options: ImageOptions = {},
    dimensions?: ImageDimensions,
  ) {
    this.base64Data = base64Data;
    this.mimeType = mimeType;
    this.theme = theme;
    this.options = options;
    this.dimensions = dimensions || getImageDimensions(base64Data, mimeType) || { widthPx: 800, heightPx: 600 };
    this.imageId = options.imageId;
  }

  getImageId(): number | undefined {
    return this.imageId;
  }

  invalidate(): void {
    this.cachedLines = undefined;
    this.cachedWidth = undefined;
  }

  render(width: number): string[] {
    if (this.cachedLines && this.cachedWidth === width) {
      return this.cachedLines;
    }

    const maxWidth = Math.min(width - 2, this.options.maxWidthCells ?? 60);

    const caps = getCapabilities();
    let lines: string[];

    if (caps.images) {
      if (caps.images === "kitty" && this.imageId === undefined) {
        this.imageId = allocateImageId();
      }
      const result = renderImage(this.base64Data, this.dimensions, {
        maxWidthCells: maxWidth,
        imageId: this.imageId,
        moveCursor: false,
      });

      if (result) {
        if (result.imageId) {
          this.imageId = result.imageId;
        }

        lines = [];
        for (let i = 0; i < result.rows - 1; i++) {
          lines.push("");
        }
        const rowOffset = result.rows - 1;
        const moveUp = rowOffset > 0 ? `\x1b[${rowOffset}A` : "";
        const moveDown = caps.images === "kitty" && rowOffset > 0 ? `\x1b[${rowOffset}B` : "";
        lines.push(moveUp + result.sequence + moveDown);
      } else {
        const fallback = imageFallback(this.mimeType, this.dimensions, this.options.filename);
        lines = [this.theme.fallbackColor(fallback)];
      }
    } else {
      const fallback = imageFallback(this.mimeType, this.dimensions, this.options.filename);
      lines = [this.theme.fallbackColor(fallback)];
    }

    this.cachedLines = lines;
    this.cachedWidth = width;

    return lines;
  }
}
