import type { FusionReadRequest, FusionStep } from "@future-os/thread-projection";
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

/** Paging is a transport detail: a report is displayed only after reassembly. */
export async function readFusionStep(input: FusionReadRequest & { stepId: string }, resolveTarget?: FusionResolver, signal?: AbortSignal): Promise<FusionStep> {
  const pages: string[] = [];
  let offset = 0;
  for (;;) {
    if (signal?.aborted)
      throw new DOMException("Discussion report read cancelled", "AbortError");
    const page = await readFusion<FusionStep>({ ...input, offset }, resolveTarget);
    if (signal?.aborted)
      throw new DOMException("Discussion report read cancelled", "AbortError");
    pages.push(page.transcript ?? "");
    const next = page.next_offset ?? 0;
    const total = page.total_bytes ?? 0;
    if (!Number.isSafeInteger(next) || !Number.isSafeInteger(total) || next < offset || next > total)
      throw new Error("Invalid discussion report cursor");
    if (next === total)
      return { ...page, transcript: pages.join("") };
    if (next === offset)
      throw new Error("Discussion report cursor did not advance");
    offset = next;
  }
}
