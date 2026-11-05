#!/usr/bin/env node
/**
 * Judge a recorded UI flow: hand the video to a video-capable model and ask whether
 * the run matched the expected flow, plus what it saw go wrong.
 *
 * This is the "watch the recording" layer on top of the scripted assertions in
 * harness/: assertions know exactly what should happen; the judge notices what the
 * assertions do not encode — a stuck error banner, a contradictory empty state, a
 * panel that shows the wrong content.
 *
 * Measured on the desktop renderer (2026-11-06): three injected anomalies were all
 * reported with the right step, and a clean recording produced no false positive.
 * A judgement takes 150-180 s, so run it per case asynchronously, not per assertion.
 *
 * Usage:
 *   FUTURE_API_KEY=... node harness/scripts/video-judge.mjs \
 *     --video /tmp/run.mp4 --expect "依次打开 模型选择 / 技能列表 / 定时任务" [--json out.json]
 *
 * Exit code: 0 = verdict pass, 1 = verdict fail, 2 = usage/transport error.
 */
import fs from 'node:fs';
import path from 'node:path';

const argv = process.argv.slice(2);
const arg = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && argv[i + 1] ? argv[i + 1] : d; };

const VIDEO = arg('video');
const EXPECT = arg('expect');
const MODEL = arg('model', 'qwen3.8-max');
const API = arg('api', 'https://future-os.cn/api/v1/chat/completions');
const OUT = arg('json');
const KEY = process.env.FUTURE_API_KEY;

if (!VIDEO || !EXPECT) { console.error('need --video and --expect'); process.exit(2); }
if (!KEY) { console.error('missing FUTURE_API_KEY'); process.exit(2); }
if (!fs.existsSync(VIDEO)) { console.error(`no such video: ${VIDEO}`); process.exit(2); }

const b64 = fs.readFileSync(VIDEO).toString('base64');
const prompt =
  `这是一段自动化测试的录屏。测试目标是：${EXPECT}\n\n` +
  '请逐步核对录屏里的实际操作，然后只输出一个 JSON 对象，不要 markdown：\n' +
  '{"verdict":"pass|fail","summary":"一句话","issues":[{"step":序号,"what":"现象","why":"为什么算异常"}]}\n' +
  '判定为 fail 的标准：界面报错、内容与目标不符、控件点了没反应、状态残留或遮挡、' +
  '同一界面出现自相矛盾的内容。与测试目标无关的既有浮层不算。';

const res = await fetch(API, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${KEY}` },
  body: JSON.stringify({
    model: MODEL,
    messages: [{ role: 'user', content: [
      { type: 'text', text: prompt },
      { type: 'video_url', video_url: { url: `data:video/mp4;base64,${b64}` } }] }],
    max_tokens: 800, temperature: 0.2,
  }),
});
if (!res.ok) { console.error(`HTTP ${res.status}: ${(await res.text()).slice(0, 300)}`); process.exit(2); }

const txt = (await res.json()).choices?.[0]?.message?.content || '';
const obj = JSON.parse((txt.match(/\{[\s\S]*\}/) || ['{}'])[0]);
const verdict = obj.verdict === 'pass' ? 'pass' : 'fail';

if (OUT) {
  fs.writeFileSync(path.resolve(OUT),
    JSON.stringify({ video: VIDEO, expect: EXPECT, model: MODEL, raw: txt, ...obj }, null, 2));
}
console.log(JSON.stringify({ video: path.basename(VIDEO), verdict,
  summary: obj.summary, issues: obj.issues || [] }, null, 1));
process.exit(verdict === 'pass' ? 0 : 1);
