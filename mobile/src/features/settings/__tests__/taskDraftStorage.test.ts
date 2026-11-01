import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  clearTaskDraft,
  loadTaskDraft,
  saveTaskDraft,
  taskDraftKey,
} from "../taskDraftStorage";

/**
 * The task form's draft slot.
 *
 * These tests drive the read path with storage that answers badly, because that
 * is the whole point of the file: a draft is a convenience, so every way the
 * stored blob can be wrong has to come back as "no draft" rather than as a form
 * that throws on open.
 */
describe("taskDraftStorage", () => {
  const key = taskDraftKey("desk_1", "tsk_1");

  beforeEach(async () => {
    await AsyncStorage.clear();
    jest.restoreAllMocks();
  });

  test("keys a draft by desktop and task, and a new task by its own slot", () => {
    expect(key).toBe("futureos.tasks.draft.v1:desk_1:tsk_1");
    expect(taskDraftKey("desk_1", null)).toBe("futureos.tasks.draft.v1:desk_1:create");
  });

  test("round-trips a draft, and forgets it on request", async () => {
    await saveTaskDraft(key, { name: "daily" }, 3);
    const stored = await loadTaskDraft(key);
    expect(stored).toMatchObject({ version: 1, promptVersion: 3, draft: { name: "daily" } });
    expect(typeof stored!.savedAt).toBe("number");

    await clearTaskDraft(key);
    expect(await loadTaskDraft(key)).toBeNull();
  });

  test("reads a draft with no version and no timestamp as the oldest possible one", async () => {
    // A record written by a build before those fields existed: the draft is
    // still good, so it is served with a neutral version instead of dropped.
    await AsyncStorage.setItem(key, JSON.stringify({ version: 1, draft: { name: "x" } }));
    expect(await loadTaskDraft(key)).toMatchObject({ promptVersion: null, savedAt: 0 });
  });

  test("treats storage that cannot be read as no draft", async () => {
    // A locked or broken store must not take the form down with it.
    jest.spyOn(AsyncStorage, "getItem").mockRejectedValueOnce(new Error("database is locked"));
    expect(await loadTaskDraft(key)).toBeNull();
  });

  test("discards a truncated write instead of migrating it", async () => {
    await AsyncStorage.setItem(key, '{"version":1,"draft":{"name"');
    expect(await loadTaskDraft(key)).toBeNull();
  });

  test("discards a record of another version", async () => {
    await AsyncStorage.setItem(key, JSON.stringify({ version: 2, draft: { name: "x" } }));
    expect(await loadTaskDraft(key)).toBeNull();
  });

  test("discards a record whose draft is missing, an array or a scalar", async () => {
    for (const draft of [undefined, [1, 2], "typed too far"]) {
      await AsyncStorage.setItem(key, JSON.stringify({ version: 1, promptVersion: null, draft }));
      expect(await loadTaskDraft(key)).toBeNull();
    }
  });

  test("discards a record that is not an object at all", async () => {
    await AsyncStorage.setItem(key, JSON.stringify(["version", 1]));
    expect(await loadTaskDraft(key)).toBeNull();
  });

  test("a write that cannot land leaves the form usable", async () => {
    jest.spyOn(AsyncStorage, "setItem").mockRejectedValueOnce(new Error("no space left"));
    await expect(saveTaskDraft(key, { name: "x" }, null)).resolves.toBeUndefined();

    jest.spyOn(AsyncStorage, "removeItem").mockRejectedValueOnce(new Error("no space left"));
    await expect(clearTaskDraft(key)).resolves.toBeUndefined();
  });
});
