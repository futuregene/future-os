import { useEffect, useState } from "react";
import { isMacOS, isWindows } from "../../lib/platform";
import { invokeCommand } from "../tauri/invoke";

interface WindowsSandboxProbeResult {
  available: boolean;
  code: string;
}

export interface SandboxAvailability {
  available: boolean;
  definitive: boolean;
  resolved: boolean;
}

let windowsProbe: Promise<boolean> | null = null;

const WINDOWS_PROBE_RETRY_DELAYS_MS = [0, 100, 250, 500, 1_000, 2_000] as const;

export function windowsSandboxAvailable(
  result: WindowsSandboxProbeResult,
): boolean {
  return result.available;
}

export function shouldPersistSandboxFallback(
  availability: SandboxAvailability,
  approvalTier: string,
): boolean {
  return (
    availability.resolved
    && availability.definitive
    && !availability.available
    && approvalTier === "sandbox"
  );
}

type WindowsSandboxProbe = () => Promise<WindowsSandboxProbeResult>;
type Delay = (milliseconds: number) => Promise<void>;

const delay: Delay = milliseconds =>
  new Promise(resolve => setTimeout(resolve, milliseconds));

export async function probeWindowsSandboxWithRetry(
  probe: WindowsSandboxProbe,
  retryDelays: readonly number[] = WINDOWS_PROBE_RETRY_DELAYS_MS,
  wait: Delay = delay,
): Promise<boolean> {
  let lastError: unknown;

  for (const retryDelay of retryDelays) {
    if (retryDelay > 0)
      await wait(retryDelay);

    try {
      return windowsSandboxAvailable(await probe());
    }
    catch (error) {
      // The bundled Agent starts off the Desktop launch path. A refused RPC
      // connection means "not ready yet", not "sandbox unsupported".
      lastError = error;
    }
  }

  throw lastError ?? new Error("Windows sandbox probe did not run");
}

function sharedWindowsProbe(): Promise<boolean> {
  windowsProbe ??= probeWindowsSandboxWithRetry(() =>
    invokeCommand<WindowsSandboxProbeResult>("probe_windows_sandbox"),
  ).catch((error) => {
    // Do not permanently cache a transient Agent connection failure. A later
    // mount (for example, opening Settings after startup) gets a fresh attempt.
    windowsProbe = null;
    throw error;
  });
  return windowsProbe;
}

function initialAvailability(): SandboxAvailability {
  if (isMacOS)
    return { available: true, definitive: true, resolved: true };
  if (!isWindows)
    return { available: false, definitive: true, resolved: true };
  return { available: false, definitive: false, resolved: false };
}

/**
 * Product-facing sandbox availability. Windows requires a successful native
 * host probe; the promise is shared so Settings and Composer do not
 * independently exercise the native probe.
 */
export function useSandboxAvailability(): SandboxAvailability {
  const [availability, setAvailability] = useState(initialAvailability);

  useEffect(() => {
    if (!isWindows)
      return;
    let current = true;
    void sharedWindowsProbe()
      .then((available) => {
        if (current)
          setAvailability({ available, definitive: true, resolved: true });
      })
      .catch(() => {
        // Exhausted connection/probe retries fail closed for this mount, but
        // are not an authoritative unsupported verdict. Keep the saved tier so
        // a later mount can recover automatically.
        if (current) {
          setAvailability({
            available: false,
            definitive: false,
            resolved: true,
          });
        }
      });
    return () => {
      current = false;
    };
  }, []);

  return availability;
}
