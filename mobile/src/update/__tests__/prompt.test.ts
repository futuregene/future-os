import { createInstance, type TFunction } from "i18next";
import { Platform } from "react-native";
import { AppAlert } from "../../components/appAlerts";
import { resources } from "../../i18n/locales";
import { promptUpgrade } from "../prompt";
import { installUpdate, type UpdateStatus } from "../update";

jest.mock("../update", () => ({
  ...jest.requireActual("../update"),
  installUpdate: jest.fn(async () => {}),
}));
const status: UpdateStatus = {
  currentVersion: "0.0.2-5c2ec56", latestVersion: "0.0.2-70+nightly",
  hasUpdate: true, appStoreUrl: null, downloadUrl: "https://dl.future-os.cn/update.apk", canInstallInApp: false,
};
let t: TFunction;
const platform = Platform.OS;
beforeEach(async () => {
  jest.clearAllMocks();
  jest.spyOn(AppAlert, "alert").mockImplementation(() => {});
  const i18n = createInstance();
  await i18n.init({ lng: "zh", resources, interpolation: { escapeValue: false } });
  t = i18n.t;
  Platform.OS = "android";
});
afterEach(() => { Platform.OS = platform; jest.restoreAllMocks(); });

test("legacy development builds show channels and do not claim the nightly is newer", () => {
  promptUpgrade(status, t);
  const [, message, buttons] = jest.mocked(AppAlert.alert).mock.calls[0]!;
  expect(message).toContain("可下载构建：0.0.2 · 每夜构建 (70)\n当前构建：0.0.2 · 开发构建 (5c2ec56)");
  expect(message).toContain("无法与每夜构建直接比较新旧");
  expect(message).not.toContain("新版本");
  expect(buttons?.map(button => button.text)).toEqual(["稍后再说", "前往下载"]);
  expect(installUpdate).not.toHaveBeenCalled();
  buttons?.[0]?.onPress?.();
  expect(installUpdate).not.toHaveBeenCalled();
  buttons?.[1]?.onPress?.();
  expect(installUpdate).toHaveBeenCalledWith(status);
});

test.each([
  ["0.0.2-abc1234+dev", "开发构建 (abc1234)"],
  ["0.0.2-abc1234+local", "本地构建 (abc1234)"],
  ["0.0.2-abc1234+local.dirty", "本地构建，含未提交修改 (abc1234)"],
  ["0.0.2-69+test", "测试构建 (69)"],
  ["0.0.2-69+nightly", "每夜构建 (69)"],
  ["0.0.2-custom+unknown", "0.0.2-custom+unknown"],
])("preserves build identity for %s", (currentVersion, label) => {
  promptUpgrade({ ...status, currentVersion }, t);
  expect(jest.mocked(AppAlert.alert).mock.calls[0]?.[1]).toContain(label);
});

test("Android installable updates clearly describe downloading and installation", () => {
  promptUpgrade({ ...status, currentVersion: "1.0.0", latestVersion: "1.1.0", canInstallInApp: true }, t);
  const [, message, buttons] = jest.mocked(AppAlert.alert).mock.calls[0]!;
  expect(message).toContain("新版本：1.1.0\n当前版本：1.0.0");
  expect(message).toContain("系统安装界面");
  expect(buttons?.[1]?.text).toBe("下载并安装");
});

test("iOS describes the App Store handoff rather than an APK download", () => {
  Platform.OS = "ios";
  promptUpgrade({ ...status, currentVersion: "1.0.0", latestVersion: "1.1.0", appStoreUrl: "https://apps.apple.com/app/futureos", canInstallInApp: true }, t);
  const [, message, buttons] = jest.mocked(AppAlert.alert).mock.calls[0]!;
  expect(message).toContain("请前往 App Store 完成更新");
  expect(buttons?.[1]?.text).toBe("前往 App Store");
});

test("installation failure still presents an actionable error", async () => {
  jest.mocked(installUpdate).mockRejectedValueOnce(new Error("offline"));
  promptUpgrade(status, t);
  jest.mocked(AppAlert.alert).mock.calls[0]?.[2]?.[1]?.onPress?.();
  await Promise.resolve();
  expect(AppAlert.alert).toHaveBeenLastCalledWith("软件更新", "下载或安装失败，请稍后重试。");
});
