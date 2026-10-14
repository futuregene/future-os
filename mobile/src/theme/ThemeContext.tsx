import { createContext, useContext, useEffect, useMemo, useState, useSyncExternalStore, type PropsWithChildren } from "react";
import { useColorScheme } from "react-native";
import { getAppearance, initializeAppearance, subscribeAppearance, type Appearance } from "./preferences";
import { buildTheme, lightTheme, resolveScheme, type Theme } from "./theme";

// The default is a real theme, not a sentinel: a component rendered without a
// provider (a test, or a tree mounted before the provider) draws in light at the
// designed text size, which is what the app looked like before it had a setting.
const ThemeContext = createContext<Theme>(lightTheme);

/** The palette, text size and sizing functions for this render. */
export function useTheme(): Theme {
  return useContext(ThemeContext);
}

export function useAppearance(): Appearance {
  return useSyncExternalStore(subscribeAppearance, getAppearance);
}

/**
 * Read the stored appearance before the first screen is drawn.
 *
 * AsyncStorage has no synchronous read, so without this gate the app paints one
 * frame at the default and then swaps — a full-screen flash on a phone whose
 * saved scheme differs from the one it boots into.
 */
export function useAppearanceReady(): boolean {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let mounted = true;
    void initializeAppearance().then(() => {
      if (mounted) setReady(true);
    });
    return () => { mounted = false; };
  }, []);
  return ready;
}

export function ThemeProvider({ children }: PropsWithChildren) {
  const systemScheme = useColorScheme();
  const { themeMode, fontSize } = useAppearance();
  // `null` (the scheme is not reported yet, or the platform has no preference)
  // is light, which is what the app has always been.
  const scheme = resolveScheme(themeMode, systemScheme);
  const theme = useMemo(() => buildTheme(scheme, fontSize), [scheme, fontSize]);
  return <ThemeContext.Provider value={theme}>{children}</ThemeContext.Provider>;
}

export type ThemedStyles<T> = (() => T) & {
  /** The same styles without a component, for tests and fixtures. */
  forTheme(theme: Theme): T;
};

/**
 * Build a set of styles from the theme.
 *
 * Colours and text sizes are the reader's choice now, so a style sheet can no
 * longer be created once at import time. It is created per theme — shared by
 * every component that uses it, not per instance — and rebuilt when the scheme
 * or the text size changes:
 *
 *     const useStyles = createThemedStyles(theme => StyleSheet.create({ ... }));
 *     const styles = useStyles();  // in the component
 */
export function createThemedStyles<T>(create: (theme: Theme) => T): ThemedStyles<T> {
  const cache = new WeakMap<Theme, T>();
  const forTheme = (theme: Theme): T => {
    let styles = cache.get(theme);
    if (!styles) {
      styles = create(theme);
      cache.set(theme, styles);
    }
    return styles;
  };
  return Object.assign(() => forTheme(useTheme()), { forTheme });
}
