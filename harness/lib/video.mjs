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

/**
 * One frame's display duration, in seconds.
 *
 * The floor was 40 ms, which silently stretched the clip whenever Chrome emitted
 * frames faster than 25 fps: a 2.9 s case produced a 4.7 s file. The floor is now
 * a real minimum (10 ms), and the 5 s cap on a single frame stays — a long idle
 * gap should not become a minute of frozen picture. Both keep the mapping in
 * `videoClock()` necessary, so it is computed from the same rule.
 */
export function frameDuration(frame, next) {
  return Math.min(5, Math.max(0.01, ((next?.ts ?? frame.ts + 120) - frame.ts) / 1000));
}

/**
 * When each moment of the run appears in the finished clip.
 *
 * The clip is not the wall clock: a single frame is capped at 5 s and no frame is
 * shorter than 10 ms, so "step 12 happened at 18.4 s" is a claim about the video,
 * not about the run. The analysis labels its screenshots with video time (that is
 * what the filmstrip is sampled from), and this clock is what makes that label
 * true. Sampled to at most 400 points — the interpolation error is well under the
 * spacing between two filmstrip frames.
 */
export function videoClock(frames) {
  if (!frames?.length)
    return null;
  const samples = [];
  let at = 0;
  const stride = Math.max(1, Math.ceil(frames.length / 400));
  for (let i = 0; i < frames.length; i += 1) {
    if (i % stride === 0 || i === frames.length - 1)
      samples.push([frames[i].ts, Number(at.toFixed(3))]);
    at += frameDuration(frames[i], frames[i + 1]);
  }
  return { startMs: frames[0].ts, endMs: frames.at(-1).ts, durationMs: Math.round(at * 1000), samples };
}

export function muxVideo(framesDir, frames, output) {
  if (!frames?.length)
    return { ok: false, reason: "no frames" };
  const lines = [];
  for (let i = 0; i < frames.length; i += 1) {
    const frame = frames[i];
    const duration = frameDuration(frame, frames[i + 1]);
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
