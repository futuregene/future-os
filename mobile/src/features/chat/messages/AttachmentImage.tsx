import { useContext, useEffect, useRef, useState } from "react";
import { Image, Pressable, StyleSheet } from "react-native";
import type { HistoryAttachment } from "../../../remote/types";
import { MarkdownImageLoaderContext } from "../../../components/MarkdownImage";
import { useImageViewer } from "../../../components/imageViewer";
import { radius } from "../../../theme/tokens";
import { createThemedStyles } from "../../../theme/ThemeContext";
import { AttachmentChip } from "./AttachmentChip";

/** A picture the user sent, drawn as itself rather than as a file name. */
export function AttachmentImage({ attachment, onOpen }: {
  attachment: HistoryAttachment;
  onOpen?(): void;
}) {
  const styles = useStyles();
  const loader = useContext(MarkdownImageLoaderContext);
  const openImageViewer = useImageViewer();
  // A bubble that was just sent points at this phone's own copy; durable history
  // names a file on the desktop that has to come through the same preview
  // transfer the reply images use.
  const local = /^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path) ? attachment.path : null;
  // A picture this phone already holds paints on the first frame: the index is
  // local, so re-entering a conversation does not show a name chip first and
  // then swap it for the picture (twice, once per image, one round trip apart).
  const [uri, setUri] = useState<string | null>(() => local ?? loader?.cached(attachment.path) ?? null);
  // Same seam as the reply images: the last ratio this picture had is reused so
  // the row it sits in does not change height when it is drawn again.
  const [aspectRatio, setAspectRatio] = useState(() => loader?.aspect?.(attachment.path) ?? 1.5);
  const pending = useRef<AbortController | null>(null);
  useEffect(() => () => pending.current?.abort(), []);
  // Load once per attachment, and only once: the transcript re-renders on every
  // streaming delta, so an unguarded effect would re-request the picture on each
  // tick. A picture kept from an earlier open resolves from disk on the spot
  // (the loader still re-checks it with the desktop behind the paint). No loader
  // means this surface cannot read the desktop (or a test has none): the chip is
  // then the only honest thing to draw.
  const autoStarted = useRef(false);
  useEffect(() => {
    if (autoStarted.current || local || !loader) return;
    autoStarted.current = true;
    const request = new AbortController();
    pending.current = request;
    void loader.load(attachment.path, request.signal)
      .then((loaded) => {
        if (!request.signal.aborted && loaded) setUri(loaded);
      })
      .catch(() => {
        // A refused or failed transfer leaves the chip in place: tapping it is
        // the flow that reports what went wrong.
      })
      .finally(() => {
        if (!request.signal.aborted) pending.current = null;
      });
  }, [attachment.path, loader, local]);
  // No bytes yet, no viewer on this surface, or a file the phone cannot read as
  // an image: the named chip is the fallback, and it still opens the full file
  // preview (with its download/share actions).
  if (!uri || !openImageViewer) return <AttachmentChip attachment={attachment} onOpen={onOpen} />;
  return (
    <Pressable
      accessibilityLabel={attachment.name}
      accessibilityRole="button"
      onPress={() => openImageViewer({ name: attachment.name, uri })}
      style={({ pressed }) => [styles.frame, pressed && styles.pressed]}
    >
      <Image
        onLoad={(event) => {
          // Same seam as the reply images: only a native renderer reports the
          // intrinsic size, and the 1.5 default keeps the box sane when it does
          // not.
          const source = event?.nativeEvent?.source;
          if (source && source.width > 0 && source.height > 0 && Number.isFinite(source.width / source.height)) {
            const ratio = source.width / source.height;
            setAspectRatio(ratio);
            loader?.rememberAspect?.(attachment.path, ratio);
          }
        }}
        resizeMode="contain"
        source={{ uri }}
        style={[styles.image, { aspectRatio }]}
      />
    </Pressable>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  frame: { borderRadius: radius.md, maxWidth: "100%", overflow: "hidden", width: 200 },
  image: { backgroundColor: theme.colors.surfaceSubtle, width: "100%" },
  pressed: { opacity: 0.85 },
}));
