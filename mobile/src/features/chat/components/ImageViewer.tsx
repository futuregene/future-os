import type { GalleryImage } from "../../../components/imageGallery";
import type { ImageViewerTarget } from "../../../components/imageViewer";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight, X } from "lucide-react-native";
import { useCallback, useContext, useEffect, useState } from "react";
import { ActivityIndicator, Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { immediateGalleryUri } from "../../../components/imageGallery";
import { MarkdownImageLoaderContext, resolveMarkdownPath } from "../../../components/MarkdownImage";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";
import { ZoomableImage } from "./ZoomableImage";
import { previewSurfaceInsets } from "./previewInsets";

/**
 * A picture from the conversation, full screen: pinch to zoom, drag to pan, and
 * — when the reply drew several — swipe sideways or tap the arrows to step
 * through them. Both ends wrap, so the sequence reads as a ring rather than a
 * row with dead ends. Tap the close button (or the Android back gesture) to
 * leave. This is the phone's counterpart to the desktop's image lightbox — the
 * same gesture, reached by tapping any image the user can see: one the agent
 * drew in a reply, or one they sent themselves.
 *
 * It deliberately carries no file actions: downloading or sharing an attachment
 * is the file preview's job (`PreviewModal`), which the attachment chip still
 * opens. Looking at the picture is one tap away and must not cost a transfer
 * dialog.
 */
export function ImageViewer({ target, onClose }: { target: ImageViewerTarget | null; onClose: () => void }) {
  if (!target)
    return null;
  // Keyed by what was opened: the index belongs to one opening, and a second
  // picture (even the same URI asked for again) starts on the picture tapped
  // rather than inheriting the last visit's page.
  const opening = `${target.uri}:${target.gallery?.index ?? 0}:${target.gallery?.entries.length ?? 0}`;
  return <ViewerSurface key={opening} onClose={onClose} target={target} />;
}

function ViewerSurface({ target, onClose }: { target: ImageViewerTarget; onClose: () => void }) {
  const { t } = useTranslation();
  const styles = useStyles();
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const entries = target.gallery?.entries ?? [];
  const count = entries.length;
  const [index, setIndex] = useState(target.gallery?.index ?? 0);
  const step = useCallback((direction: -1 | 1) => {
    if (count < 2)
      return;
    setIndex(previous => (previous + direction + count) % count);
  }, [count]);
  const entry: GalleryImage = entries[index] ?? { id: "single", name: target.name ?? "", src: target.uri, uri: target.uri };
  const title = entry.name || target.name || "";
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
            {title}
          </Text>
          {count > 1
            ? (
                <Text accessibilityLabel={t("common.imagePosition", { current: index + 1, total: count })} style={styles.position}>
                  {index + 1}
                  {" / "}
                  {count}
                </Text>
              )
            : null}
          <Pressable
            accessibilityLabel={t("common.close")}
            accessibilityRole="button"
            onPress={onClose}
            style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
          >
            <CloseIcon />
          </Pressable>
        </View>
        {/* Keyed by the picture so a step starts from fit instead of inheriting
            the previous one's zoom and pan. */}
        <ViewerPage entry={entry} key={entry.id} onPage={count > 1 ? step : undefined} />
        {count > 1
          ? (
              <>
                <Pressable
                  accessibilityLabel={t("common.previousImage")}
                  accessibilityRole="button"
                  onPress={() => step(-1)}
                  style={({ pressed }) => [styles.pageButton, styles.pageButtonLeft, pressed && styles.pageButtonPressed]}
                >
                  <ChevronLeft color={colors.ink} size={26} />
                </Pressable>
                <Pressable
                  accessibilityLabel={t("common.nextImage")}
                  accessibilityRole="button"
                  onPress={() => step(1)}
                  style={({ pressed }) => [styles.pageButton, styles.pageButtonRight, pressed && styles.pageButtonPressed]}
                >
                  <ChevronRight color={colors.ink} size={26} />
                </Pressable>
              </>
            )
          : null}
      </View>
    </Modal>
  );
}

/** One picture of the sequence, resolving bytes the reply had not loaded yet. */
function ViewerPage({ entry, onPage }: { entry: GalleryImage; onPage?: (direction: -1 | 1) => void }) {
  const { t } = useTranslation();
  const styles = useStyles();
  const { colors } = useTheme();
  const loader = useContext(MarkdownImageLoaderContext);
  const path = resolveMarkdownPath(entry.src, entry.basePath);
  const [uri, setUri] = useState(() => immediateGalleryUri(entry) ?? (path && loader ? loader.cached(path) : null));
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (uri || !path || !loader)
      return;
    const request = new AbortController();
    loader.load(path, request.signal)
      .then(next => {
        if (!request.signal.aborted && next) setUri(next);
      })
      .catch(() => {
        if (!request.signal.aborted) setFailed(true);
      });
    return () => request.abort();
  }, [uri, path, loader]);
  if (!uri)
    return (
      <View style={styles.pageLoading}>
        {failed
          ? <Text style={styles.unavailable}>{t("common.imageUnavailable")}</Text>
          : <ActivityIndicator color={colors.accent} />}
      </View>
    );
  return <ZoomableImage accessibilityLabel={entry.name} key={uri} onPage={onPage} uri={uri} />;
}

function CloseIcon() {
  const { colors } = useTheme();
  return <X color={colors.ink} size={22} />;
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  surface: { alignItems: "stretch", backgroundColor: theme.colors.surface, flex: 1 },
  header: { alignItems: "center", flexDirection: "row", gap: spacing.sm, paddingHorizontal: spacing.md },
  iconButton: { alignItems: "center", borderRadius: radius.md, height: layout.touchTarget, justifyContent: "center", width: layout.touchTarget },
  pageButton: {
    alignItems: "center",
    backgroundColor: theme.colors.surface,
    borderColor: theme.colors.lineSoft,
    borderRadius: radius.pill,
    borderWidth: StyleSheet.hairlineWidth,
    height: layout.touchTarget,
    justifyContent: "center",
    position: "absolute",
    top: "50%",
    width: layout.touchTarget,
    marginTop: -layout.touchTarget / 2,
  },
  pageButtonLeft: { left: spacing.sm },
  pageButtonRight: { right: spacing.sm },
  pageButtonPressed: { opacity: 0.6 },
  pageLoading: { alignItems: "center", flex: 1, justifyContent: "center" },
  position: { color: theme.colors.inkMuted, fontSize: theme.font(13), fontVariant: ["tabular-nums"] },
  pressed: { opacity: 0.6 },
  title: { color: theme.colors.inkStrong, flex: 1, fontSize: theme.font(16), fontWeight: "700", minWidth: 0 },
  unavailable: { color: theme.colors.inkMuted, fontSize: theme.font(14) },
}));
