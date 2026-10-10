import { useState } from "react";
import {
  ActivityIndicator,
  FlatList,
  Image,
  Modal,
  Pressable,
  StyleSheet,
  Text,
  View,
  useWindowDimensions,
} from "react-native";
import type { TFunction } from "i18next";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { X } from "lucide-react-native";
import type { AlbumImage } from "future-file-handler";
import { Button } from "../../../components/Button";
import { colors, layout, radius, spacing } from "../../../theme/tokens";

export interface AlbumPickerModalProps {
  visible: boolean;
  /** The listing, or `null` while it is still being read. */
  images: AlbumImage[] | null;
  /** Why the listing is empty, when it is. */
  error: string | null;
  /** How many images this message can still take. */
  limit: number;
  onCancel(): void;
  onConfirm(images: AlbumImage[]): void;
  t: TFunction;
}

const COLUMNS = 3;

/**
 * The album grid for phones where no system photo picker exists — an Android
 * compatibility container, or an old build with no gallery app. It stays a
 * plain list of what the phone reports: no albums, no editing, no paging, just
 * "pick up to N of the newest images". The caller remounts it per listing, so a
 * cancelled round trip never carries its ticks into the next one.
 */
export function AlbumPickerModal({
  visible,
  images,
  error,
  limit,
  onCancel,
  onConfirm,
  t,
}: AlbumPickerModalProps) {
  const insets = useSafeAreaInsets();
  const { width } = useWindowDimensions();
  const [selected, setSelected] = useState<string[]>([]);

  const toggle = (uri: string) => {
    setSelected(current =>
      current.includes(uri)
        ? current.filter(item => item !== uri)
        : current.length >= limit
          ? current
          : [...current, uri],
    );
  };

  const gutter = layout.gutter;
  const gap = spacing.xs;
  const sheetWidth = width - spacing.md * 2 - gutter * 2;
  const cell = Math.floor((sheetWidth - gap * (COLUMNS - 1)) / COLUMNS);
  const chosen = images?.filter(image => selected.includes(image.uri)) ?? [];

  return (
    <Modal animationType="fade" onRequestClose={onCancel} transparent visible={visible}>
      <View style={styles.overlay}>
        <View
          accessibilityViewIsModal
          style={[
            styles.sheet,
            {
              paddingTop: insets.top + spacing.md,
              paddingBottom: insets.bottom + spacing.md,
              paddingLeft: insets.left + gutter,
              paddingRight: insets.right + gutter,
            },
          ]}
        >
          <View style={styles.header}>
            <Text accessibilityRole="header" style={styles.title}>
              {t("attachment.albumTitle")}
            </Text>
            <Text numberOfLines={1} style={styles.count}>
              {t("attachment.albumSelected", { count: chosen.length, limit })}
            </Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("attachment.albumClose")}
              onPress={onCancel}
              style={({ pressed }) => [styles.close, pressed && styles.closePressed]}
            >
              <X size={20} color={colors.inkSoft} />
            </Pressable>
          </View>

          {images === null && error === null ? (
            <View style={styles.center}>
              <ActivityIndicator color={colors.accent} />
              <Text style={styles.state}>{t("attachment.albumLoading")}</Text>
            </View>
          ) : error !== null || images === null || images.length === 0 ? (
            <View style={styles.center}>
              <Text style={styles.state}>
                {error !== null ? error : t("attachment.albumEmpty")}
              </Text>
            </View>
          ) : (
            <FlatList
              data={images}
              keyExtractor={image => image.uri}
              numColumns={COLUMNS}
              columnWrapperStyle={styles.row}
              contentContainerStyle={styles.grid}
              initialNumToRender={COLUMNS * 4}
              windowSize={5}
              renderItem={({ item }) => {
                const index = selected.indexOf(item.uri);
                return (
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={item.name}
                    accessibilityState={{ selected: index >= 0 }}
                    onPress={() => toggle(item.uri)}
                    style={[styles.cell, { width: cell, height: cell }]}
                  >
                    <Image
                      resizeMethod="resize"
                      resizeMode="cover"
                      source={{ uri: item.uri }}
                      style={styles.thumb}
                    />
                    {index >= 0 ? (
                      <View style={styles.selectedOverlay}>
                        <View style={styles.badge}>
                          <Text style={styles.badgeText}>{index + 1}</Text>
                        </View>
                      </View>
                    ) : null}
                  </Pressable>
                );
              }}
            />
          )}

          <View style={styles.footer}>
            {images !== null && images.length > 0 && chosen.length === 0 ? (
              <Text numberOfLines={1} style={styles.hint}>
                {t("attachment.albumLimitHint", { count: limit })}
              </Text>
            ) : null}
            <View style={styles.actions}>
              <Button
                label={t("chat.cancel")}
                onPress={onCancel}
                variant="secondary"
              />
              <Button
                disabled={chosen.length === 0}
                label={t("attachment.albumConfirm", { count: chosen.length })}
                onPress={() => onConfirm(chosen)}
              />
            </View>
          </View>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  overlay: { flex: 1, backgroundColor: colors.overlay },
  sheet: {
    flex: 1,
    margin: spacing.md,
    borderRadius: radius.xl,
    backgroundColor: colors.surface,
  },
  header: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  title: { fontSize: 17, fontWeight: "600", color: colors.inkStrong },
  count: { flex: 1, fontSize: 13, color: colors.inkMuted },
  close: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    marginRight: -spacing.sm,
  },
  closePressed: { opacity: 0.6 },
  center: { flex: 1, alignItems: "center", justifyContent: "center", gap: spacing.md },
  state: { fontSize: 14, color: colors.inkSoft, textAlign: "center" },
  grid: { paddingVertical: spacing.sm },
  row: { gap: spacing.xs, marginBottom: spacing.xs },
  cell: { borderRadius: radius.sm, overflow: "hidden", backgroundColor: colors.surfaceSubtle },
  thumb: { width: "100%", height: "100%" },
  selectedOverlay: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    borderWidth: 3,
    borderColor: colors.accent,
    backgroundColor: "rgba(37, 99, 235, 0.25)",
    alignItems: "flex-end",
  },
  badge: {
    minWidth: 22,
    height: 22,
    margin: spacing.xs,
    paddingHorizontal: spacing.xs,
    borderRadius: radius.pill,
    backgroundColor: colors.accent,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: { color: colors.surface, fontSize: 12, fontWeight: "600" },
  footer: { gap: spacing.sm, paddingTop: spacing.sm },
  hint: { fontSize: 12, color: colors.inkMuted },
  actions: { flexDirection: "row", justifyContent: "flex-end", gap: spacing.sm },
});
