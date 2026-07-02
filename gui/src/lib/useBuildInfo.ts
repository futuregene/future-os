import { invokeCommand } from "../integrations/tauri/invoke";
import { useAsyncResource } from "./useAsyncResource";

/** Version + release/dev channel, injected at build time (see scripts/version.mjs). */
export interface BuildInfo {
  version: string;
  /** Release builds carry a plain `X.Y.Z`; dev builds carry a `-dev.<hash>` suffix. */
  isRelease: boolean;
}

/**
 * Load the app's build identity from the Tauri backend. `data` is `null` until
 * it resolves; treat "not yet known" as "not a test build" so nothing flashes.
 */
export function useBuildInfo() {
  return useAsyncResource<BuildInfo | null>(
    () => invokeCommand<BuildInfo>("app_build_info"),
    [],
    null,
  );
}
