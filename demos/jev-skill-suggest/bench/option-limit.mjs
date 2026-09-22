#!/usr/bin/env node
// Where exactly is the option cap on a Choice, and does none_of_these count against it?
//
// suggest.mjs's CHUNK_SIZE comment claims "a Choice accepts at most 255 options, so a roster past
// ~254 skills has to be split". That "~254" is the whole reason CHUNK_SIZE exists, so it should be
// measured rather than asserted — and the answer decides whether CHUNK_SIZE may be 255 (255 skills
// + one none option = 256 options) or must stay 254.
//
//   TYPESAFE_API_KEY=... node bench/option-limit.mjs
//
// Costs one tiny request per size (a few hundred tokens each; the option text is a stub, not a
// real description — only the COUNT is being measured).
import { JevClient } from "../jev.mjs";

const sizes = process.argv.slice(2).map(Number).filter(Boolean);
const tested = sizes.length ? sizes : [254, 255, 256, 257];

const client = new JevClient({ apiKey: process.env.TYPESAFE_API_KEY });
if (!client.apiKey) {
  console.error("TYPESAFE_API_KEY is not set");
  process.exit(2);
}

/** A Choice with `total` options, the last one being none_of_these (the shape #chunkQuestions builds). */
const request = (total) => {
  const criteria = {};
  for (let i = 0; i < total - 1; i += 1) criteria[`skill_${String(i).padStart(3, "0")}`] = `Stub skill number ${i}.`;
  criteria.none_of_these = "No skill in this list would help with the request";
  return {
    state: { request: "add a line to the notes file" },
    questions: {
      chunk_0: {
        type: "choice",
        instructions: {
          question:
            "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
          how_to_judge:
            'Pick the closest match if any is plausible, otherwise choose "none_of_these". ' +
            "Choosing it is a normal answer here, not a fallback.",
        },
        criteria,
      },
    },
  };
};

const rows = [];
for (const total of tested) {
  const body = request(total);
  try {
    const { answers, usage } = await client.systemOne(body);
    const answer = answers?.chunk_0;
    const options = Object.keys(answer?.probabilities ?? {}).length;
    rows.push({ total, ok: true, status: 200, echoedOptions: options, pick: answer?.choice ?? null, tokens: usage?.input_tokens ?? null });
  } catch (error) {
    rows.push({
      total,
      ok: false,
      status: error.status ?? "network",
      message: error.body?.detail?.message || error.body?.message || error.message,
    });
  }
}

console.log("\n一个 Choice 里放 N 个选项（最后一个是 none_of_these）：\n");
console.log("选项总数 | 结果 | 响应里的概率项数 | 选中 | input_tokens | 报错");
console.log("--------|------|----------------|------|--------------|------");
for (const r of rows) {
  console.log(
    `${String(r.total).padStart(7)} | ${r.ok ? "OK  " : "FAIL"} | ${String(r.echoedOptions ?? "-").padStart(14)} | ` +
      `${String(r.pick ?? "-").padEnd(4)} | ${String(r.tokens ?? "-").padStart(12)} | ${r.message ?? ""}`,
  );
}

const ok = rows.filter((r) => r.ok).map((r) => r.total);
const failed = rows.filter((r) => !r.ok).map((r) => r.total);
console.log(
  `\n最大可用：${ok.length ? Math.max(...ok) : "—"}${failed.length ? `　首次失败：${Math.min(...failed)}` : ""}`,
);
