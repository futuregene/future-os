import type { ImageViewerTarget } from "../../../components/imageViewer";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react-native";
import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";
import { ZoomableImage } from "./ZoomableImage";
import { previewSurfaceInsets } from "./previewInsets";

/**
 * A picture from the conversation, full screen: pinch to zoom, drag to pan, tap
 * the close button (or the Android back gesture) to leave. This is the phone's
 * counterpart to the desktop's image lightbox — the same gesture, reached by
 * tapping any image the user can see: one the agent drew in a reply, or one they
 * sent themselves.
 *
 * It deliberately carries no file actions: downloading or sharing an attachment
 * is the file preview's job (`PreviewModal`), which the attachment chip still
 * opens. Looking at the picture is one tap away and must not cost a transfer
 * dialog.
 */
export function ImageViewer({ target, onClose }: { target: ImageViewerTarget | null; onClose: () => void }) {
  const { t } = useTranslation();
  const styles = useStyles();
  const insets = useSafeAreaInsets();
  if (!target) return null;
  return (
    <Modal
      animationType="fade"
      onRequestClose={onClose}
      statusBarTranslucent
      transparent={false}
      visible
    >
      <View style={[styles.surface, previewSurfaceInsets(insets)]} testID="image-viewer">
        <View style={styles.header}>
          <Text numberOfLines={1} style={styles.title}>
            {target.name ?? ""}
          </Text>
          <Pressable
            accessibilityLabel={t("common.close")}
            accessibilityRole="button"
            onPress={onClose}
            style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
          >
            <CloseIcon />
          </Pressable>
        </View>
        {/* Keyed by uri so a second picture starts from fit instead of inheriting
            the previous one's zoom and pan. */}
        <ZoomableImage accessibilityLabel={target.name} key={target.uri} uri={target.uri} />
      </View>
    </Modal>
  );
}

function CloseIcon() {
  const { colors } = useTheme();
  return <X color={colors.ink} size={22} />;
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  surface: { alignItems: "stretch", backgroundColor: theme.colors.surface, flex: 1 },
  header: { alignItems: "center", flexDirection: "row", gap: spacing.sm, paddingHorizontal: spacing.md },
  iconButton: { alignItems: "center", borderRadius: radius.md, height: layout.touchTarget, justifyContent: "center", width: layout.touchTarget },
  pressed: { opacity: 0.6 },
  title: { color: theme.colors.inkStrong, flex: 1, fontSize: theme.font(16), fontWeight: "700", minWidth: 0 },
}));
