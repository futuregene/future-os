import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Info, X } from "lucide-react-native";
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import type { RemoteSkill } from "../../../remote/types";
import { colors, layout, radius, spacing } from "../../../theme/tokens";
import { filterActions, filterSkills, type SlashAction } from "../skillCompletion";

/** One two-line row: the command, then a single description line. */
export const SKILL_ROW_HEIGHT = 56;
/** A row plus the hairline that separates it from the row above. */
const ROW_PITCH = SKILL_ROW_HEIGHT + 1;
const HEADER_HEIGHT = 44;
const PANEL_BORDER = 1;
/** The menu shows this many skills without scrolling. */
export const VISIBLE_SKILL_ROWS = 3;

/** How tall the `/` menu above the composer is: exactly as tall as the skills it
 * has to show (plus the context actions that lead them), and no taller than the
 * room the keyboard leaves — the conversation keeps the rest. */
export function skillPickerHeight(viewportHeight: number, keyboardHeight: number, actionCount = 0) {
  const rows = VISIBLE_SKILL_ROWS + actionCount;
  const wanted = 2 * PANEL_BORDER + HEADER_HEIGHT + ROW_PITCH * rows;
  const space = viewportHeight - keyboardHeight - 100;
  return Math.max(120, Math.min(wanted, space));
}

/** Mounted only while completing a token. Reopening refreshes installed skills.
 *
 * Every row is two stacked lines — the English command, then the description in
 * the UI language — so the list reads like the desktop ` / ` menu; the info
 * button opens the rest of the description in its own dialog. */
export function SkillPicker({ query, supported, load, onSelect, onClose, onShowDetails, detailsName, maxHeight, actions = [], onActionSelect }: {
  query: string;
  supported: boolean;
  load: () => Promise<RemoteSkill[]>;
  onSelect: (name: string) => void;
  onClose: () => void;
  /** Opens a skill's description; the surface itself lives with the composer. */
  onShowDetails: (skill: RemoteSkill) => void;
  /** Skill whose description is currently on screen, for the row's state. */
  detailsName?: string | null;
  maxHeight: number;
  /** Context actions, rendered above the skills they can be confused with. */
  actions?: SlashAction[];
  /** Runs the chosen action; the picker owns no action behaviour itself. */
  onActionSelect?: (action: SlashAction) => void;
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
  const matchedActions = filterActions(actions, query);
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
        {matchedActions.map(action => (
          <View key={action.id} style={styles.option}>
            <Pressable accessibilityRole="button"
              accessibilityLabel={action.label}
              accessibilityHint={action.description}
              onPress={() => onActionSelect?.(action)}
              style={({ pressed }) => [styles.select, pressed && styles.pressed]}>
              <Text numberOfLines={1} style={styles.name}>{action.label}</Text>
              <Text numberOfLines={1} style={styles.description}>{action.description}</Text>
            </Pressable>
          </View>
        ))}
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
          : matches.length === 0 ? (matchedActions.length > 0
            ? null
            : <Text style={styles.hint}>{t(skills.length ? "skills.noResults" : "skills.empty")}</Text>)
          : matches.map(skill => {
            // Line 1 is always the English command (desktop parity); only the
            // description follows the UI language.
            const description = useZh ? skill.descriptionZh || skill.description : skill.description;
            const expanded = detailsName === skill.name;
            return (
              <View key={skill.name} style={styles.option}>
                <View style={styles.row}>
                  <Pressable accessibilityRole="button"
                    accessibilityLabel={`/${skill.name}${skill.nameZh ? ` · ${skill.nameZh}` : ""}`}
                    accessibilityHint={description}
                    onPress={() => onSelect(skill.name)}
                    style={({ pressed }) => [styles.select, pressed && styles.pressed]}>
                    <Text numberOfLines={1} style={styles.name}>{`/${skill.name}`}</Text>
                    <Text numberOfLines={1} style={styles.description}>{description}</Text>
                  </Pressable>
                  <Pressable accessibilityRole="button"
                    accessibilityLabel={t("skills.details", { name: skill.name })}
                    accessibilityState={{ expanded }}
                    onPress={() => onShowDetails(skill)}
                    style={({ pressed }) => [styles.detailsButton, (pressed || expanded) && styles.pressed]}>
                    <Info color={colors.inkMuted} size={18} />
                  </Pressable>
                </View>
              </View>
            );
          })}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  panel: { marginBottom: spacing.xs, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg,
    backgroundColor: colors.surface, overflow: "hidden" },
  header: { height: HEADER_HEIGHT, flexDirection: "row", alignItems: "center", paddingLeft: spacing.lg },
  title: { flex: 1, fontSize: 13, fontWeight: "600", color: colors.inkSoft },
  close: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  option: { borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.lineSoft },
  row: { flexDirection: "row", alignItems: "center" },
  select: { flex: 1, minWidth: 0, minHeight: SKILL_ROW_HEIGHT, flexDirection: "column", alignItems: "stretch",
    justifyContent: "center", gap: spacing.xs, paddingLeft: spacing.lg, paddingRight: spacing.md, paddingVertical: spacing.sm },
  detailsButton: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  pressed: { backgroundColor: colors.surfaceSubtle },
  name: { fontSize: 14, fontWeight: "600", color: colors.ink },
  description: { fontSize: 12, color: colors.inkMuted },
  hint: { fontSize: 13, color: colors.inkSoft, padding: spacing.md, flexShrink: 1 },
  loading: { flexDirection: "row", alignItems: "center", paddingLeft: spacing.md },
  retry: { minHeight: 44, justifyContent: "center", alignItems: "center" },
  retryText: { color: colors.accent, fontWeight: "600" },
});
