import type { GalleryImage } from "../imageGallery";
import { createImageGallery, immediateGalleryUri } from "../imageGallery";

const picture = (id: string, overrides: Partial<GalleryImage> = {}): GalleryImage => ({
  id,
  name: `${id}.png`,
  src: `assets/${id}.png`,
  uri: null,
  ...overrides,
});

test("keeps the order the reply drew its pictures, even when one resolves late", () => {
  const gallery = createImageGallery();
  gallery.upsert(picture("first"));
  gallery.upsert(picture("second"));
  // The second picture's bytes arrive before the first's: updating the same id
  // must not move it to the end, or the viewer would page in load order.
  gallery.upsert(picture("second", { uri: "file:///second.png" }));
  gallery.upsert(picture("first", { uri: "file:///first.png" }));
  expect(gallery.snapshot().map(image => image.id)).toEqual(["first", "second"]);
  expect(gallery.snapshot().map(image => image.uri)).toEqual(["file:///first.png", "file:///second.png"]);
});

test("a picture that leaves the reply leaves the sequence", () => {
  const gallery = createImageGallery();
  gallery.upsert(picture("first"));
  gallery.upsert(picture("second"));
  gallery.remove("first");
  expect(gallery.snapshot().map(image => image.id)).toEqual(["second"]);
});

test("an entry draws its loaded bytes, or a remote URL it already names", () => {
  expect(immediateGalleryUri(picture("local"))).toBeNull();
  expect(immediateGalleryUri(picture("local", { uri: "file:///local.png" }))).toBe("file:///local.png");
  expect(immediateGalleryUri(picture("remote", { src: "https://example.com/a.png" }))).toBe("https://example.com/a.png");
});
