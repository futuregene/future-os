import { DEFAULT_APPEARANCE, getAppearance, initializeAppearance, parseAppearance, setAppearance } from "../preferences";

const mockGetItem = jest.fn(async (_key: string): Promise<string | null> => null);
const mockSetItem = jest.fn(async (_key: string, _value: string) => {});
jest.mock("@react-native-async-storage/async-storage", () => ({
  getItem: (key: string) => mockGetItem(key),
  setItem: (key: string, value: string) => mockSetItem(key, value),
}));

/** A module instance of its own, whose one-shot restore has not run yet. */
function freshModule(): typeof import("../preferences") {
  let fresh!: typeof import("../preferences");
  jest.isolateModules(() => {
    // The restore is one-shot per module instance, so reading it again has to
    // run the module again. `import()` needs --experimental-vm-modules here.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    fresh = require("../preferences");
  });
  return fresh;
}

beforeEach(async () => {
  mockGetItem.mockReset().mockResolvedValue(null);
  mockSetItem.mockReset().mockResolvedValue(undefined);
  await initializeAppearance();
  await setAppearance(DEFAULT_APPEARANCE);
  mockGetItem.mockClear();
  mockSetItem.mockClear();
});

describe("reading a stored appearance", () => {
  it("is the default when nothing is stored", () => {
    expect(parseAppearance(null)).toEqual({ themeMode: "system", fontSize: "medium" });
  });

  it("reads a stored record back", () => {
    expect(parseAppearance('{"themeMode":"dark","fontSize":"xlarge"}')).toEqual({ themeMode: "dark", fontSize: "xlarge" });
  });

  it("falls back field by field, so one bad value cannot cost the other", () => {
    expect(parseAppearance('{"themeMode":"dark","fontSize":"huge"}')).toEqual({ themeMode: "dark", fontSize: "medium" });
    expect(parseAppearance('{"themeMode":"sepia","fontSize":"large"}')).toEqual({ themeMode: "system", fontSize: "large" });
    // Not our JSON, and not even an object.
    expect(parseAppearance("{")).toEqual(DEFAULT_APPEARANCE);
    expect(parseAppearance('"dark"')).toEqual(DEFAULT_APPEARANCE);
    expect(parseAppearance("null")).toEqual(DEFAULT_APPEARANCE);
    expect(parseAppearance('{"fontSize":null}')).toEqual(DEFAULT_APPEARANCE);
  });
});

describe("restoring on launch", () => {
  it("reads the stored record once, and applies it", async () => {
    const fresh = freshModule();
    expect(fresh.getAppearance()).toEqual(DEFAULT_APPEARANCE);
    mockGetItem.mockResolvedValue('{"themeMode":"dark","fontSize":"small"}');
    await fresh.initializeAppearance();
    await fresh.initializeAppearance();
    expect(mockGetItem).toHaveBeenCalledTimes(1);
    expect(mockGetItem).toHaveBeenCalledWith("future.mobile.appearance");
    expect(fresh.getAppearance()).toEqual({ themeMode: "dark", fontSize: "small" });
  });

  it("starts in the defaults when storage is unavailable", async () => {
    const fresh = freshModule();
    mockGetItem.mockRejectedValue(new Error("no storage"));
    await fresh.initializeAppearance();
    expect(fresh.getAppearance()).toEqual(DEFAULT_APPEARANCE);
  });
});

describe("saving a choice", () => {
  it("writes the whole record and applies it", async () => {
    await setAppearance({ themeMode: "dark" });
    expect(mockSetItem).toHaveBeenCalledWith("future.mobile.appearance", JSON.stringify({ themeMode: "dark", fontSize: "medium" }));
    expect(getAppearance()).toEqual({ themeMode: "dark", fontSize: "medium" });
    await setAppearance({ fontSize: "large" });
    expect(mockSetItem).toHaveBeenLastCalledWith("future.mobile.appearance", JSON.stringify({ themeMode: "dark", fontSize: "large" }));
    expect(getAppearance()).toEqual({ themeMode: "dark", fontSize: "large" });
  });

  it("keeps the previous value when the write is refused", async () => {
    await setAppearance({ fontSize: "xlarge" });
    mockSetItem.mockRejectedValueOnce(new Error("full"));
    await expect(setAppearance({ themeMode: "light" })).rejects.toThrow("full");
    expect(getAppearance()).toEqual({ themeMode: "system", fontSize: "xlarge" });
  });
});
