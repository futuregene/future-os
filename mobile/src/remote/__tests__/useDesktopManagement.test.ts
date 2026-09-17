import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { RemoteClient } from "../client";
import { useDesktopManagement } from "../useDesktopManagement";

jest.mock("../readPages", () => ({
  requestReadPage: (client: { requestRetry: (command: unknown, session: string) => unknown }, command: unknown, session: string) => client.requestRetry(command, session),
}));

function mount() {
  const request = jest.fn(async () => ({ data: {} }));
  const requestRetry = jest.fn(async () => ({ data: { models: [], skills: [] } }));
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
