import { Platform } from "react-native";
import { findSupportedMimeType } from "future-file-handler";
import { supportedExternalMime } from "../fileHandler";

jest.mock("react-native", () => ({
  __esModule: true,
  Platform: {
    OS: "ios",
    select: (specifics: Record<string, unknown>) =>
      specifics?.ios ?? specifics?.native ?? specifics?.default,
  },
  TurboModuleRegistry: {
    get: () => null,
    getEnforcing: () => {
      throw new Error("native module not found");
    },
  },
  NativeEventEmitter: class {},
}));

jest.mock("future-file-handler", () => ({
  __esModule: true,
  findSupportedMimeType: jest.fn(),
}));

const mockedFind = findSupportedMimeType as jest.Mock;

describe("supportedExternalMime", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (Platform as { OS: string }).OS = "ios";
  });

  test("returns null for a name with no external candidates", async () => {
    expect(await supportedExternalMime("program.exe")).toBeNull();
    expect(mockedFind).not.toHaveBeenCalled();
  });

  test("returns the first allow-listed candidate on non-Android platforms", async () => {
    // `.csv` is external with a text/plain fallback → candidates[0] = text/csv.
    expect(await supportedExternalMime("table.csv")).toBe("text/csv");
    expect(mockedFind).not.toHaveBeenCalled();
  });

  test.each([["photo.png", "image/png"], ["notes.md", "text/markdown"], ["data.json", "application/json"]])(
    "also supports explicitly opening previewable %s files",
    async (name, mime) => {
      (Platform as { OS: string }).OS = "android";
      mockedFind.mockResolvedValueOnce(mime);
      await expect(supportedExternalMime(name)).resolves.toBe(mime);
      expect(mockedFind).toHaveBeenCalledWith(name, [mime]);
    },
  );

  test("queries the native intent handler on Android", async () => {
    (Platform as { OS: string }).OS = "android";
    mockedFind.mockResolvedValueOnce("text/csv");
    await expect(supportedExternalMime("table.csv")).resolves.toBe("text/csv");
    expect(mockedFind).toHaveBeenCalledWith("table.csv", ["text/csv", "text/plain"]);
  });
});
