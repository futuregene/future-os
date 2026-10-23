import { useCallback, useEffect, useRef, useState } from "react";
import type { MutableRefObject } from "react";
import * as Network from "expo-network";
import { AppState } from "react-native";
import type { RecoveryReason } from "./client";
import { NATIVE_PRESENTATION_GRACE_MS, nativePresentationInFlight } from "./nativePresentation";

// Avoid tearing down for a quick app switch / native activity transition.
// This is not a background execution or push-notification guarantee.
export const BACKGROUND_GRACE_MS = 5_000;
export const NETWORK_PROBE_TIMEOUT_MS = 4_000;

interface AppLifecycleOptions {
  /**
   * The last network verdict, shared with the pending-revoke drainer so a
   * revoke is not attempted on a radio the OS says is off.
   */
  networkAvailableRef: MutableRefObject<boolean | null>;
  /**
   * Hold or retire every runtime's socket.
   *
   * Ordinary WebSockets are not a background execution mechanism on either
   * mobile OS. With a runtime per desktop this is the *only* thing standing
   * between a backgrounded app and N open sockets, so it is one decision made
   * once for all of them rather than N listeners each guessing.
   */
  setRuntimesActive(active: boolean): void;
  /** Recover every runtime's connection. */
  recoverAll(reason: RecoveryReason): void;
}

export interface AppLifecycle {
  /**
   * Whether the app is in the foreground. A runtime asks this before it tells a
   * desktop which conversation to detail: a phone in the background is reading
   * nothing, so it should cost nothing.
   */
  foreground: boolean;
}

/**
 * The process-wide lifecycle: whether the app is in front, and what the network
 * is doing.
 *
 * Both are facts about *this phone*, not about any desktop, so they are watched
 * once here and applied to every runtime through the two callbacks above. A
 * per-desktop copy would be N listeners disagreeing about the same radio, and
 * N chances to leave a socket open behind the app.
 */
export function useAppLifecycle({
  networkAvailableRef,
  setRuntimesActive,
  recoverAll,
}: AppLifecycleOptions): AppLifecycle {
  const [foreground, setForeground] = useState(AppState.currentState !== "background");
  const refreshNetworkStateRef = useRef(async () => networkAvailableRef.current !== false);

  const recover = useCallback(
    (reason: RecoveryReason) => {
      if (AppState.currentState !== "active") return;
      if (reason === "foreground") {
        // Refresh the advisory snapshot in parallel. A sleeping radio or hung
        // native query must not delay the first real connection attempt.
        void refreshNetworkStateRef.current();
      }
      recoverAll(reason);
    },
    [recoverAll],
  );

  useEffect(() => {
    let previous = AppState.currentState;
    let backgrounded = previous === "background";
    let backgroundDeadline: number | null = backgrounded ? Date.now() : null;
    let presentationTimer: ReturnType<typeof setTimeout> | null = null;
    const clearPresentationTimer = () => {
      if (presentationTimer) clearTimeout(presentationTimer);
      presentationTimer = null;
    };
    const subscription = AppState.addEventListener("change", (next) => {
      const returnedToForeground = next === "active" && previous !== "active";
      const enteredBackground = next === "background" && previous !== "background";
      previous = next;
      setForeground(next !== "background");
      if (enteredBackground) {
        backgrounded = true;
        clearPresentationTimer();
        // Keep a picker/quick-switch connection alive. Do not depend on the
        // picker promise still being pending at expiry: Android can deliver
        // its result before delivering the resumed AppState event.
        const graceMs = nativePresentationInFlight() ? NATIVE_PRESENTATION_GRACE_MS : BACKGROUND_GRACE_MS;
        backgroundDeadline = Date.now() + graceMs;
        presentationTimer = setTimeout(() => {
          presentationTimer = null;
          if (previous !== "active") setRuntimesActive(false);
        }, graceMs);
      } else if (returnedToForeground) {
        clearPresentationTimer();
        // iOS alerts/control center emit inactive -> active without suspending
        // the app. They must not restart history/catalogue sync or handshake.
        if (backgrounded) {
          backgrounded = false;
          // The OS can suspend JS before the grace timer fires, then deliver
          // active before overdue timers on resume. Retire the stale socket
          // and in-flight recovery before starting a new foreground attempt.
          if (backgroundDeadline !== null && Date.now() >= backgroundDeadline) {
            setRuntimesActive(false);
          }
          backgroundDeadline = null;
          setRuntimesActive(true);
          recover("foreground");
        }
      }
    });
    return () => {
      clearPresentationTimer();
      subscription.remove();
    };
  }, [recover, setRuntimesActive]);

  useEffect(() => {
    let active = true;
    let observationRevision = 0;
    let previousType: Network.NetworkStateType | undefined;
    const observe = (state: Network.NetworkState, triggerRecovery = true): boolean => {
      if (!active) return false;
      const available =
        state.type !== Network.NetworkStateType.NONE &&
        state.isConnected !== false &&
        state.isInternetReachable !== false;
      const wasAvailable = networkAvailableRef.current;
      const pathChanged =
        wasAvailable === true &&
        available &&
        previousType !== undefined &&
        previousType !== Network.NetworkStateType.UNKNOWN &&
        state.type !== undefined &&
        state.type !== Network.NetworkStateType.UNKNOWN &&
        state.type !== previousType;
      networkAvailableRef.current = available;
      previousType = state.type;
      // Negative hints must not close a working socket or cancel its retries.
      // Positive native events can accelerate recovery; transport errors and
      // authenticated presence remain the source of connection truth.
      if (!available) return false;
      if (triggerRecovery && wasAvailable === false) recover("network-restored");
      else if (triggerRecovery && pathChanged) recover("network-changed");
      return true;
    };
    refreshNetworkStateRef.current = async () => {
      const revision = ++observationRevision;
      let timer: ReturnType<typeof setTimeout> | undefined;
      try {
        const state = await Promise.race([
          Network.getNetworkStateAsync(),
          new Promise<never>((_, reject) => {
            timer = setTimeout(() => reject(new Error("network_probe_timeout")), NETWORK_PROBE_TIMEOUT_MS);
          }),
        ]);
        // A newer query or native event supersedes this snapshot. Its late
        // offline result must not tear down a newly recovered connection.
        if (!active) return false;
        if (revision !== observationRevision) return networkAvailableRef.current !== false;
        return observe(state, false);
      } catch (nextError) {
        if (!active) return false;
        if (revision === observationRevision) {
          console.warn("[remote] foreground network refresh failed", {
            error: nextError,
          });
        }
        return networkAvailableRef.current !== false;
      } finally {
        if (timer) clearTimeout(timer);
      }
    };
    const initialRevision = observationRevision;
    void Network.getNetworkStateAsync()
      .then((state) => {
        if (initialRevision === observationRevision) observe(state);
      })
      .catch(() => undefined);
    const subscription = Network.addNetworkStateListener((state) => {
      observationRevision += 1;
      observe(state);
    });
    return () => {
      active = false;
      subscription.remove();
      refreshNetworkStateRef.current = async () => networkAvailableRef.current !== false;
    };
  }, [networkAvailableRef, recover]);

  return { foreground };
}
