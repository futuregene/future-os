import { beforeEach, describe, expect, it, vi } from "vitest";
import { persistImageAttachments } from "./threadAttachments";

const deleteTempAttachment = vi.fn();
const generateImageThumbnail = vi.fn();
const importEphemeralImage = vi.fn();
const validateImageAttachment = vi.fn();

vi.mock("../../integrations/storage/files", () => ({
  deleteTempAttachment: (...args: unknown[]) => deleteTempAttachment(...args),
  generateImageThumbnail: (...args: unknown[]) => generateImageThumbnail(...args),
  importEphemeralImage: (...args: unknown[]) => importEphemeralImage(...args),
  validateImageAttachment: (...args: unknown[]) => validateImageAttachment(...args),
}));

describe("persistImageAttachments", () => {
  beforeEach(() => {
    deleteTempAttachment.mockReset();
    generateImageThumbnail.mockReset();
    importEphemeralImage.mockReset();
    validateImageAttachment.mockReset();
    validateImageAttachment.mockResolvedValue(undefined);
  });

  it("rejects the whole batch when an image fails to decode, before moving any original", async () => {
    generateImageThumbnail.mockResolvedValue("/thumb/ok.jpg");
    validateImageAttachment
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(new Error("bad image"));

    await expect(persistImageAttachments([
      { kind: "image", name: "ok.png", path: "/tmp/futureos-attachments/ok.png" },
      { kind: "image", name: "bad.png", path: "/tmp/futureos-attachments/bad.png" },
    ], "thread-1")).rejects.toThrow("bad.png");

    expect(importEphemeralImage).not.toHaveBeenCalled();
    expect(deleteTempAttachment).not.toHaveBeenCalled();
  });

  it("still sends a decodable image without a thumbnail when thumbnail generation fails", async () => {
    generateImageThumbnail.mockRejectedValue(new Error("disk full"));
    importEphemeralImage.mockResolvedValue("/origin/ok.png");
    deleteTempAttachment.mockResolvedValue(undefined);

    // A thumbnail write failure must not reject the send: the image validated,
    // so it is persisted (durable path) and returned without a thumbnail.
    await expect(persistImageAttachments([
      { kind: "image", name: "ok.png", path: "/tmp/futureos-attachments/ok.png" },
    ], "thread-1")).resolves.toEqual([
      { kind: "image", name: "ok.png", path: "/origin/ok.png" },
    ]);
    expect(importEphemeralImage).toHaveBeenCalledTimes(1);
  });

  it("rewrites a pasted image only after validation succeeds", async () => {
    generateImageThumbnail.mockResolvedValue("/thumb/ok.jpg");
    importEphemeralImage.mockResolvedValue("/origin/ok.png");
    deleteTempAttachment.mockResolvedValue(undefined);

    await expect(persistImageAttachments([
      { kind: "image", name: "ok.png", path: "/tmp/futureos-attachments/ok.png" },
    ], "thread-1")).resolves.toEqual([
      { kind: "image", name: "ok.png", path: "/origin/ok.png", thumbnail: "/thumb/ok.jpg" },
    ]);
  });
});
