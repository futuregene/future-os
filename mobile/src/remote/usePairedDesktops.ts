import { useCallback, useEffect, useRef, useState } from "react";
import type { MutableRefObject } from "react";
import { AppState } from "react-native";
import { RemoteApiError, classifyError } from "./connectionState";
import { attemptPendingRevoke, claimPairingCode, serverRevoke } from "./pairing";
import { discardPendingPrompt } from "./pendingPromptStorage";
import { discardPendingContinuation } from "./pendingContinuationStorage";
import {
  clearCredentials,
  clearPendingRevoke,
  labelDesktop as labelStoredDesktop,
  loadActiveDesktopId,
  loadCredentials,
  loadPairedDesktops,
  loadPendingRevoke,
  renameDesktop as renameStoredDesktop,
  saveCredentials,
  savePendingRevoke,
  setActiveDesktop,
} from "./storage";
import type { PairedDesktop, RemoteCredentials } from "./types";

/**
 * The phone's pairings: which desktops it is paired to, which one it is reading,
 * and the two operations that change that set.
 *
 * Pairing is an app-level concern, not a connection's: a desktop stays paired
 * while it is offline, unreachable, or asleep, and its pairing has to survive
 * the connection lifecycle entirely (a reconnect never re-pairs, a pairing
 * never depends on a socket). Keeping it apart from every runtime is what lets
 * N runtimes exist at all — "which desktops are paired" is answered once, and
 * each runtime is only told about its own.
 */
export interface PairedDesktops {
  /** True until the stored registry has been read — the app's boot state. */
  booting: boolean;
  desktops: PairedDesktop[];
  activeDesktopId: string | null;
  /**
   * Credentials a claim produced, for the desktop they name, until that
   * desktop's runtime reports its own. Null once nothing has just been paired.
   */
  claimedCredentials: RemoteCredentials | null;
  /**
   * Remount counter per desktop.
   *
   * A connection that has given up is not re-opened from the inside, so the way
   * back from a terminal fault is a fresh runtime rather than a retry inside
   * the old one. Bumping a desktop's generation remounts exactly that runtime,
   * which is also why it is per desktop: recovering one machine must not
   * disturb the others' live connections.
   */
  generations: Record<string, number>;
  error: string | null;
  pair(code: string): Promise<void>;
  /** Remove the pairing this phone is reading. */
  unpair(): Promise<void>;
  removeDesktop(desktopId: string): Promise<void>;
  renameDesktop(desktopId: string, name: string): Promise<void>;
  labelDesktop(desktopId: string, patch: { name?: string; icon?: string }): Promise<void>;
  /** Read another paired desktop. Persisted, so the next launch opens it. */
  selectDesktop(desktopId: string): Promise<void>;
  /** Rebuild a runtime from scratch (the explicit "reconnect" affordance). */
  reconnect(desktopId?: string): void;
  drainRevokes(): Promise<void>;
  clearError(): void;
}

export function usePairedDesktops({
  networkAvailableRef,
}: {
  networkAvailableRef: MutableRefObject<boolean | null>;
}): PairedDesktops {
  const [booting, setBooting] = useState(true);
  const [desktops, setDesktops] = useState<PairedDesktop[]>([]);
  const [activeDesktopId, setActiveDesktopId] = useState<string | null>(null);
  const [generations, setGenerations] = useState<Record<string, number>>({});
  const [error, setError] = useState<string | null>(null);
  /**
   * The credentials a claim just produced.
   *
   * A successful pair has durable credentials before any runtime exists for
   * them, and the screens should be able to read the new desktop immediately:
   * without this the app would fall back to "no desktop selected" for as long
   * as the runtime takes to boot — the picker, for a machine the user has just
   * been told they are connected to. Only ever used for the desktop it names.
   */
  const [claimedCredentials, setClaimedCredentials] = useState<RemoteCredentials | null>(null);
  const revokesRunningRef = useRef(false);
  const revokeTerminalRef = useRef(new Set<string>());

  const refreshDesktops = useCallback(async () => {
    const stored = await loadPairedDesktops();
    setDesktops(stored);
    setActiveDesktopId((current) =>
      current && stored.some((item) => item.desktopId === current) ? current : null);
    return stored;
  }, []);

  const drainRevokes = useCallback(async () => {
    if (revokesRunningRef.current || networkAvailableRef.current === false) return;
    revokesRunningRef.current = true;
    try {
      const attempted = new Set(revokeTerminalRef.current);
      for (;;) {
        const pending = await loadPendingRevoke(attempted);
        if (!pending || attempted.has(pending.pairId)) return;
        attempted.add(pending.pairId);
        try {
          await attemptPendingRevoke(pending);
          await clearPendingRevoke(pending.pairId);
        } catch (error) {
          if (
            classifyError(error) !== "transport" ||
            (error instanceof RemoteApiError &&
              error.status >= 400 &&
              error.status < 500 &&
              error.status !== 408 &&
              error.status !== 429)
          )
            revokeTerminalRef.current.add(pending.pairId);
        }
      }
    } finally {
      revokesRunningRef.current = false;
    }
  }, [networkAvailableRef]);

  useEffect(() => {
    const timer = setInterval(() => {
      if (AppState.currentState === "active") void drainRevokes().catch(() => undefined);
    }, 30_000);
    return () => clearInterval(timer);
  }, [drainRevokes]);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        void drainRevokes();
        // Resolve the registry before the active selection: an empty
        // installation and an unselected desktop are different states, and the
        // app renders a different screen for each.
        const stored = await refreshDesktops();
        if (!active) return;
        if (stored.length > 0) {
          const storedActive = await loadActiveDesktopId();
          if (!active) return;
          // A dangling pointer is not an error: the picker is where the user
          // chooses, and a stale name must not be reported as a broken pairing.
          if (storedActive && stored.some((item) => item.desktopId === storedActive))
            setActiveDesktopId(storedActive);
        }
      } catch (nextError) {
        if (!active) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        if (active) setBooting(false);
      }
    })();
    return () => { active = false; };
  }, [drainRevokes, refreshDesktops]);

  const pair = useCallback(async (code: string) => {
    setError(null);
    const claimed = await claimPairingCode(code);
    // A successful pair must mean its credentials are durable: the runtime that
    // mounts for it reads them from storage, so an unwritten bundle would look
    // like a desktop that paired and immediately vanished.
    await saveCredentials(claimed);
    setClaimedCredentials(claimed);
    await refreshDesktops();
    setActiveDesktopId(claimed.expectedDesktopId);
  }, [refreshDesktops]);

  const selectDesktop = useCallback(async (desktopId: string) => {
    await setActiveDesktop(desktopId);
    setActiveDesktopId(desktopId);
  }, []);

  const reconnect = useCallback((desktopId?: string) => {
    const target = desktopId ?? activeDesktopId;
    if (!target) return;
    setGenerations((current) => ({ ...current, [target]: (current[target] ?? 0) + 1 }));
  }, [activeDesktopId]);

  const unpair = useCallback(async () => {
    const current = await loadCredentials().catch(() => null);
    // Preserve the existing best-effort desktop notification: the phone stops
    // locally first and compensates the cloud revoke after.
    if (current) await savePendingRevoke(current);
    await clearCredentials(current?.pairId);
    await refreshDesktops();
    await discardPendingPrompt(current?.pairId);
    await discardPendingContinuation(current?.pairId);
    if (current)
      void serverRevoke(current)
        .then(() => clearPendingRevoke(current.pairId))
        .catch(() => undefined);
  }, [refreshDesktops]);

  const removeDesktop = useCallback(async (desktopId: string) => {
    if (desktopId === activeDesktopId) return unpair();
    const stored = await loadCredentials(desktopId);
    if (!stored) return;
    await savePendingRevoke(stored);
    await clearCredentials(stored.pairId);
    await discardPendingPrompt(stored.pairId);
    await discardPendingContinuation(stored.pairId);
    await refreshDesktops();
    void drainRevokes();
  }, [activeDesktopId, drainRevokes, refreshDesktops, unpair]);

  const renameDesktop = useCallback(async (desktopId: string, name: string) => {
    await renameStoredDesktop(desktopId, name);
    await refreshDesktops();
  }, [refreshDesktops]);

  const labelDesktop = useCallback(async (
    desktopId: string,
    patch: { name?: string; icon?: string },
  ) => {
    await labelStoredDesktop(desktopId, patch);
    await refreshDesktops();
  }, [refreshDesktops]);

  const clearError = useCallback(() => setError(null), []);

  return {
    booting,
    desktops,
    activeDesktopId,
    claimedCredentials,
    generations,
    error,
    pair,
    unpair,
    removeDesktop,
    renameDesktop,
    labelDesktop,
    selectDesktop,
    reconnect,
    drainRevokes,
    clearError,
  };
}

export type { RemoteCredentials };
