import { useState } from "react";
import { Image, Linking, ScrollView, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import * as Clipboard from "expo-clipboard";
import { Button } from "../../components/Button";
import { AppAlert as Alert } from "../../components/appAlerts";
import { colors, radius, spacing } from "../../theme/tokens";
import { SettingsSection, settingsStyles } from "./SettingsPrimitives";

/** The account to search for, and the article this entry links to. The QR asset
 * encodes the account's own follow payload (`weixin.qq.com/r/mp/...`). */
const ACCOUNT_NAME = "FutureOS";
const FIRST_ARTICLE_URL = "https://mp.weixin.qq.com/s/qefitj15rYRhK6lYmTj2dA";
const QR_CODE = require("../../../assets/wechat-qrcode.png");

/**
 * "Follow our official account", reached from Settings → This phone.
 *
 * Deliberately independent of the paired desktop: following an account needs no
 * connection, so never gate this screen on `desktopOnline`.
 *
 * Copying the name is the primary action here rather than the QR code. A phone
 * cannot scan its own screen, and WeChat only recognises a QR from inside a chat,
 * so pasting the name into WeChat's search box is the one route that works with
 * no second device. The QR stays for the case where someone else scans this
 * screen.
 */
export function FollowAccountPage() {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  async function copyName() {
    try {
      await Clipboard.setStringAsync(ACCOUNT_NAME);
      setCopied(true);
    }
    catch (error) {
      Alert.alert(t("common.error"), error instanceof Error ? error.message : String(error));
    }
  }

  async function readArticle() {
    try {
      await Linking.openURL(FIRST_ARTICLE_URL);
    }
    catch (error) {
      Alert.alert(t("common.error"), error instanceof Error ? error.message : String(error));
    }
  }

  return <ScrollView contentContainerStyle={settingsStyles.content}>
    <SettingsSection title={t("desktopSettings.followAccount")}>
      <View style={styles.card}>
        <Text style={settingsStyles.description}>
          {t("desktopSettings.followAccountDescription", { name: ACCOUNT_NAME })}
        </Text>
        <View style={styles.qrPlate}>
          <Image
            accessibilityLabel={t("desktopSettings.followAccountQrAlt", { name: ACCOUNT_NAME })}
            source={QR_CODE}
            style={styles.qr}
          />
        </View>
        <Text style={settingsStyles.description}>
          {t("desktopSettings.followAccountScanHint", { name: ACCOUNT_NAME })}
        </Text>
        <Text style={settingsStyles.description}>
          {t("desktopSettings.followAccountSearchHint", { name: ACCOUNT_NAME })}
        </Text>
        <View style={settingsStyles.actions}>
          <Button compact label={copied ? t("desktopSettings.followAccountCopied") : t("desktopSettings.followAccountCopyName")} onPress={() => void copyName()} />
          <Button compact label={t("desktopSettings.followAccountReadArticle")} onPress={() => void readArticle()} variant="secondary" />
        </View>
      </View>
    </SettingsSection>
  </ScrollView>;
}

const styles = StyleSheet.create({
  card: { padding: spacing.md, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg, backgroundColor: colors.surface, gap: spacing.md },
  // A white plate keeps the code off the card colour and preserves the quiet zone
  // the asset ships with — a plain code depends on that margin.
  qrPlate: { alignSelf: "center", padding: spacing.sm, backgroundColor: "#FFFFFF", borderRadius: radius.md },
  qr: { width: 176, height: 176 },
});
