import { ListChecks, MoreHorizontal, Search } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, StyleSheet } from "react-native";
import { ActionMenu } from "../components/ActionMenu";
import { radius } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";

/**
 * The list's secondary tools, as one control in the header.
 *
 * They used to be two 44pt buttons at the end of the row under the header —
 * the row the pages' tabs sit in — and a third page would have left each tab
 * about 77pt wide on a phone, shrinking the labels to fit. Search and batch
 * selection are one-shot actions rather than modes of the page they sit beside,
 * so they move up here and open from one button, which leaves the tab row its
 * whole width for the pages themselves.
 *
 * The search *field* is still the list's: it replaces the tab row while the
 * reader types, so the header — the machine, the connection, settings — stays
 * on screen. This is only the way in.
 */
export function SessionListActions({
  onSearch,
  onSelect,
  canSelect,
}: {
  onSearch(): void;
  onSelect(): void;
  /**
   * Whether a batch has anything it may hold.
   *
   * Read from the same catalogue-derived judgement the list selects rows by,
   * so this button cannot promise a selection the list would refuse to make.
   */
  canSelect: boolean;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Pressable
        accessibilityLabel={t("sessions.listActions")}
        accessibilityRole="button"
        onPress={() => setOpen(true)}
        style={({ pressed }) => [styles.trigger, pressed && styles.pressed]}
      >
        <MoreHorizontal color={colors.inkSoft} size={21} />
      </Pressable>
      <ActionMenu
        title={t("sessions.listActions")}
        visible={open}
        onClose={() => setOpen(false)}
        actions={[
          {
            label: t("sessions.search"),
            icon: <Search size={18} color={colors.inkSoft} />,
            onPress: onSearch,
          },
          {
            label: t("sessions.select"),
            icon: <ListChecks size={18} color={colors.inkSoft} />,
            disabled: !canSelect,
            onPress: onSelect,
          },
        ]}
      />
    </>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  trigger: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: theme.colors.lineSoft,
  },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
}));
