import { readFileBase64, writeThumbnail } from "../../integrations/storage/files";
import { extOf, READ_SOURCE_MAX_BYTES } from "./attachments";

const EXT_IMAGE_MIME: Record<string, string> = {
  bmp: "image/bmp",
  gif: "image/gif",
  jpeg: "image/jpeg",
  jpg: "image/jpeg",
  png: "image/png",
  svg: "image/svg+xml",
  webp: "image/webp",
};

function loadImage(src: string) {
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("image decode failed"));
    image.src = src;
  });
}

/**
 * Downscale an image to ~96px and persist a JPEG thumbnail under the thread's
 * persistent image dir (`~/.future/app/images/<threadId>/thumb`). The backend
 * assigns a unique filename, so no client-side key is needed. DOM/canvas work
 * lives here rather than in the classification module.
 */
export async function generateImageThumbnail(path: string, threadId: string): Promise<string | null> {
  try {
    const ext = extOf(path);
    const base64 = await readFileBase64({ maxBytes: READ_SOURCE_MAX_BYTES, path });
    const mime = EXT_IMAGE_MIME[ext] ?? "image/png";
    const image = await loadImage(`data:${mime};base64,${base64}`);
    const max = 96;
    const scale = Math.min(1, max / Math.max(image.width || 1, image.height || 1));
    const width = Math.max(1, Math.round((image.width || max) * scale));
    const height = Math.max(1, Math.round((image.height || max) * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx)
      return null;
    ctx.drawImage(image, 0, 0, width, height);
    const jpeg = canvas.toDataURL("image/jpeg", 0.6).split(",")[1] ?? "";
    if (!jpeg)
      return null;
    return await writeThumbnail({ base64Jpeg: jpeg, threadId });
  }
  catch {
    return null;
  }
}
