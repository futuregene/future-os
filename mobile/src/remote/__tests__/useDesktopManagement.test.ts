import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { RemoteClient } from "../client";
import { useDesktopManagement } from "../useDesktopManagement";

jest.mock("../readPages", () => ({
  requestReadPage: (client: { requestRetry: (command: unknown, session: string) => unknown }, command: unknown, session: string) => client.requestRetry(command, session),
}));

function mount() {
  // Untyped jest mocks: the seam under test is "which command and which lane",
  // and asserting that through a narrower fake would fight the real signature.
  const request: jest.Mock = jest.fn(async () => ({ data: {} }));
  const requestRetry: jest.Mock = jest.fn(async () => ({ data: {} }));
  const client = { request, requestRetry, accessIdentity: "bridge-one" };
  const ref = { current: client as unknown as RemoteClient | null };
  let api!: ReturnType<typeof useDesktopManagement>;
  function Host() { api = useDesktopManagement(ref); return null; }
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(Host)); });
  return { client, ref, api, tree, request, requestRetry };
}

test("settings and skill changes are commands to the desktop, not phone preferences", async () => {
  const h = mount();
  try {
    await h.api.getDesktopSettings();
    expect(h.requestRetry).toHaveBeenCalledWith({ type: "get_desktop_settings" }, "settings");
    await h.api.updateDesktopSettings({ autoTitleFirstTurn: true, hiddenModels: ["provider/model"] });
    expect(h.request).toHaveBeenCalledWith({ type: "update_desktop_settings", settings: { autoTitleFirstTurn: true, hiddenModels: ["provider/model"] } }, "settings", 60_000);
    await h.api.installSkill("research", "1.2.0");
    expect(h.request).toHaveBeenCalledWith({ type: "install_skill", skillId: "research", version: "1.2.0" }, "settings", 60_000);
    await h.api.uninstallSkill("research");
    expect(h.request).toHaveBeenCalledWith({ type: "uninstall_skill", skillId: "research" }, "settings", 60_000);
    await h.api.listSettingsModels();
    expect(h.requestRetry).toHaveBeenCalledWith({ type: "list_settings_models" }, "settings");
  } finally { act(() => h.tree.unmount()); }
});

test.each(["desktop", "bridge"])("rejects a late response after the %s identity changes", async kind => {
  const h = mount();
  let finish!: (value: { data: object }) => void;
  h.request.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const pending = h.api.updateDesktopSettings({ autoConnectRemote: true });
  const rejected = expect(pending).rejects.toThrow("stale_desktop");
  if (kind === "desktop") h.ref.current = null;
  else h.client.accessIdentity = "bridge-two";
  finish({ data: {} });
  await rejected;
  act(() => h.tree.unmount());
});

test("offline writes fail immediately and are never queued", async () => {
  const h = mount();
  h.ref.current = null;
  await expect(h.api.updateDesktopSettings({ autoUpgradeSkills: true })).rejects.toThrow("not_connected");
  expect(h.request).not.toHaveBeenCalled();
  act(() => h.tree.unmount());
});

test("the read surfaces unwrap their own field from the desktop's reply", async () => {
  const h = mount();
  h.requestRetry.mockImplementation(async (command: { type: string }) => {
    if (command.type === "list_skills") return { data: { skills: [{ id: "research" }] } };
    if (command.type === "list_available_skills") return { data: { skills: [{ id: "future-web" }] } };
    if (command.type === "list_settings_models") return { data: { models: [{ id: "m" }] } };
    if (command.type === "suggest_skill") return { data: { skill: { name: "future-web", description: "web" } } };
    if (command.type === "skill_reco_today") {
      return { data: { today: { count: 2, skillIds: ["future-web"], messageHashes: ["h"] } } };
    }
    return { data: {} };
  });
  try {
    // Each surface hands the caller the field it needs, not the envelope: a
    // caller that had to know the wire shape would break on every refactor.
    await expect(h.api.listInstalledSkills()).resolves.toEqual([{ id: "research" }]);
    await expect(h.api.listAvailableSkills()).resolves.toEqual([{ id: "future-web" }]);
    await expect(h.api.listSettingsModels()).resolves.toEqual([{ id: "m" }]);
    await expect(h.api.suggestSkill("find the web tool", [{ name: "future-web", description: "web" }]))
      .resolves.toEqual({ name: "future-web", description: "web" });
    expect(h.requestRetry).toHaveBeenCalledWith(
      { candidates: [{ name: "future-web", description: "web" }], query: "find the web tool", type: "suggest_skill" },
      "settings",
    );
    await expect(h.api.skillRecoToday()).resolves.toEqual({
      count: 2, skillIds: ["future-web"], messageHashes: ["h"],
    });
  } finally {
    act(() => h.tree.unmount());
  }
});

test("a declined recommendation is null rather than an absent field", async () => {
  const h = mount();
  h.requestRetry.mockResolvedValueOnce({ data: { skill: null } });
  try {
    await expect(h.api.suggestSkill("q", [])).resolves.toBeNull();
  } finally {
    act(() => h.tree.unmount());
  }
});

test("the provider surfaces read and write through the same command seam", async () => {
  const h = mount();
  h.request.mockImplementation(async () => ({ data: { builtin: [], custom: [] } }));
  h.requestRetry.mockImplementation(async (command: { type: string }) =>
    command.type === "list_providers" ? { data: { builtin: [], custom: [] } } : { data: {} });
  try {
    await expect(h.api.listProviders()).resolves.toEqual({ builtin: [], custom: [] });
    // Listing is a read: it must not claim the mutation lane or a 60 s budget.
    expect(h.requestRetry).toHaveBeenCalledWith({ type: "list_providers" }, "settings");

    const provider = { id: "future", apiKey: "k", baseUrl: "https://x" } as never;
    await h.api.updateBuiltinProvider(provider);
    expect(h.request).toHaveBeenCalledWith(
      { provider, type: "update_builtin_provider" }, "settings", 60_000);
    await h.api.upsertCustomProvider({ id: "acme" } as never);
    expect(h.request).toHaveBeenCalledWith(
      { provider: { id: "acme" }, type: "upsert_custom_provider" }, "settings", 60_000);
    await h.api.deleteCustomProvider("acme");
    expect(h.request).toHaveBeenCalledWith(
      { providerId: "acme", type: "delete_custom_provider" }, "settings", 60_000);

    // recordSkillReco is a best-effort write: its reply is discarded, not
    // returned to the caller.
    await h.api.recordSkillReco("future-web", "hash");
    expect(h.request).toHaveBeenCalledWith(
      { messageHash: "hash", skillId: "future-web", type: "record_skill_reco" }, "settings", 60_000);
  } finally {
    act(() => h.tree.unmount());
  }
});

