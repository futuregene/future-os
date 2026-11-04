/**
 * Turn a CDP screencast into a watchable mp4.
 *
 * Chrome only emits frames when the picture changes, so the timing file is what
 * makes pauses real: without per-frame durations a 3-second thinking pause would
 * become one frame of 100 ms. Same approach as `scripts/screenshots/cdp.mjs`.
 */
import { execFileSync } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export function muxVideo(framesDir, frames, output) {
  if (!frames?.length)
    return { ok: false, reason: "no frames" };
  const lines = [];
  for (let i = 0; i < frames.length; i += 1) {
    const frame = frames[i];
    const next = frames[i + 1];
    const duration = Math.min(5, Math.max(0.04, ((next?.ts ?? frame.ts + 120) - frame.ts) / 1000));
    lines.push(`file '${frame.file.replace(/'/g, "'\\''")}'`);
    lines.push(`duration ${duration.toFixed(3)}`);
  }
  lines.push(`file '${frames.at(-1).file.replace(/'/g, "'\\''")}'`);
  const list = join(framesDir, "frames.txt");
  writeFileSync(list, lines.join("\n"));
  try {
    execFileSync("ffmpeg", [
      "-y", "-loglevel", "error",
      "-f", "concat", "-safe", "0", "-i", list,
      "-fps_mode", "vfr",
      "-pix_fmt", "yuv420p",
      "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2",
      "-movflags", "+faststart",
      output,
    ], { stdio: ["ignore", "ignore", "pipe"] });
  }
  catch (error) {
    return { ok: false, reason: String(error?.stderr ?? error?.message ?? error).slice(0, 400) };
  }
  return { ok: existsSync(output), output };
}
