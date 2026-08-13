import AsyncStorage from "@react-native-async-storage/async-storage";
import { clearSessionDraft, loadSessionDraft, saveSessionDraft } from "../draftStorage";

jest.mock("@react-native-async-storage/async-storage", () => ({
  __esModule: true,
  default: {
    getItem: jest.fn(),
    setItem: jest.fn(),
    removeItem: jest.fn(),
  },
}));

jest.mock("expo-file-system", () => {
  // Cache-paths (temporary camera/cache files) read as pruned; keep real
  // picker URIs so attachment round-trips exercise the existence filter.
  const mockCachePrefix = "file:///cache/";
  // Sentinel that makes the File constructor throw, simulating a native
  // File API that is unavailable (e.g. some test/platform environments).
  const mockThrowPrefix = "file:///throws/";
  return {
    __esModule: true,
    File: class {
      uri: string;
      constructor(uri: string) {
        if (uri.startsWith(mockThrowPrefix)) throw new Error("File API unavailable");
        this.uri = uri;
      }
      get exists() {
        return !this.uri.startsWith(mockCachePrefix);
      }
    },
  };
});

const mockedAsync = AsyncStorage as jest.Mocked<typeof AsyncStorage>;

const attachment = {
  localUri: "file:///docs/photo.jpg",
  name: "photo.jpg",
  mimeType: "image/jpeg",
  kind: "image" as const,
  originalSize: 100,
  transferSize: 80,
};

describe("session draft storage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  test("saves and loads a draft with text and attachments", async () => {
    await saveSessionDraft("s1", { text: "hello", attachments: [attachment] });
    expect(mockedAsync.setItem).toHaveBeenCalledWith(
      "futureos.remote.draft.v1:s1",
      JSON.stringify({ version: 1, text: "hello", attachments: [attachment] }),
    );
    mockedAsync.getItem.mockResolvedValueOnce(
      JSON.stringify({ version: 1, text: "hello", attachments: [attachment] }),
    );
    const draft = await loadSessionDraft("s1");
    expect(draft).toEqual({ version: 1, text: "hello", attachments: [attachment] });
  });

  test("empty draft clears the slot instead of writing a blank entry", async () => {
    await saveSessionDraft("s1", { text: "   ", attachments: [] });
    expect(mockedAsync.removeItem).toHaveBeenCalledWith("futureos.remote.draft.v1:s1");
    expect(mockedAsync.setItem).not.toHaveBeenCalled();
  });

  test("stale-version draft is discarded", async () => {
    mockedAsync.getItem.mockResolvedValueOnce(
      JSON.stringify({ version: 999, text: "old", attachments: [] }),
    );
    expect(await loadSessionDraft("s1")).toBeNull();
  });

  test("corrupt JSON is treated as no draft", async () => {
    mockedAsync.getItem.mockResolvedValueOnce("not json{");
    expect(await loadSessionDraft("s1")).toBeNull();
  });

  test("empty session id is a no-op", async () => {
    await saveSessionDraft("", { text: "x", attachments: [] });
    expect(mockedAsync.setItem).not.toHaveBeenCalled();
    await clearSessionDraft("");
    expect(mockedAsync.removeItem).not.toHaveBeenCalled();
    expect(await loadSessionDraft("")).toBeNull();
  });

  test("attachments whose backing file was pruned are dropped", async () => {
    const pruned = { ...attachment, localUri: "file:///cache/pruned.jpg" };
    const live = { ...attachment, localUri: "file:///docs/keep.png", name: "keep.png" };
    mockedAsync.getItem.mockResolvedValueOnce(
      JSON.stringify({ version: 1, text: "x", attachments: [pruned, live] }),
    );
    const draft = await loadSessionDraft("s1");
    expect(draft?.attachments).toEqual([live]);
  });

  test("attachments are kept when the File API is unavailable to verify them", async () => {
    // A native File API that throws must not silently drop a user's pending
    // work — the attachment is kept rather than becoming a dead tap target.
    const unverifiable = { ...attachment, localUri: "file:///throws/photo.jpg" };
    mockedAsync.getItem.mockResolvedValueOnce(
      JSON.stringify({ version: 1, text: "x", attachments: [unverifiable] }),
    );
    const draft = await loadSessionDraft("s1");
    expect(draft?.attachments).toEqual([unverifiable]);
  });

  test("clearSessionDraft removes the stored slot", async () => {
    await clearSessionDraft("s1");
    expect(mockedAsync.removeItem).toHaveBeenCalledWith("futureos.remote.draft.v1:s1");
  });
});
