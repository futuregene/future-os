// Text/tool subset of OpenCode e03db9bc message-v2.ts + compaction.ts.
// Uses the exact upstream AI SDK version, NOT a hand-written model-message
// serializer. No network or model calls. Omitted media/provider metadata in the
// frozen corpus cannot be reconstructed and is explicitly outside this subset.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const require = createRequire(path.join(input.sdkRoot, 'package.json'));
if (require('ai/package.json').version !== '6.0.168') throw new Error('SDK version mismatch');
const { convertToModelMessages } = require('ai');
const messages = input.messages;
const results = new Map();
for (const [i, message] of messages.entries()) {
  for (const block of message.content) if (block.type === 'tool_result') {
    if (results.has(block.tool_call_id)) throw new Error('ambiguous tool result id');
    results.set(block.tool_call_id, { ...block, index: i });
  }
}
const entries = [];
const toolNames = new Set();
for (const [index, message] of messages.entries()) {
  if (message.role === 'tool') continue;
  const parts = [], serialized = [];
  for (const block of message.content) {
    if (block.type === 'text') {
      if (message.role !== 'user' || block.text !== '') parts.push({ type: 'text', text: block.text });
      serialized.push(`[${message.role === 'user' ? 'User' : 'Assistant'}]: ${block.text}`);
    } else if (block.type === 'tool_call') {
      const result = results.get(block.id);
      if (!result) throw new Error('unanswered tool call in normalized input');
      toolNames.add(block.name);
      parts.push({ type: `tool-${block.name}`, state: result.is_error ? 'output-error' : 'output-available',
        toolCallId: block.id, input: block.args,
        ...(result.is_error ? { errorText: result.content } : { output: result.content }) });
      serialized.push(`[Assistant tool call]: ${block.name}(${JSON.stringify(block.args)})`);
      const value = result.content;
      serialized.push(result.is_error ? `[Tool error]: ${value}` :
        `[Tool result]: ${value.length <= 2000 ? value : value.slice(0, 2000) + '\n[truncated]'}`);
      results.delete(block.id);
    } else throw new Error(`unsupported frozen content: ${block.type}`);
  }
  if (parts.length) entries.push({ start: index, ui: { id: String(index), role: message.role, parts }, text: serialized.join('\n') });
}
if (results.size) throw new Error('orphan result in normalized input');
const tools = Object.fromEntries([...toolNames].map(name => [name, { toModelOutput: ({ output }) => ({ type: 'text', value: output }) }]));
const modelMessages = async items => convertToModelMessages(items.map(e => e.ui), { tools });
// Exact upstream Token.estimate(JSON.stringify(toModelMessages(...))).
const size = async items => Math.max(0, Math.round(JSON.stringify(await modelMessages(items)).length / 4));
const all = entries.flatMap((e, i) => e.ui.role === 'user' ? [{ start: i, end: entries.length }] : []);
for (let i = 0; i + 1 < all.length; i++) all[i].end = all[i+1].start;
const usable = Math.max(0, input.window - Math.min(20000, input.maxOutput));
const budget = Math.min(15000, Math.max(2000, Math.floor(usable * .25)));
let total = 0, keep;
for (const turn of all.toReversed()) {
  const cost = await size(entries.slice(turn.start, turn.end));
  if (total + cost <= budget) {
    total += cost; keep = turn.start; continue;
  }
  const remaining = budget - total;
  if (remaining > 0 && turn.end - turn.start > 1) {
    for (let start = turn.start + 1; start < turn.end; start++) {
      if (await size(entries.slice(start, turn.end)) > remaining) continue;
      keep = start; break;
    }
  }
  break;
}
// Upstream: no keep, or keep.start === 0 => head=all, tail_start_id undefined.
const tail = keep === undefined || keep === 0 ? entries.length : keep;
const tailMessageIndex = tail < entries.length ? entries[tail].start : messages.length;
console.log(JSON.stringify({
  sdkVersion: '6.0.168', budget, tailEntry: tail, entries: entries.length,
  head: entries.slice(0, tail).map(e => e.text).join('\n\n'),
  tailMessages: messages.slice(tailMessageIndex),
  tailModelMessages: await modelMessages(entries.slice(tail)),
  tailEstimatedTokens: await size(entries.slice(tail)),
  allModelMessages: input.includeAll ? await modelMessages(entries) : undefined,
}));
