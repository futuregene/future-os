import { useState } from "react";
import { Modal, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../components/Button";
import { DialogSurface } from "../components/DialogSurface";
import { PeerIconGlyph } from "../remote/PeerIconGlyph";
import { PEER_ICONS, peerIconId } from "../remote/peerIcons";
import { useRemoteControls } from "../remote/RemoteContext";
import type { PairedDesktop } from "../remote/types";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";

/** How long a name this phone keeps. The row shows it in one line, so a longer
 * label would only be an ellipsis in every list it appears in. */
const MAX_NAME_LENGTH = 40;

/**
 * A paired desktop's name and icon, edited from the row that shows them.
 *
 * Both are this phone's own record of the machine: `labelDesktop` writes the
 * local credential book and nothing goes over the wire. That is why the editor
 * belongs here and not on the machine's settings page, whose every other row is
 * read from or written to that machine — there the label read as one more
 * remote setting, and on a machine that was not answering it was the one thing
 * a user wants (call it something recognisable) that could not be changed.
 *
 * One patch on save, carrying only what changed: `labelDesktop` treats an
 * omitted key as "leave it". A blank name clears the label, so the row falls
 * back to the machine's id, and the icon grid has no "none" entry for the same
 * reason — the default mark is drawn by the default id, so picking it is how a
 * chosen icon is dropped.
 */
export function DesktopLabelDialog({ desktop, onClose }: {
  /** The machine this editor names; its stored label is what the fields open on. */
  desktop: PairedDesktop;
  onClose(): void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const remote = useRemoteControls();
  // The effective id, not the stored one: `null` and an id from a newer build
  // both draw as the default glyph, and the grid has to mark the mark the row is
  // showing.
  const [icon, setIcon] = useState(() => peerIconId(desktop.icon));
  const [name, setName] = useState(desktop.name ?? "");
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  const nameChanged = name.trim() !== (desktop.name ?? "");
  const iconChanged = icon !== peerIconId(desktop.icon);

  const close = () => { if (!saving) onClose(); };

  const save = async () => {
    if (saving || (!nameChanged && !iconChanged)) return;
    setSaving(true);
    setFailed(false);
    try {
      await remote.labelDesktop(desktop.desktopId, {
        ...(nameChanged ? { name: name.trim() } : {}),
        ...(iconChanged ? { icon } : {}),
      });
      // The list is the provider's, so the write is made visible by its re-read;
      // this surface has nothing left to say once it has been accepted.
      onClose();
    } catch {
      // The book is local and can still refuse (read-only, locked): say so
      // instead of closing on a write that did not happen.
      setFailed(true);
      setSaving(false);
    }
  };

  return (
    <Modal animationType="fade" onRequestClose={close} transparent visible>
      <DialogSurface
        footer={(
          <View style={styles.actions}>
            <View style={styles.action}>
              <Button compact disabled={saving} label={t("desktops.cancel")} onPress={close} variant="secondary" />
            </View>
            <View style={styles.action}>
              <Button
                compact
                disabled={saving || (!nameChanged && !iconChanged)}
                label={t(saving ? "desktops.saving" : "desktops.save")}
                onPress={() => void save()}
              />
            </View>
          </View>
        )}
      >
        <Text accessibilityRole="header" style={styles.title}>{t("desktops.editTitle")}</Text>
        <View style={styles.field}>
          <TextInput
            accessibilityLabel={t("desktops.name")}
            autoCapitalize="none"
            autoCorrect={false}
            editable={!saving}
            maxLength={MAX_NAME_LENGTH}
            onChangeText={setName}
            onSubmitEditing={() => void save()}
            placeholder={desktop.desktopId}
            placeholderTextColor={colors.inkMuted}
            returnKeyType="done"
            selectTextOnFocus
            style={styles.input}
            value={name}
          />
          <Text style={styles.hint}>{t("desktops.nameHint")}</Text>
        </View>
        <View style={styles.field}>
          <Text style={styles.label}>{t("desktops.iconLabel")}</Text>
          {/* Each tile draws its icon in that icon's own colour — the same
              palette the desktop picker uses — and the chosen one is marked by
              its frame. Recolouring the glyph would hide the colour the choice
              is about. */}
          <View accessibilityRole="radiogroup" style={styles.iconGrid}>
            {PEER_ICONS.map(option => {
              const selected = option.id === icon;
              return (
                <Pressable
                  accessibilityLabel={t(`desktops.icons.${option.labelKey}`)}
                  accessibilityRole="radio"
                  accessibilityState={{ checked: selected, disabled: saving }}
                  disabled={saving}
                  key={option.id}
                  onPress={() => setIcon(option.id)}
                  style={({ pressed }) => [
                    styles.iconOption,
                    selected && styles.iconOptionSelected,
                    pressed && styles.pressed,
                  ]}
                >
                  <PeerIconGlyph id={option.id} size={20} />
                </Pressable>
              );
            })}
          </View>
          <Text style={styles.hint}>{t("desktops.iconHint")}</Text>
        </View>
        {failed ? <Text accessibilityRole="alert" style={styles.error}>{t("desktops.actionFailed")}</Text> : null}
      </DialogSurface>
    </Modal>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  title: { color: theme.colors.inkStrong, fontSize: theme.font(20), fontWeight: "700" },
  field: { gap: spacing.xs },
  label: { color: theme.colors.inkSoft, fontSize: theme.font(13), fontWeight: "600" },
  input: {
    minHeight: 48,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    fontSize: theme.font(15),
    color: theme.colors.ink,
  },
  hint: { color: theme.colors.inkMuted, fontSize: theme.font(12), lineHeight: theme.leading(18) },
  iconGrid: { flexDirection: "row", flexWrap: "wrap", gap: spacing.sm },
  iconOption: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surface,
  },
  iconOptionSelected: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  error: { color: theme.colors.danger, fontSize: theme.font(13) },
  actions: { flexDirection: "row", gap: spacing.md },
  action: { flex: 1, minWidth: 0 },
}));
