import type { MobileAttachment } from "../../../remote/types";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { loadSendQueue, saveSendQueue } from "../sendQueuePersistence";

/** Files the fake filesystem still holds (an OS cache cleanup removes them). */
const mockExisting = new Set<string>();

jest.mock("expo-file-system", () => ({
  File: class {
    uri: string;
    constructor(uri: string) {
      this.uri = uri;
    }

    get exists(): boolean {
      return mockExisting.has(this.uri);
    }
  },
}));

function attachment(localUri: string): MobileAttachment {
  return {
    kind: "image",
    localUri,
    mimeType: "image/png",
    name: localUri.split("/").pop() ?? "file.png",
    originalSize: 10,
    transferSize: 10,
  };
}

const KEY = "futureos.remote.send-queue.v1";

beforeEach(() => {
  mockExisting.clear();
  AsyncStorage.clear();
});

test("a queue written before the app closed comes back with its sentences", async () => {
  await saveSendQueue(new Map([
    ["desktop:s1", [{ attachments: [], content: "held one", createdAt: 1, id: "q1" }]],
  ]));
  const loaded = await loadSendQueue();
  expect(loaded.get("desktop:s1")?.map(item => item.content)).toEqual(["held one"]);
});

test("attachments the OS cleaned up are dropped, and an item left with nothing goes with them", async () => {
  mockExisting.add("file:///kept.png");
  await saveSendQueue(new Map([
    ["desktop:s1", [
      { attachments: [attachment("file:///kept.png")], content: "with a kept file", createdAt: 1, id: "q1" },
      { attachments: [attachment("file:///gone.png")], content: "with a lost file", createdAt: 2, id: "q2" },
      { attachments: [attachment("file:///gone.png")], content: "   ", createdAt: 3, id: "q3" },
    ]],
  ]));
  const loaded = await loadSendQueue();
  const items = loaded.get("desktop:s1") ?? [];
  // The words survive even when their file does not; the empty one does not.
  expect(items.map(item => item.content)).toEqual(["with a kept file", "with a lost file"]);
  expect(items[0]!.attachments).toHaveLength(1);
  expect(items[1]!.attachments).toHaveLength(0);
});

test("a record from another shape is ignored rather than half-read", async () => {
  await AsyncStorage.setItem(KEY, JSON.stringify({ version: 99, conversations: { "desktop:s1": [] } }));
  expect((await loadSendQueue()).size).toBe(0);
  await AsyncStorage.setItem(KEY, "not json");
  expect((await loadSendQueue()).size).toBe(0);
});

test("items that are not a message are dropped on the way in", async () => {
  await AsyncStorage.setItem(KEY, JSON.stringify({
    version: 1,
    conversations: {
      "desktop:s1": [
        { content: "no id", createdAt: 1, attachments: [] },
        { id: "q1", content: "no timestamp", attachments: [] },
        { id: "q2", content: "bad attachment", createdAt: 2, attachments: [{ localUri: "x" }] },
        { id: "q3", content: "good", createdAt: 3, attachments: [] },
      ],
    },
  }));
  const loaded = await loadSendQueue();
  expect(loaded.get("desktop:s1")?.map(item => item.content)).toEqual(["good"]);
});

test("an empty conversation is not written at all", async () => {
  await saveSendQueue(new Map([["desktop:s1", []]]));
  const raw = await AsyncStorage.getItem(KEY);
  expect(JSON.parse(raw!).conversations).toEqual({});
});

test("the command id a message was sent under survives the app being killed", async () => {
  // A phone that comes back with the message still queued and a *fresh* command id
  // would send the words their desktop may already have: the id is the only thing
  // that makes the two the same message, so it is part of what is stored.
  await saveSendQueue(new Map([
    ["desktop:s1", [
      { attachments: [], commandId: "prompt_abc", content: "already sent", createdAt: 1, id: "q1" },
    ]],
  ]));
  const loaded = await loadSendQueue();
  expect(loaded.get("desktop:s1")?.[0]?.commandId).toBe("prompt_abc");
});

test("a row with no command id reads back as one", async () => {
  await saveSendQueue(new Map([
    ["desktop:s1", [{ attachments: [], content: "never sent", createdAt: 1, id: "q1" }]],
  ]));
  const loaded = await loadSendQueue();
  expect(loaded.get("desktop:s1")?.[0]).not.toHaveProperty("commandId");
});
