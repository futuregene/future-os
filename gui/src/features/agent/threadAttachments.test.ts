import { beforeEach, describe, expect, it, vi } from "vitest";
import { persistImageAttachments } from "./threadAttachments";

const deleteTempAttachment = vi.fn();
const generateImageThumbnail = vi.fn();
const importWorkspaceImage = vi.fn();

vi.mock("../../integrations/storage/files", () => ({
  deleteTempAttachment: (...args: unknown[]) => deleteTempAttachment(...args),
  generateImageThumbnail: (...args: unknown[]) => generateImageThumbnail(...args),
  importWorkspaceImage: (...args: unknown[]) => importWorkspaceImage(...args),
}));

describe("persistImageAttachments", () => {
  beforeEach(() => {
    deleteTempAttachment.mockReset();
    generateImageThumbnail.mockReset();
    importWorkspaceImage.mockReset();
  });

  it("validates the whole batch before moving any pasted original", async () => {
    generateImageThumbnail
      .mockResolvedValueOnce("/thumb/ok.jpg")
      .mockRejectedValueOnce(new Error("bad image"));

    await expect(persistImageAttachments([
      { kind: "image", name: "ok.png", path: "/tmp/futureos-attachments/ok.png" },
      { kind: "image", name: "bad.png", path: "/tmp/futureos-attachments/bad.png" },
    ], "thread-1")).rejects.toThrow("bad.png");

    expect(importWorkspaceImage).not.toHaveBeenCalled();
    expect(deleteTempAttachment).not.toHaveBeenCalled();
  });

  it("rewrites a pasted image only after validation succeeds", async () => {
    generateImageThumbnail.mockResolvedValue("/thumb/ok.jpg");
    importWorkspaceImage.mockResolvedValue("/origin/ok.png");
    deleteTempAttachment.mockResolvedValue(undefined);

    await expect(persistImageAttachments([
      { kind: "image", name: "ok.png", path: "/tmp/futureos-attachments/ok.png" },
    ], "thread-1")).resolves.toEqual([
      { kind: "image", name: "ok.png", path: "/origin/ok.png", thumbnail: "/thumb/ok.jpg" },
    ]);
  });
});
