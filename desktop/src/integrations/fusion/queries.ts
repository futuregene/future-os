import type { FusionReadRequest } from "@future-os/thread-projection";
import { invokeCommand } from "../tauri/invoke";

export type FusionResolver = (input: FusionReadRequest) => Promise<string | null>;

/** Keep remote reads on their original host; never fall back to local data. */
export async function readFusion<T>(input: FusionReadRequest, resolveTarget?: FusionResolver): Promise<T> {
  if (resolveTarget) {
    const result = await resolveTarget({ ...input, fusion: true });
    if (result === null)
      throw new Error("Fusion read unavailable");
    return JSON.parse(result) as T;
  }
  return invokeCommand<T>("get_fusion", { ...input });
}
