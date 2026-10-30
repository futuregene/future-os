import { Linking } from "react-native";
import { openPrivacyPolicy, privacyPolicyUrl, PRIVACY_POLICY_URL } from "../legal";

describe("privacy policy link", () => {
  it("points at the production mobile policy page", () => {
    expect(PRIVACY_POLICY_URL).toBe("https://future-os.cn/privacy-mobile.html");
  });

  it("carries the app language so the page opens in the same one", () => {
    expect(privacyPolicyUrl("zh")).toBe("https://future-os.cn/privacy-mobile.html?lang=zh");
    expect(privacyPolicyUrl("zh-Hans")).toContain("lang=zh");
    expect(privacyPolicyUrl("en")).toContain("lang=en");
    // 没识别出来的语言回落到英文，页面本身仍是双语
    expect(privacyPolicyUrl("fr")).toContain("lang=en");
  });

  it("opens the policy in the system browser", async () => {
    const open = jest.spyOn(Linking, "openURL").mockResolvedValue(true as never);
    await openPrivacyPolicy("zh");
    expect(open).toHaveBeenCalledWith("https://future-os.cn/privacy-mobile.html?lang=zh");
    open.mockRestore();
  });

  it("stays silent when the browser cannot be opened", async () => {
    const open = jest.spyOn(Linking, "openURL").mockRejectedValue(new Error("no browser"));
    await expect(openPrivacyPolicy("en")).resolves.toBeUndefined();
    open.mockRestore();
  });
});
