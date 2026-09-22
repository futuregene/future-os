const $ = (id) => document.getElementById(id);
const queryEl = $("query");
const cardsEl = $("cards");
const verdictEl = $("verdict");
const gateEl = $("gate");
const metaEl = $("meta");
const allListEl = $("all-list");
const debugPreEl = $("debug-pre");

let minChars = 6;
let gateThreshold = 0.15;
// Fallback until /api/status answers; the server reports the real value (default: whole roster).
let chunkSize = 254;
let timer = null;
let controller = null;
let seq = 0;
let lastPayload = null;

const pct = (value) => (value === null || value === undefined ? "—" : `${(value * 100).toFixed(1)}%`);

function h(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value);
  }
  for (const child of children.flat()) if (child) node.append(child);
  return node;
}

const bar = (value, klass = "") =>
  h("div", { class: `bar ${klass}` }, h("i", { style: `width:${Math.max(0, Math.min(1, value || 0)) * 100}%` }));

const EXAMPLES = [
  "把这份季度报告做成一版路演用的 PPT，要能直接导出 PDF",
  "帮我查一下 BRCA1 这个变异在 ClinVar 里的临床意义",
  "把这个 PDF 里面的表格抽成 markdown",
  "从多序列比对结果推断一个系统发育树并画图",
  "解释一下什么是 monad",
  "把这三张卡片加到我们的 Trello backlog 里",
  "我在做单细胞 RNA 测序的聚类分析，想找个合适的流程",
  "write a blog post about our new release and deploy it",
];

// ----------------------------------------------------------------- data access

async function post(body, signal) {
  const response = await fetch("/api/suggest", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  return response.json();
}

function schedule() {
  const query = queryEl.value.trim();
  clearTimeout(timer);

  if (query.length < minChars) {
    controller?.abort();
    lastPayload = null;
    renderIdle();
    return;
  }
  // One request per pause. The pipeline is a single Choice call, so there is no second, slower
  // stage to schedule behind this one.
  timer = setTimeout(() => run(query), 350);
}

async function run(query) {
  controller?.abort();
  controller = new AbortController();
  const mine = ++seq;
  metaEl.textContent = "排名中…";

  let payload;
  try {
    payload = await post({ query, limit: 200 }, controller.signal);
  } catch (error) {
    if (error.name === "AbortError") {
      console.log("[run] aborted");
      return;
    }
    console.error(`[run] failed: ${error.message}`);
    metaEl.textContent = `请求失败：${error.message}`;
    return;
  }
  if (mine !== seq || queryEl.value.trim() !== query) {
    console.log(`[run] dropped (seq ${mine}/${seq}, query ${JSON.stringify(queryEl.value.trim())})`);
    return;
  }

  lastPayload = payload;
  console.log(`[run] backend=${payload.backend} mode=${payload.mode} gate=${payload.gate.mean} verdict=${payload.verdict ?? "-"} top=${payload.top[0]?.name}`);
  render(payload);
}

async function loadStatus() {
  const status = await (await fetch("/api/status")).json();
  minChars = status.minQueryChars;
  gateThreshold = status.gateThreshold;
  chunkSize = status.thresholds?.chunkSize ?? chunkSize;

  $("backend-dot").className = `dot ${status.mode === "typesafe" ? "live" : "local"}`;
  $("status").textContent =
    `${status.mode === "typesafe" ? "Jev 在线" : "本地回退"} · ${status.model || "bm25"} · ` +
    `${status.roster.total} 个技能 (builtin ${status.roster.builtin} + third-party ${status.roster.thirdParty})`;

  const banner = $("banner");
  if (status.mode === "typesafe") banner.hidden = true;
  else {
    banner.hidden = false;
    banner.textContent = `⚠️ 未走 Jev：${status.note}\n当前显示的是本地 BM25 启发式排序，只是为了让界面能点。把可用的 TYPESAFE_API_KEY 给 server 重启即会自动切回 Jev（同一个 UI、同样的单次 Choice 判定）。`;
  }
  renderIdle();
}

// -------------------------------------------------------------------- render

function renderIdle() {
  verdictEl.innerHTML = "";
  verdictEl.append(
    h("span", { class: "verdict-icon", text: "💤" }),
    h(
      "div",
      {},
      h("div", { class: "verdict-title", text: "等待输入…" }),
      h("div", { class: "verdict-sub", text: `至少输入 ${minChars} 个字符开始推荐` }),
    ),
  );
  cardsEl.innerHTML = "";
  gateEl.innerHTML = "";
  allListEl.innerHTML = "";
  debugPreEl.textContent = "—";
  metaEl.textContent = "—";
}

function renderVerdict(payload) {
  const top = payload.top?.[0];
  const local = payload.backend !== "typesafe";
  const icon = h("span", { class: "verdict-icon" });
  let title;
  let sub;

  if (!payload.gate_passes) {
    icon.textContent = "🤔";
    title = h("div", { class: "verdict-title", text: "判定：这一轮不需要技能" });
    sub =
      `Choice 把 ${pct(payload.gate.noneProbability)} 的概率给了 none_of_these（阈值 ${gateThreshold}），` +
      "所以判定为不需要技能；下面仍然按概率列出最接近的几个候选，方便你判断这个拒答是否合理。";
  } else {
    icon.textContent = local ? "🧪" : "✅";
    title = h(
      "div",
      { class: "verdict-title" },
      local ? "本地启发式候选（非 Jev）：" : "推荐：",
      h("span", { class: "name", text: payload.verdict }),
      top?.nameZh ? h("span", { class: "zh", text: ` ${top.nameZh}` }) : "",
    );
    sub =
      `Choice 给它的概率 ${pct(top?.p)}（none_of_these 只有 ${pct(payload.gate.noneProbability)}，阈值 ${gateThreshold}）。` +
      "第 2、3 名列在下面 —— Choice 的概率会饱和，所以“第二名紧跟其后”只说明模型不确定，不说明它更合适。";
  }

  verdictEl.innerHTML = "";
  verdictEl.append(icon, h("div", {}, title, h("div", { class: "verdict-sub", text: sub || "" })));
}

function renderCards(payload) {
  cardsEl.innerHTML = "";
  payload.top.forEach((entry, index) => {
    const isWinner = payload.verdict === entry.name;
    const card = h("div", { class: `card${isWinner ? " winner" : ""}` });

    card.append(
      h(
        "div",
        { class: "card-head" },
        h("span", { class: "rank", text: `#${index + 1}` }),
        h("span", { class: "name", text: entry.name }),
        entry.nameZh ? h("span", { class: "zh", text: entry.nameZh }) : "",
        h("span", { class: `tag${entry.builtin ? " builtin" : ""}`, text: entry.builtin ? "builtin" : "third-party" }),
        entry.category ? h("span", { class: "tag", text: entry.category }) : "",
        isWinner ? h("span", { class: "tag", text: "🏆 winner" }) : "",
      ),
      h("div", { class: "card-desc", text: entry.description }),
    );

    // The only number the decision reads: how much probability the Choice gave this candidate.
    card.append(
      h(
        "div",
        { class: "bars" },
        h(
          "div",
          { class: "bar-row" },
          h("span", { text: "选中概率" }),
          bar(entry.p, isWinner ? "fit" : ""),
          h("span", { class: "value", text: pct(entry.p) }),
        ),
      ),
    );
    cardsEl.append(card);
  });
}

function renderGate(payload) {
  const gate = payload.gate ?? {};
  gateEl.innerHTML = "";

  const none = gate.noneProbability;
  const refused = !payload.gate_passes;
  gateEl.append(
    h(
      "div",
      { class: "gate-item" },
      h("span", { text: "① 每块 noul / choice" }),
      h("b", { text: gate.chunkCount ? `${gate.chunkCount} 块 × ${chunkSize} 个技能` : "—" }),
    ),
    h(
      "div",
      { class: "gate-item" },
      h("span", { text: "② 各块弃权（选了 none）" }),
      h("b", { text: gate.declinedChunks === null || gate.declinedChunks === undefined ? "—" : `${gate.declinedChunks} / ${gate.chunkCount}` }),
    ),
    h(
      "div",
      { class: "gate-item" },
      h("span", { text: "③ 进入最终 Choice 的候选" }),
      h("b", { text: gate.survivorCount ?? "—" }),
    ),
    h(
      "div",
      { class: "gate-item" },
      h("span", { text: `④ 最终 Choice 给 none 的概率（阈值 ${gateThreshold}）` }),
      h("b", { text: pct(none) }),
      bar(none ?? 0, refused ? "low" : "fit"),
    ),
    h(
      "div",
      { class: "gate-item" },
      h("span", { text: "判定" }),
      h("b", { text: refused ? "none 占优 → 拒答" : `有候选 → 推荐 ${payload.verdict}` }),
      bar(1, refused ? "low" : "fit"),
    ),
  );

  if (payload.backend !== "typesafe") {
    gateEl.append(h("div", { class: "gate-item" }, h("span", { text: "以上是本地回退值，不是 Jev 的输出" })));
  }
}

function renderMeta(payload) {
  const usage = payload.rank.usage || {};
  const tokens = usage.input_tokens ?? usage.prompt_tokens;
  const lines = [
    `backend ${payload.backend}${payload.rank.model ? ` · ${payload.rank.model}` : ""} · ${payload.mode}`,
    `一次调用 ${payload.rank.ms} ms · 含网络往返共 ${payload.total_ms} ms`,
    tokens ? `input tokens ${payload.rank.usage.input_tokens}` : "",
    `${payload.choices_seen} 个候选装在一个 Choice 里；判定只读 none_of_these 的概率`,
  ].filter(Boolean);
  metaEl.textContent = lines.join("\n");
}

function renderAll(payload) {
  allListEl.innerHTML = "";
  payload.ranked.forEach((entry, index) => {
    allListEl.append(
      h(
        "div",
        { class: "all-row" },
        h("span", { text: `${index + 1}` }),
        h("span", { class: "nm", text: entry.name }),
        bar(entry.p),
        h("span", { class: "value", text: pct(entry.p) }),
      ),
    );
  });
}

function renderDebug(payload) {
  debugPreEl.textContent = JSON.stringify(
    {
      backend: payload.backend,
      model: payload.rank.model,
      "stage1 · 请求": payload.rank.request,
      "stage1 · 门控": payload.gate,
      "stage1 · 各块弃权": payload.rank?.declinedChunks ?? null,
      "stage1 · 幸存候选": payload.rank?.survivors ?? null,
      "stage1 · top": payload.top.map((entry) => ({ name: entry.name, p: entry.p })),
      "stage1 · usage": payload.rank.usage,
      判定: {
        verdict: payload.verdict,
        gate_passes: payload.gate_passes,
        gate_threshold: payload.gate_threshold,
        none_probability: payload.gate.noneProbability,
      },
      说明: "只有一次调用。曾经的第二次调用（复核 top-3 的 fit）已移除，理由见 README / REPORT §2.2.5。",
    },
    null,
    2,
  );
}

function render(payload) {
  if (payload.skipped) return renderIdle();
  renderVerdict(payload);
  renderCards(payload);
  renderGate(payload);
  renderAll(payload);
  renderDebug(payload);
  renderMeta(payload);
}

// -------------------------------------------------------------------- wiring

for (const example of EXAMPLES) {
  $("examples").append(h("button", { class: "chip", text: example, onclick: () => { queryEl.value = example; queryEl.focus(); schedule(); } }));
}

queryEl.addEventListener("input", schedule);
$("verify-now").addEventListener("click", () => {
  const query = queryEl.value.trim();
  if (query.length >= minChars) run(query);
});

for (const tab of document.querySelectorAll(".tab")) {
  tab.addEventListener("click", () => {
    for (const other of document.querySelectorAll(".tab")) other.classList.toggle("active", other === tab);
    for (const name of ["candidates", "all", "debug"]) $(`tab-${name}`).hidden = name !== tab.dataset.tab;
  });
}

document.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
    event.preventDefault();
    const query = queryEl.value.trim();
    if (query.length >= minChars) run(query);
  }
});

loadStatus().then(() => queryEl.focus());
