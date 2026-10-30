import { Linking } from "react-native";

/**
 * 隐私政策页（iOS / Android 通用版）。
 *
 * 固定指向官网的生产域名，不随构建渠道（release / test / nightly）变化：用户
 * 与审核看到的法律文本必须是同一份，测试渠道也没有必要给一份"演示用"的政策页。
 * 页面自带中英双语，用 `?lang=` 与 App 当前语言对齐。
 *
 * 语言由调用方传入（`i18n.language`），这里刻意不 import i18next 实例：法律页
 * 与语言框架解耦，测试环境也不会因为多一次 i18n 初始化而互相干扰。
 */
export const PRIVACY_POLICY_URL = "https://future-os.cn/privacy-mobile.html";

export function privacyPolicyUrl(language: string): string {
  const lang = language.toLowerCase().startsWith("zh") ? "zh" : "en";
  return `${PRIVACY_POLICY_URL}?lang=${lang}`;
}

/**
 * 用系统浏览器打开隐私政策。
 *
 * 打不开浏览器（被家长控制、无默认浏览器等）不是致命错误：这条入口只是让用户
 * 随时能读到政策文本，失败时静默返回，不打断配对与对话。
 */
export async function openPrivacyPolicy(language: string): Promise<void> {
  try {
    await Linking.openURL(privacyPolicyUrl(language));
  } catch {
    // 静默失败：不影响主流程。
  }
}
