import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  clearSessionDraft,
  clearSessionDraftIfMatches,
  loadSessionDraft,
  saveSessionDraft,
} from "../draftStorage";

const mockData = new Map<string, string>();

jest.mock("@react-native-async-storage/async-storage", () => ({
  __esModule: true,
  default: {
    getItem: jest.fn(async (key: string) => mockData.get(key) ?? null),
    setItem: jest.fn(async (key: string, value: string) => {
      mockData.set(key, value);
    }),
    removeItem: jest.fn(async (key: string) => {
      mockData.delete(key);
    }),
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
    mockData.clear();
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

  test("cold acknowledgement clears only the draft that produced it", async () => {
    mockedAsync.getItem.mockResolvedValueOnce(
      JSON.stringify({ version: 1, text: "hello", attachments: [attachment] }),
    );
    await clearSessionDraftIfMatches("s1", { text: "hello", attachments: [attachment] });
    expect(mockedAsync.removeItem).toHaveBeenCalledWith("futureos.remote.draft.v1:s1");

    jest.clearAllMocks();
    mockedAsync.getItem.mockResolvedValueOnce(
      JSON.stringify({ version: 1, text: "newer edit", attachments: [attachment] }),
    );
    await clearSessionDraftIfMatches("s1", { text: "hello", attachments: [attachment] });
    expect(mockedAsync.removeItem).not.toHaveBeenCalled();
  });

  test("cold acknowledgement keeps the draft when the attachments differ", async () => {
    const different = { ...attachment, transferSize: 999 };
    mockedAsync.getItem.mockResolvedValueOnce(
      JSON.stringify({ version: 1, text: "hello", attachments: [attachment] }),
    );
    await clearSessionDraftIfMatches("s1", { text: "hello", attachments: [different] });
    expect(mockedAsync.removeItem).not.toHaveBeenCalled();
  });

  test("cold acknowledgement cannot delete a draft saved concurrently", async () => {
    const original = { text: "hello", attachments: [attachment] };
    await saveSessionDraft("s1", original);
    let releaseRead!: () => void;
    const readStarted = new Promise<void>(resolve => {
      mockedAsync.getItem.mockImplementationOnce(async () => {
        resolve();
        await new Promise<void>(release => {
          releaseRead = release;
        });
        return JSON.stringify({ version: 1, ...original });
      });
    });

    const clearing = clearSessionDraftIfMatches("s1", original);
    await readStarted;
    const newer = { text: "newer edit", attachments: [attachment] };
    const saving = saveSessionDraft("s1", newer);

    expect(mockedAsync.setItem).toHaveBeenCalledTimes(1);
    releaseRead();
    await Promise.all([clearing, saving]);
    await expect(loadSessionDraft("s1")).resolves.toEqual({ version: 1, ...newer });
  });
});
