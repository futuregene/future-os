import type { DirEntry } from "../../integrations/storage/files";
import { describe, expect, it } from "vitest";
import { parentDirectory, siblingImages } from "./imageSiblings";

function file(name: string, dir = "/ws/figures"): DirEntry {
  return { isDir: false, modified: 1, name, path: `${dir}/${name}`, size: 10 };
}

function dir(name: string, parent = "/ws"): DirEntry {
  return { isDir: true, modified: 1, name, path: `${parent}/${name}`, size: 0 };
}

describe("parentDirectory", () => {
  it("splits either separator and keeps the root intact", () => {
    expect(parentDirectory("/ws/figures/a.png")).toBe("/ws/figures");
    expect(parentDirectory("C:\\work\\figures\\a.png")).toBe("C:\\work\\figures");
    // A file at the root has no directory part to walk to.
    expect(parentDirectory("a.png")).toBe("");
  });
});

describe("siblingImages", () => {
  it("keeps only previewable images, in listing order", () => {
    const entries = [file("b.png"), file("script.py"), dir("nested"), file("a.jpg"), file("data.csv")];
    expect(siblingImages(entries, "/ws/figures/a.jpg").map(entry => entry.name)).toEqual(["b.png", "a.jpg"]);
  });

  it("answers with nothing when the opened file is not in the listing", () => {
    // A stale read (or a file opened from somewhere the tree does not hold) must
    // not hand the preview a list that does not contain what is on screen.
    const entries = [file("a.png"), file("b.png")];
    expect(siblingImages(entries, "/ws/figures/gone.png")).toEqual([]);
    expect(siblingImages(null, "/ws/figures/a.png")).toEqual([]);
  });

  it("ignores directories named like images", () => {
    const entries = [dir("plots", "/ws/figures"), file("a.png")];
    expect(siblingImages(entries, "/ws/figures/a.png").map(entry => entry.name)).toEqual(["a.png"]);
  });
});
