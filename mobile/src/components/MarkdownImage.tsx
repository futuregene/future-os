import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityIndicator, Image, Pressable, StyleSheet, Text, View } from "react-native";
import { basename, classifyMarkdownTarget, localFilePath, remoteMarkdownImageUrl } from "@future-os/markdown";
import { radius, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";
import { useImageViewer } from "./imageViewer";

export interface MarkdownImageLoader {
  /** Includes desktop/pair/session identity; paths alone are not cache keys. */
  scope: string;
  cached(path: string): string | null;
  load(path: string, signal: AbortSignal): Promise<string | null>;
}
export const MarkdownImageLoaderContext = createContext<MarkdownImageLoader | null>(null);
export const MarkdownImageBasePathContext = createContext<string | undefined>(undefined);

/** Resolve a document-relative Markdown path (an image or a link) against the
 * desktop path of the document that contains it, never the downloaded phone
 * cache file. The host canonicalizes and enforces its own file-read boundary.
 * Returns null when there is no usable base: a document opened from this
 * phone's picker names a phone-local file, and a relative path cannot be
 * resolved to anything the desktop can read.
 */
export function resolveMarkdownPath(src: string, basePath?: string): string | null {
  const path = localFilePath(src);
  if (!path) return null;
  if (basePath && /^[a-z][a-z0-9+.-]*:\/\//i.test(basePath)) return null;
  if (!basePath || /^(?:[a-z]:[\\/]|[\\/])/i.test(path)) return path;
  const slash = Math.max(basePath.lastIndexOf("/"), basePath.lastIndexOf("\\"));
  return slash < 0 ? path : `${basePath.slice(0, slash + 1)}${path}`;
}

export function MarkdownImage({ src, alt, width, href, openTarget }: {
  src: string;
  alt: string;
  width?: number;
  href?: string;
  openTarget(target: string): void;
}) {
  const styles = useStyles();
  const loader = useContext(MarkdownImageLoaderContext);
  const openImageViewer = useImageViewer();
  const basePath = useContext(MarkdownImageBasePathContext);
  const url = remoteMarkdownImageUrl(src);
  const path = resolveMarkdownPath(src, basePath);
  const target = classifyMarkdownTarget(href ?? "");
  const linked = target.kind === "local-file" || target.kind === "external-url";
  if (url) {
    const image = <LoadedImage key={url} alt={alt} uri={url} width={width} />;
    if (linked)
      return <Pressable accessibilityRole="link" accessibilityLabel={alt || href} onPress={() => openTarget(href!)}>{image}</Pressable>;
    // A picture the agent linked from elsewhere (a chart host, a gist, a
    // published figure) is still a picture in the reply: tap it to see it full
    // screen rather than having to open the link and come back.
    if (!openImageViewer)
      return image;
    return (
      <Pressable accessibilityLabel={alt} accessibilityRole="button" onPress={() => openImageViewer({ name: alt, uri: url })}>
        {image}
      </Pressable>
    );
  }
  if (!path || !loader) return (
    <Text onPress={localFilePath(src) ? () => openTarget(href ?? src) : undefined} style={styles.link}>
      {alt || basename(src)}
    </Text>
  );
  return <LocalImage key={`${loader.scope}:${path}`} alt={alt || basename(path)} loader={loader} path={path} width={width} onOpen={() => openTarget(href ?? src)} linked={linked} />;
}

function LocalImage({ alt, loader, path, width, onOpen, linked }: {
  alt: string;
  loader: MarkdownImageLoader;
  path: string;
  width?: number;
  onOpen(): void;
  linked: boolean;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const openImageViewer = useImageViewer();
  const [uri, setUri] = useState(() => loader.cached(path));
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const pending = useRef<AbortController | null>(null);
  useEffect(() => () => pending.current?.abort(), []);
  const load = useCallback(async () => {
    if (pending.current) return;
    const request = new AbortController();
    pending.current = request;
    setLoading(true);
    setFailed(false);
    try {
      const result = await loader.load(path, request.signal);
      if (!request.signal.aborted) setUri(result);
    } catch {
      if (!request.signal.aborted) setFailed(true);
    } finally {
      if (!request.signal.aborted) {
        pending.current = null;
        setLoading(false);
      }
    }
  }, [loader, path]);
  // Load once per mount, and only once: a reply re-projects on every streaming
  // delta, so an unguarded effect here would re-request the image on each tick.
  // An image written into the Markdown is part of the document either way — in a
  // reply or in a file preview — so it never waits for a tap.
  const autoStarted = useRef(false);
  useEffect(() => {
    if (autoStarted.current || uri) return;
    autoStarted.current = true;
    void load();
  }, [load, uri]);
  if (uri) {
    // Zoomed-in viewing is local: the bytes are already here, so the viewer opens
    // without a transfer. A linked picture keeps opening its link.
    const picture = <LoadedImage key={uri} alt={alt} uri={uri} width={width} onFailure={() => { setUri(null); setFailed(true); }} />;
    if (linked || !openImageViewer)
      return <Pressable accessibilityLabel={alt} accessibilityRole={linked ? "link" : "button"} onPress={onOpen}>{picture}</Pressable>;
    return <Pressable accessibilityLabel={alt} accessibilityRole="button" onPress={() => openImageViewer({ name: alt, uri })}>{picture}</Pressable>;
  }
  return (
    <View style={styles.placeholder}>
      <Text selectable style={styles.caption}>{alt}</Text>
      <Pressable accessibilityRole="button" accessibilityLabel={t(failed ? "attachment.retryImage" : "attachment.loadImage")} disabled={loading} onPress={() => void load()} style={styles.loadButton}>
        {loading ? <ActivityIndicator color={colors.accent} /> : <Text style={styles.link}>{t(failed ? "attachment.retryImage" : "attachment.loadImage")}</Text>}
      </Pressable>
      <Text accessibilityRole="link" onPress={onOpen} style={styles.link}>{t(linked ? "attachment.openImageLink" : "attachment.open")}</Text>
    </View>
  );
}

function LoadedImage({ alt, uri, width, onFailure }: { alt: string; uri: string; width?: number; onFailure?(): void }) {
  const styles = useStyles();
  const [failed, setFailed] = useState(false);
  const [aspectRatio, setAspectRatio] = useState(1.5);
  if (failed) return <Text selectable style={styles.caption}>{alt || uri}</Text>;
  // `width` is a declarative maximum (from `<img width=…>`): `width: "100%"`
  // fills the container while `maxWidth` caps it, so a 600px image fills a
  // phone and is capped on a wide preview; an unset width fills unbounded.
  // `alignSelf: "center"` overrides the `100%` width and centers a capped image
  // in its (block) parent; the parent is block in chat and preview alike.
  return <Image accessibilityLabel={alt} source={{ uri }} resizeMode="contain" style={[styles.image, { aspectRatio }, width ? { alignSelf: "center", maxWidth: width } : null]}
    onError={() => { setFailed(true); onFailure?.(); }}
    onLoad={(event) => {
      // The intrinsic size only exists on renderers that decode the image
      // natively; react-native-web's Image reports no `source`, so this must not
      // assume it is there (it threw inside the load callback on the web harness).
      // The 1.5 default above keeps the layout sane when the size is unknown.
      const source = event?.nativeEvent?.source;
      if (source && source.width > 0 && source.height > 0
        && Number.isFinite(source.width / source.height)) {
        setAspectRatio(source.width / source.height);
      }
    }} />;
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  image: { width: "100%", marginVertical: spacing.sm, borderRadius: radius.md, backgroundColor: theme.colors.surfaceSubtle },
  caption: { color: theme.colors.inkMuted },
  link: { color: theme.colors.accent, textDecorationLine: "underline" },
  placeholder: { marginVertical: spacing.sm, padding: spacing.md, borderRadius: radius.md, backgroundColor: theme.colors.surfaceSubtle },
  loadButton: { minHeight: 44, justifyContent: "center", alignItems: "flex-start" },
}));
