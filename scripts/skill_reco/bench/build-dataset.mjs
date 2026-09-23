#!/usr/bin/env node
// Builds the 100-question set.
//
//   65 "covered" questions: one per sampled skill, written by the gold model from that
//      skill's own SKILL.md. The question must NOT name the skill, and must not use the
//      vocabulary that only that skill's roster line uses — otherwise the question would
//      be answerable by string matching. Both are checked automatically (`validate`) and a
//      rejected attempt is retried.
//   35 "uncovered" questions: hand-written, about things no skill in the roster serves
//      (small talk, general knowledge, tool-specific asks like booking a flight).
//
// The skill a question was written from is recorded as `source_skill` for bookkeeping only.
// It is NOT the label: the label comes from a separate ground-truth pass (ground-truth.mjs)
// that shows the gold model the roster and the question and nothing else.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { MODELS, QUESTIONS_FILE, ROSTER_FILE, cacheFor, extractJson, llm, pool, writeJson } from "./common.mjs";
import { loadRoster } from "../roster.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));

writeJson(ROSTER_FILE, {
  total: roster.skills.length,
  builtin: roster.builtinCount,
  thirdParty: roster.thirdPartyCount,
  skills: roster.skills.map(({ name, builtin, category, nameZh, description, descriptionZh }) => ({
    name,
    builtin,
    category,
    nameZh,
    description,
    descriptionZh,
  })),
});

// ------------------------------------------------------------------- the sampling

const POSITIVE_TARGET = Number(process.env.POSITIVES ?? 65);
const builtin = roster.skills.filter((s) => s.builtin).sort((a, b) => a.name.localeCompare(b.name));
const thirdParty = roster.skills.filter((s) => !s.builtin).sort((a, b) => a.name.localeCompare(b.name));

/** Even stride, so the sample spreads over the whole third-party catalog instead of a prefix. */
const stride = (list, count) => {
  if (count >= list.length) return list;
  const step = list.length / count;
  return Array.from({ length: count }, (_, i) => list[Math.floor(i * step)]);
};

const sampled = [...builtin, ...stride(thirdParty, POSITIVE_TARGET - builtin.length)];
console.log(`sampled ${sampled.length} skills (${builtin.length} builtin + ${sampled.length - builtin.length} third-party)`);

// ------------------------------------------------------- the "don't name it" checks

const tokensOf = (text) =>
  String(text ?? "")
    .toLowerCase()
    .split(/[^a-z0-9+#.]+/)
    .filter((token) => token.length >= 4);

/** `scanpy` should also block "scan py", "ScanPy", "scan-py" and the bare "py" is too short. */
const nameVariants = (name) => {
  const base = name.toLowerCase();
  return new Set([base, base.replace(/-/g, " "), base.replace(/-/g, ""), base.replace(/^future-/, "")]);
};

/**
 * Terms that give the skill away: they come from this skill's roster line and appear nowhere in
 * any other skill's documentation. "produce" or "write" fail that test (other skills use them
 * too), while "scanpy"-style names do not. Built once per skill.
 */
function distinctiveTerms(skill, others) {
  const mine = new Set(tokensOf(`${skill.name} ${skill.description} ${skill.category}`));
  const elsewhere = new Set();
  for (const other of others) {
    for (const token of tokensOf(`${other.name} ${other.description} ${other.excerpt}`)) elsewhere.add(token);
  }
  return new Set([...mine].filter((token) => token.length >= 5 && !elsewhere.has(token)));
}

function validate(question, skill, others) {
  const haystack = question.toLowerCase();
  const hits = [];
  for (const variant of nameVariants(skill.name)) {
    if (variant.length >= 4 && haystack.includes(variant)) hits.push(`names the skill ("${variant}")`);
  }
  for (const term of distinctiveTerms(skill, others)) {
    if (new RegExp(`\\b${term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`).test(haystack)) hits.push(`uses a term no other skill's docs have ("${term}")`);
  }
  return hits;
}

// ---------------------------------------------------------------- the 35 negatives

const NEGATIVES = [
  { en: "hey, how's it going? just checking in", zh: "在吗？随便聊聊，今天过得怎么样" },
  { en: "tell me a joke about programmers", zh: "给我讲个程序员的冷笑话" },
  { en: "what should I have for dinner tonight?", zh: "晚饭吃什么好呢" },
  { en: "thanks, that was really helpful!", zh: "谢谢，帮了大忙了！" },
  { en: "are you able to remember what we talked about yesterday?", zh: "你还记得我们昨天聊了什么吗" },
  { en: "explain what a monad is, simply", zh: "简单解释一下什么是 monad" },
  { en: "who wrote the novel Middlemarch?", zh: "《米德尔马契》这本小说是谁写的" },
  { en: "why is the sky blue?", zh: "为什么天空是蓝色的" },
  { en: "summarize the history of the Roman Republic for me", zh: "给我讲讲罗马共和国的历史" },
  { en: "what is the difference between TCP and UDP?", zh: "TCP 和 UDP 有什么区别" },
  { en: "rename the function fetchUser to loadUser across this repo", zh: "帮我把仓库里所有 fetchUser 重命名成 loadUser" },
  { en: "my CSS grid layout collapses on Safari, can you debug it?", zh: "我的 CSS grid 布局在 Safari 上塌了，帮我看看" },
  { en: "add pagination to this REST endpoint", zh: "给这个 REST 接口加上分页" },
  { en: "split this 800-line Python file into modules", zh: "把这个 800 行的 Python 文件拆成几个模块" },
  { en: "write a git pre-commit hook that runs prettier", zh: "写一个跑 prettier 的 git pre-commit 钩子" },
  { en: "review the error handling in this Go service", zh: "看看这个 Go 服务的错误处理有没有问题" },
  { en: "why is my docker build so slow, it takes 6 minutes", zh: "我的 docker build 为什么要 6 分钟，太慢了" },
  { en: "book me a flight to Berlin next Tuesday", zh: "帮我订下周二去柏林的机票" },
  { en: "add these three cards to our Trello backlog", zh: "把这三张卡片加到我们的 Trello 看板上" },
  { en: "post this update to our Mastodon account", zh: "把这条更新发到我们的 Mastodon 账号" },
  { en: "create a Spotify playlist from my liked songs", zh: "用我点过赞的歌建一个 Spotify 歌单" },
  { en: "send a Slack DM to the on-call engineer", zh: "给值班工程师发个 Slack 私聊" },
  { en: "file an expense report in Concur for this receipt", zh: "帮我在 Concur 里报销这张收据" },
  { en: "sync my calendar with the team Notion page", zh: "把我的日历和团队 Notion 页面对齐" },
  { en: "order more coffee beans from the usual shop", zh: "从常去的那家店再买点咖啡豆" },
  { en: "what is the biological function of the BRCA1 protein in general?", zh: "BRCA1 蛋白一般有什么生物学功能" },
  { en: "explain what single-cell sequencing is at a high level", zh: "科普一下单细胞测序是什么" },
  { en: "is it true that getting cold makes you sick?", zh: "受凉真的会感冒吗" },
  { en: "how do I read a scientific paper efficiently?", zh: "怎么才能高效地读一篇论文" },
  { en: "what makes a good research question?", zh: "一个好的研究问题应该具备什么特点" },
  { en: "recommend a good textbook for learning statistics", zh: "推荐一本学统计的好教材" },
  { en: "what's the current state of the art in protein folding? just curious", zh: "现在蛋白质折叠领域进展到哪一步了，纯好奇" },
  { en: "what can you actually do for me?", zh: "你到底能帮我做什么" },
  { en: "your previous answer was too long, be brief from now on", zh: "你刚才的回答太长了，之后简短点" },
  { en: "switch to a cheaper model for this conversation", zh: "这次对话换个便宜点的模型" },
].map((pair, i) => ({
  id: `n${String(i + 1).padStart(3, "0")}`,
  text: i % 2 === 0 ? pair.en : pair.zh,
  lang: i % 2 === 0 ? "en" : "zh",
  origin: "hand-written uncovered",
}));

// ------------------------------------------------------------- generate positives

const SYSTEM = [
  "You write benchmark questions for a skill-routing evaluation.",
  "You are given one agent skill's documentation. Write ONE request that an ordinary user",
  "would type into a general-purpose AI assistant, where that skill is exactly what the",
  "request needs.",
  "",
  "Hard rule — the question must never give the skill away:",
  "- Do not name the skill.",
  "- Do not name any tool, package, command, file format, dataset, service or API that appears",
  "  in the documentation.",
  "- Describe the situation and the wanted outcome in the user's own everyday words; leave the",
  "  method entirely to the assistant.",
  "",
  "Also:",
  "- Sound like a real user, one or two sentences.",
  "- Ask for a concrete deliverable or operation, not a general question about the topic.",
  "- It must not be answerable from general knowledge alone: it has to require doing the work.",
].join("\n");

const SCHEMA = [
  "Reply with ONLY this JSON object, no other text:",
  '{"question": "<one user request, no skill/tool names>", "forbidden_terms_used": ["<any>"]}',
].join("\n");

const cache = cacheFor("questions");
const pending = sampled.filter((skill) => !cache.has(skill.name));
console.log(`generating ${pending.length} questions (${sampled.length - pending.length} cached)`);

const attempts = new Map();

await pool(
  pending,
  async (skill) => {
    const others = sampled.filter((other) => other.name !== skill.name);
    const lang = Math.random() < 0.5 ? "Chinese" : "English";
    let best = null;
    let rejections = [];
    let blocked = [];
    for (let attempt = 1; attempt <= 5; attempt += 1) {
      const prompt = [
        `Write the request in ${lang}.`,
        "",
        `The skill is named "${skill.name}" (category: ${skill.category}).`,
        "Do not use that name, or any word from the documentation that refers to it, in the question.",
        blocked.length ? `You have used these words in a previous attempt; they are not allowed: ${blocked.join(", ")}.` : "",
        "Its documentation begins:",
        "-----",
        skill.excerpt.slice(0, 1500),
        "-----",
        "",
        SCHEMA,
      ].filter(Boolean).join("\n");
      const { text, durationMs, usage } = await llm({ model: MODELS.gold, prompt, systemPrompt: SYSTEM, thinking: "low" });
      const parsed = extractJson(text);
      const question = parsed?.question?.trim();
      if (!question) {
        rejections.push("no question in response");
        continue;
      }
      const hits = validate(question, skill, others);
      if (!hits.length) {
        best = { question, hits: [], attempts: attempt, durationMs, usage };
        break;
      }
      rejections.push(`${hits.join("; ")} :: ${question.slice(0, 60)}`);
      best = { question, hits, attempts: attempt, durationMs, usage };
      // Name the offending words in the next attempt so it can avoid them deliberately.
      blocked = [...new Set([...blocked, ...hits.map((hit) => hit.match(/"([^"]+)"/)?.[1]).filter(Boolean)])];
    }
    const record = {
      source_skill: skill.name,
      category: skill.category,
      lang,
      question: best.question,
      clean: best.hits.length === 0,
      name_hits: best.hits,
      attempts: best.attempts,
      durationMs: best.durationMs,
      usage: best.usage,
    };
    attempts.set(skill.name, record);
    cache.put(skill.name, record);
    return record;
  },
  { concurrency: 6, label: "questions" },
);

const records = sampled.map((skill) => attempts.get(skill.name) ?? cache.get(skill.name));
const clean = records.filter((record) => record.clean);
console.log(`\nclean on the first accepted attempt: ${clean.length}/${records.length}`);
const retried = records.filter((record) => record.attempts > 1);
console.log(`needed more than one attempt: ${retried.length}`);
for (const record of retried.slice(0, 5)) {
  console.log(`  ${record.source_skill}: ${record.attempts} attempts, rejected because ${record.name_hits.join("; ")}`);
}

const positives = sampled.map((skill, i) => {
  const record = records.find((r) => r.source_skill === skill.name);
  return {
    id: `p${String(i + 1).padStart(3, "0")}`,
    text: record.question,
    lang: record.lang,
    origin: "written from a skill's SKILL.md, without naming it",
    source_skill: record.source_skill,
    clean: record.clean,
  };
});

const questions = [...positives, ...NEGATIVES];
writeJson(QUESTIONS_FILE, {
  generated_at: new Date().toISOString(),
  generator_model: MODELS.gold,
  note: "source_skill is bookkeeping only; the label is produced by ground-truth.mjs",
  total: questions.length,
  covered: positives.length,
  uncovered: NEGATIVES.length,
  questions,
});
console.log(`\nwrote ${QUESTIONS_FILE} (${questions.length} questions: ${positives.length} covered + ${NEGATIVES.length} uncovered)`);
console.log(`next: node ground-truth.mjs`);
