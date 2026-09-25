import { createElement } from "react";
import { Image, Linking } from "react-native";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import * as Clipboard from "expo-clipboard";
import { Button } from "../../../components/Button";
import { FollowAccountPage } from "../FollowAccountPage";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { name?: string }) =>
      options?.name ? `${key}: ${options.name}` : key,
    i18n: { get language() { return "en"; } },
  }),
}));
jest.mock("../../../components/appAlerts", () => ({ AppAlert: { alert: jest.fn() } }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
// SettingsPrimitives pulls in lucide-react-native, which ships ESM that jest does
// not transform — the same mock the other settings suites use.
jest.mock("lucide-react-native", () => ({ ChevronRight: "ChevronRight" }));

let tree: ReactTestRenderer;

beforeEach(async () => {
  jest.clearAllMocks();
  jest.spyOn(Clipboard, "setStringAsync").mockResolvedValue(true);
  jest.spyOn(Linking, "openURL").mockResolvedValue(true);
  await act(async () => { tree = create(createElement(FollowAccountPage)); });
});

const button = (label: string) => tree.root.findAllByType(Button).find(node => node.props.label === label);

test("copies exactly the account name, which is what WeChat's search box needs", async () => {
  const copy = button("desktopSettings.followAccountCopyName")!;
  expect(copy).toBeDefined();
  await act(async () => copy.props.onPress());
  expect(Clipboard.setStringAsync).toHaveBeenCalledTimes(1);
  // Exactly the account name — what WeChat's search box needs, not the URL and
  // not a prefixed string.
  expect(Clipboard.setStringAsync).toHaveBeenCalledWith("FutureOS");
  // Read-back confirms the label switched, so the user gets feedback the paste
  // will work rather than tapping again.
  await act(async () => {});
  expect(button("desktopSettings.followAccountCopied")).toBeDefined();
});

test("opens the published article", async () => {
  await act(async () => button("desktopSettings.followAccountReadArticle")!.props.onPress());
  expect(Linking.openURL).toHaveBeenCalledTimes(1);
  expect(Linking.openURL).toHaveBeenCalledWith("https://mp.weixin.qq.com/s/qefitj15rYRhK6lYmTj2dA");
});

test("renders the account QR code with an accessible label", () => {
  const images = tree.root.findAllByType(Image);
  expect(images).toHaveLength(1);
  // A broken asset would render nothing scannable, so assert the source resolved.
  expect(images[0]!.props.source).toBeTruthy();
  expect(images[0]!.props.accessibilityLabel).toContain("FutureOS");
});
