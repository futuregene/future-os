import { useEffect, useState } from "react";
import { isLinux, isMacOS, isWindows } from "../../lib/platform";
import { invokeCommand } from "../tauri/invoke";

export interface SandboxProbeResult {
  available: boolean;
  code: string;
  backend: string;
  path?: string;
  version?: string;
  capabilities?: Record<string, boolean>;
}

export interface SandboxAvailability {
  available: boolean;
  definitive: boolean;
  resolved: boolean;
  code?: string;
  backend?: string;
}

let sharedProbe: Promise<SandboxProbeResult> | null = null;

const PROBE_RETRY_DELAYS_MS = [0, 100, 250, 500, 1_000, 2_000] as const;

type SandboxProbe = () => Promise<SandboxProbeResult>;
type Delay = (milliseconds: number) => Promise<void>;

const delay: Delay = milliseconds =>
  new Promise(resolve => setTimeout(resolve, milliseconds));

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

export async function probeSandboxWithRetry(
  probe: SandboxProbe,
  retryDelays: readonly number[] = PROBE_RETRY_DELAYS_MS,
  wait: Delay = delay,
): Promise<SandboxProbeResult> {
  let lastError: unknown;

  for (const retryDelay of retryDelays) {
    if (retryDelay > 0)
      await wait(retryDelay);

    try {
      return await probe();
    }
    catch (error) {
      // Agent startup and transport failures are transient. A successful RPC
      // carrying available=false is authoritative and is never retried.
      lastError = error;
    }
  }

  throw lastError ?? new Error("Sandbox probe did not run");
}

function productProbe(): Promise<SandboxProbeResult> {
  sharedProbe ??= probeSandboxWithRetry(() =>
    invokeCommand<SandboxProbeResult>("probe_sandbox"),
  ).catch((error) => {
    sharedProbe = null;
    throw error;
  });
  return sharedProbe;
}

function initialAvailability(): SandboxAvailability {
  if (isMacOS) {
    return {
      available: true,
      definitive: true,
      resolved: true,
      code: "available",
      backend: "macos_seatbelt",
    };
  }
  if (!isWindows && !isLinux) {
    return {
      available: false,
      definitive: true,
      resolved: true,
      code: "platform_unsupported",
      backend: "none",
    };
  }
  return { available: false, definitive: false, resolved: false };
}

/**
 * Product-facing sandbox availability. Linux and Windows consume the same
 * Agent probe used by execution. Explicit unavailable results are definitive;
 * exhausted transport failures preserve the saved tier for a later retry.
 */
export function useSandboxAvailability(): SandboxAvailability {
  const [availability, setAvailability] = useState(initialAvailability);

  useEffect(() => {
    if (!isWindows && !isLinux)
      return;
    let current = true;
    void productProbe()
      .then((result) => {
        if (current) {
          setAvailability({
            available: result.available,
            definitive: true,
            resolved: true,
            code: result.code,
            backend: result.backend,
          });
        }
      })
      .catch(() => {
        if (current) {
          setAvailability({
            available: false,
            definitive: false,
            resolved: true,
            code: "probe_transport_error",
          });
        }
      });
    return () => {
      current = false;
    };
  }, []);

  return availability;
}
