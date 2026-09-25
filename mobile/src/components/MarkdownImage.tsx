import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityIndicator, Image, Pressable, StyleSheet, Text, View } from "react-native";
import { basename, classifyMarkdownTarget, localFilePath, remoteMarkdownImageUrl } from "@future-os/markdown";
import { colors, radius, spacing } from "../theme/tokens";

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

export function MarkdownImage({ src, alt, href, openTarget }: {
  src: string;
  alt: string;
  href?: string;
  openTarget(target: string): void;
}) {
  const loader = useContext(MarkdownImageLoaderContext);
  const basePath = useContext(MarkdownImageBasePathContext);
  const url = remoteMarkdownImageUrl(src);
  const path = resolveMarkdownPath(src, basePath);
  const target = classifyMarkdownTarget(href ?? "");
  const linked = target.kind === "local-file" || target.kind === "external-url";
  if (url) {
    const image = <LoadedImage key={url} alt={alt} uri={url} />;
    return linked ? <Pressable accessibilityRole="link" accessibilityLabel={alt || href} onPress={() => openTarget(href!)}>{image}</Pressable> : image;
  }
  if (!path || !loader) return (
    <Text onPress={localFilePath(src) ? () => openTarget(href ?? src) : undefined} style={styles.link}>
      {alt || basename(src)}
    </Text>
  );
  return <LocalImage key={`${loader.scope}:${path}`} alt={alt || basename(path)} loader={loader} path={path} onOpen={() => openTarget(href ?? src)} linked={linked} />;
}

function LocalImage({ alt, loader, path, onOpen, linked }: {
  alt: string;
  loader: MarkdownImageLoader;
  path: string;
  onOpen(): void;
  linked: boolean;
}) {
  const { t } = useTranslation();
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
  if (uri) return <Pressable accessibilityRole={linked ? "link" : "button"} accessibilityLabel={alt} onPress={onOpen}>
    <LoadedImage key={uri} alt={alt} uri={uri} onFailure={() => { setUri(null); setFailed(true); }} />
  </Pressable>;
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

function LoadedImage({ alt, uri, onFailure }: { alt: string; uri: string; onFailure?(): void }) {
  const [failed, setFailed] = useState(false);
  const [aspectRatio, setAspectRatio] = useState(1.5);
  if (failed) return <Text selectable style={styles.caption}>{alt || uri}</Text>;
  return <Image accessibilityLabel={alt} source={{ uri }} resizeMode="contain" style={[styles.image, { aspectRatio }]}
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

const styles = StyleSheet.create({
  image: { width: "100%", marginVertical: spacing.sm, borderRadius: radius.md, backgroundColor: colors.surfaceSubtle },
  caption: { color: colors.inkMuted },
  link: { color: colors.accent, textDecorationLine: "underline" },
  placeholder: { marginVertical: spacing.sm, padding: spacing.md, borderRadius: radius.md, backgroundColor: colors.surfaceSubtle },
  loadButton: { minHeight: 44, justifyContent: "center", alignItems: "flex-start" },
});
