import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react-native";
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import type { RemoteSkill } from "../../../remote/types";
import { colors, radius, spacing } from "../../../theme/tokens";
import { filterSkills } from "../skillCompletion";

/** Mounted only while completing a token. Reopening refreshes installed skills. */
export function SkillPicker({ query, supported, load, onSelect, onClose, maxHeight }: {
  query: string;
  supported: boolean;
  load: () => Promise<RemoteSkill[]>;
  onSelect: (name: string) => void;
  onClose: () => void;
  maxHeight: number;
}) {
  const { t, i18n } = useTranslation();
  const [skills, setSkills] = useState<RemoteSkill[]>([]);
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!supported) return;
    let cancelled = false;
    void load().then(result => {
      if (cancelled) return;
      setSkills(result);
      setStatus("ready");
    }).catch(() => {
      if (!cancelled) setStatus("error");
    });
    return () => { cancelled = true; };
  }, [load, supported, attempt]);
  const matches = filterSkills(skills, query);
  const useZh = i18n.language.startsWith("zh");
  return (
    <View style={[styles.panel, { maxHeight }]}>
      <View style={styles.header}>
        <Text accessibilityRole="header" style={styles.title}>{t("skills.title")}</Text>
        <Pressable accessibilityRole="button" accessibilityLabel={t("skills.close")}
          onPress={onClose} style={styles.close}>
          <X color={colors.inkSoft} size={18} />
        </Pressable>
      </View>
      <ScrollView keyboardShouldPersistTaps="always" bounces={false}>
        {!supported ? <Text style={styles.hint}>{t("skills.updateDesktop")}</Text>
          : status === "loading" ? <View style={styles.loading}>
            <ActivityIndicator color={colors.accent} />
            <Text style={styles.hint}>{t("skills.loading")}</Text>
          </View>
          : status === "error" ? <View>
            <Text accessibilityRole="alert" style={styles.hint}>{t("skills.loadFailed")}</Text>
            <Pressable accessibilityRole="button" accessibilityLabel={t("common.retry")} onPress={() => { setStatus("loading"); setAttempt(value => value + 1); }} style={styles.retry}>
              <Text style={styles.retryText}>{t("common.retry")}</Text>
            </Pressable>
          </View>
          : matches.length === 0 ? <Text style={styles.hint}>{t(skills.length ? "skills.noResults" : "skills.empty")}</Text>
          : matches.map(skill => (
            <Pressable key={skill.name} accessibilityRole="button"
              accessibilityLabel={`/${skill.name}${skill.nameZh ? ` · ${skill.nameZh}` : ""}`}
              onPress={() => onSelect(skill.name)}
              style={({ pressed }) => [styles.option, pressed && styles.pressed]}>
              <Text numberOfLines={1} style={styles.name}>{useZh && skill.nameZh ? skill.nameZh : skill.name}</Text>
              <Text numberOfLines={1} style={styles.command}>/{skill.name}</Text>
              <Text numberOfLines={1} style={styles.description}>
                {useZh ? skill.descriptionZh || skill.description : skill.description}
              </Text>
            </Pressable>
          ))}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  panel: { marginBottom: spacing.xs, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg,
    backgroundColor: colors.surface, overflow: "hidden" },
  header: { flexDirection: "row", alignItems: "center", paddingLeft: spacing.lg },
  title: { flex: 1, fontSize: 13, fontWeight: "600", color: colors.inkSoft },
  close: { width: 44, height: 44, alignItems: "center", justifyContent: "center" },
  option: { minHeight: 64, paddingHorizontal: spacing.lg, paddingVertical: spacing.sm,
    borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.lineSoft },
  pressed: { backgroundColor: colors.surfaceSubtle },
  name: { fontSize: 14, fontWeight: "600", color: colors.ink },
  command: { fontSize: 11, color: colors.inkMuted },
  description: { fontSize: 12, color: colors.inkSoft },
  hint: { fontSize: 13, color: colors.inkSoft, padding: spacing.md, flexShrink: 1 },
  loading: { flexDirection: "row", alignItems: "center", paddingLeft: spacing.md },
  retry: { minHeight: 44, justifyContent: "center", alignItems: "center" },
  retryText: { color: colors.accent, fontWeight: "600" },
});
