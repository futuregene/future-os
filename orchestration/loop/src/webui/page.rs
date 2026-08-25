//! The embedded single-page dashboard. Hand-rolled vanilla JS (no build
//! step, no CDN — the page must work fully offline on a loopback server).

pub const PAGE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>FutureOS · Loop Control Plane</title>
<style>
:root {
  --bg: #0b0f14; --bg-1: #10161d; --bg-2: #161e27; --bg-3: #1d2732;
  --line: #24303d; --line-2: #2e3d4d;
  --tx: #dbe4ec; --tx-2: #8fa1b3; --tx-3: #5d6f80;
  --acc: #4da3ff; --acc-dim: #1f3a56;
  --ok: #3fb96c; --ok-bg: #12301e;
  --warn: #e0a53c; --warn-bg: #38290d;
  --bad: #e05c5c; --bad-bg: #3a1616;
  --vio: #a58bff; --vio-bg: #241d3d;
  --mono: "SF Mono", ui-monospace, "Cascadia Code", Menlo, Consolas, monospace;
  --sans: -apple-system, "Segoe UI", "Inter", Roboto, "Helvetica Neue", sans-serif;
}
* { box-sizing: border-box; margin: 0; padding: 0; }
html, body { height: 100%; }
body { background: var(--bg); color: var(--tx); font: 13px/1.5 var(--sans); overflow: hidden; }
a { color: var(--acc); text-decoration: none; }
button { font: inherit; cursor: pointer; }
::-webkit-scrollbar { width: 10px; height: 10px; }
::-webkit-scrollbar-thumb { background: var(--line-2); border-radius: 5px; border: 2px solid var(--bg); }
::-webkit-scrollbar-track { background: transparent; }

#app { display: flex; flex-direction: column; height: 100vh; }

/* ── header ─────────────────────────────────────────── */
header { display: flex; align-items: center; gap: 14px; padding: 0 18px; height: 50px;
  background: var(--bg-1); border-bottom: 1px solid var(--line); flex: 0 0 auto; }
.logo { display: flex; align-items: baseline; gap: 8px; font-weight: 650; font-size: 14px; letter-spacing: .2px; }
.logo .mark { color: var(--acc); }
.logo .sub { color: var(--tx-3); font-weight: 400; font-size: 11.5px; }
.tabs { display: flex; gap: 2px; margin-left: 14px; }
.tab { padding: 5px 13px; border-radius: 6px; color: var(--tx-2); background: transparent; border: 1px solid transparent; font-size: 12.5px; }
.tab:hover { color: var(--tx); background: var(--bg-2); }
.tab.active { color: var(--tx); background: var(--bg-3); border-color: var(--line-2); }
.hdr-right { margin-left: auto; display: flex; align-items: center; gap: 12px; color: var(--tx-3); font-size: 11.5px; }
.live { display: flex; align-items: center; gap: 6px; }
.live .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--tx-3); transition: background .3s; }
.live.on .dot { background: var(--ok); box-shadow: 0 0 6px var(--ok); }
.live.off .dot { background: var(--bad); }

main { flex: 1; overflow: hidden; display: flex; }
.view { flex: 1; overflow-y: auto; padding: 20px 22px 60px; }
.hidden { display: none !important; }

/* ── stat cards ─────────────────────────────────────── */
.stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(130px, 1fr)); gap: 10px; margin-bottom: 18px; }
.stat { background: var(--bg-1); border: 1px solid var(--line); border-radius: 9px; padding: 11px 14px 9px; }
.stat .v { font: 600 21px/1.15 var(--mono); letter-spacing: -.5px; }
.stat .k { color: var(--tx-3); font-size: 10.5px; text-transform: uppercase; letter-spacing: .8px; margin-top: 3px; }
.stat.accent .v { color: var(--acc); }
.stat.warn .v { color: var(--warn); }
.stat.ok .v { color: var(--ok); }

/* ── sections ───────────────────────────────────────── */
.sect { margin-bottom: 22px; }
.sect > h2 { font-size: 12px; font-weight: 650; text-transform: uppercase; letter-spacing: 1px;
  color: var(--tx-2); margin-bottom: 10px; display: flex; align-items: center; gap: 8px; }
.sect > h2 .count { color: var(--tx-3); font-weight: 400; }
.card { background: var(--bg-1); border: 1px solid var(--line); border-radius: 10px; }

/* ── badges & chips ─────────────────────────────────── */
.badge { display: inline-flex; align-items: center; gap: 5px; padding: 1.5px 8px; border-radius: 20px;
  font: 550 10.5px/1.6 var(--sans); letter-spacing: .3px; white-space: nowrap; }
.badge::before { content: ""; width: 5px; height: 5px; border-radius: 50%; background: currentColor; }
.b-ok   { color: #6fd598; background: var(--ok-bg); }
.b-warn { color: #f0c06a; background: var(--warn-bg); }
.b-bad  { color: #f08a8a; background: var(--bad-bg); }
.b-info { color: #7cb9f5; background: var(--acc-dim); }
.b-vio  { color: #c2aeff; background: var(--vio-bg); }
.b-mut  { color: var(--tx-2); background: var(--bg-3); }
.b-mut::before { background: var(--tx-3); }
.chip { display: inline-block; padding: 0 6px; border-radius: 4px; background: var(--bg-3);
  color: var(--tx-2); font: 500 10.5px/1.7 var(--mono); }
.prio { font: 650 10.5px/1.7 var(--mono); padding: 0 6px; border-radius: 4px; }
.prio-P0 { color: #f08a8a; background: var(--bad-bg); }
.prio-P1 { color: #f0c06a; background: var(--warn-bg); }
.prio-P2 { color: var(--tx-2); background: var(--bg-3); }

/* ── attention banner ───────────────────────────────── */
.attn { border: 1px solid var(--line); border-radius: 10px; overflow: hidden; }
.attn-row { display: flex; align-items: center; gap: 12px; padding: 10px 14px; border-top: 1px solid var(--line); cursor: pointer; }
.attn-row:first-child { border-top: 0; }
.attn-row:hover { background: var(--bg-2); }
.attn-row .goal { font: 550 11.5px var(--mono); color: var(--tx-2); }
.attn-row .what { flex: 1; color: var(--tx); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.attn-row .rec { color: var(--tx-2); font-size: 12px; max-width: 46%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

/* ── goal cards ─────────────────────────────────────── */
.goals { display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 12px; }
.gcard { background: var(--bg-1); border: 1px solid var(--line); border-radius: 10px; padding: 14px 16px;
  cursor: pointer; transition: border-color .15s, transform .1s; }
.gcard:hover { border-color: var(--line-2); transform: translateY(-1px); }
.gcard.terminal { opacity: .62; }
.gcard.cancelled { opacity: .45; }
.gcard .top { display: flex; align-items: center; gap: 8px; margin-bottom: 7px; }
.gcard .gid { font: 550 11px var(--mono); color: var(--tx-3); }
.gcard .obj { font-size: 13.5px; font-weight: 550; line-height: 1.4; margin-bottom: 9px;
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.gcard .meta { display: flex; gap: 14px; color: var(--tx-3); font-size: 11px; margin-bottom: 10px; }
.gcard .meta b { color: var(--tx-2); font-weight: 550; }
.gcard .dec { display: flex; align-items: center; gap: 8px; background: var(--bg-2); border-radius: 7px;
  padding: 7px 10px; font-size: 11.5px; color: var(--tx-2); }
.gcard .dec .why { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
.bar { height: 4px; background: var(--bg-3); border-radius: 2px; margin-top: 10px; overflow: hidden; }
.bar > i { display: block; height: 100%; background: var(--acc); border-radius: 2px; }

/* ── detail layout ──────────────────────────────────── */
.backrow { display: flex; align-items: center; gap: 12px; margin-bottom: 14px; flex-wrap: wrap; }
.btn { background: var(--bg-2); color: var(--tx); border: 1px solid var(--line-2); border-radius: 7px; padding: 5px 13px; font-size: 12px; }
.btn:hover { background: var(--bg-3); }
.btn.primary { background: var(--acc); border-color: var(--acc); color: #06121f; font-weight: 600; }
.btn.danger { color: #f08a8a; border-color: #5c2b2b; }
.btn:disabled { opacity: .45; cursor: default; }
.dhead { display: flex; align-items: flex-start; gap: 14px; margin-bottom: 16px; flex-wrap: wrap; }
.dhead .obj { font-size: 17px; font-weight: 650; line-height: 1.35; flex: 1; min-width: 260px; }
.dhead .path { color: var(--tx-3); font: 11.5px var(--mono); margin-top: 4px; }
.grid2 { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.grid3 { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; }
@media (max-width: 1100px) { .grid2, .grid3 { grid-template-columns: 1fr; } }
.panel { background: var(--bg-1); border: 1px solid var(--line); border-radius: 10px; padding: 14px 16px; }
.panel h3 { font-size: 11px; font-weight: 650; text-transform: uppercase; letter-spacing: .9px; color: var(--tx-2); margin-bottom: 10px; }
.kv { display: grid; grid-template-columns: 96px 1fr; gap: 4px 14px; font-size: 12px; }
.kv dt { color: var(--tx-3); white-space: nowrap; }
.kv dd { color: var(--tx); font-family: var(--mono); font-size: 11.5px; word-break: break-word; min-width: 0; }

/* ── tables ─────────────────────────────────────────── */
table { width: 100%; border-collapse: collapse; font-size: 12px; }
th { text-align: left; color: var(--tx-3); font-weight: 550; font-size: 10.5px; text-transform: uppercase;
  letter-spacing: .7px; padding: 8px 12px; border-bottom: 1px solid var(--line); }
td { padding: 8px 12px; border-bottom: 1px solid var(--line); vertical-align: top; }
tr:last-child td { border-bottom: 0; }
tbody tr:hover { background: var(--bg-2); }
td .sub { color: var(--tx-3); font-size: 11px; margin-top: 2px; }
.twrap { background: var(--bg-1); border: 1px solid var(--line); border-radius: 10px; overflow: hidden; }
.mono { font-family: var(--mono); font-size: 11.5px; }
.ttitle { max-width: 420px; }
.ttitle .t { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.clickable { cursor: pointer; }

/* ── graph ──────────────────────────────────────────── */
.gwrap { display: flex; gap: 0; }
.gwrap svg { flex: 0 0 auto; }
.gnode { cursor: pointer; }
.gnode rect { fill: var(--bg-2); stroke: var(--line-2); rx: 6; }
.gnode.selected rect { stroke: var(--acc); stroke-width: 2; }
.gnode text { fill: var(--tx); font: 11px var(--sans); }
.gnode .gsub { fill: var(--tx-3); font: 9.5px var(--mono); }
.gedge { stroke: var(--line-2); stroke-width: 1.4; fill: none; marker-end: url(#arrow); }

/* ── inspector / drawer ─────────────────────────────── */
.drawer { position: fixed; top: 50px; right: 0; bottom: 0; width: min(560px, 92vw); background: var(--bg-1);
  border-left: 1px solid var(--line-2); box-shadow: -12px 0 40px rgba(0,0,0,.5); z-index: 50;
  display: flex; flex-direction: column; transform: translateX(100%); transition: transform .18s ease; }
.drawer.open { transform: translateX(0); }
.drawer .dhead2 { display: flex; align-items: center; gap: 10px; padding: 14px 18px; border-bottom: 1px solid var(--line); }
.drawer .dbody { flex: 1; overflow-y: auto; padding: 16px 18px 40px; }
.drawer .x { margin-left: auto; background: none; border: 0; color: var(--tx-3); font-size: 17px; }
.fgroup { margin-bottom: 14px; }
.fgroup .fk { color: var(--tx-3); font-size: 10.5px; text-transform: uppercase; letter-spacing: .7px; margin-bottom: 3px; }
.fgroup .fv { font-size: 12.5px; word-break: break-word; white-space: pre-wrap; }
.fgroup .fv.mono { font-size: 11.5px; }
textarea, input[type=text] { width: 100%; background: var(--bg-2); border: 1px solid var(--line-2); border-radius: 7px;
  color: var(--tx); padding: 8px 10px; font: 12.5px var(--sans); resize: vertical; }
textarea:focus, input[type=text]:focus { outline: 1px solid var(--acc); }

/* ── misc ───────────────────────────────────────────── */
.toast { position: fixed; bottom: 22px; left: 50%; transform: translateX(-50%); background: var(--bg-3);
  border: 1px solid var(--line-2); border-radius: 8px; padding: 9px 18px; font-size: 12.5px; z-index: 99;
  box-shadow: 0 8px 30px rgba(0,0,0,.5); animation: fade .25s; }
.toast.err { border-color: #5c2b2b; color: #f08a8a; }
@keyframes fade { from { opacity: 0; transform: translate(-50%, 6px); } }
.empty { color: var(--tx-3); text-align: center; padding: 46px 0; font-size: 12.5px; }
.spark { display: flex; align-items: flex-end; gap: 2px; height: 34px; }
.spark i { flex: 1; background: var(--acc-dim); border-radius: 1.5px 1.5px 0 0; min-height: 2px; }
.spark i.hot { background: var(--acc); }
.legend { display: flex; gap: 14px; color: var(--tx-3); font-size: 11px; margin-top: 6px; flex-wrap: wrap; }
.legend b { font-weight: 550; }
pre.raw { background: var(--bg-2); border-radius: 7px; padding: 10px 12px; font: 10.5px/1.5 var(--mono);
  overflow-x: auto; max-height: 320px; overflow-y: auto; white-space: pre-wrap; word-break: break-all; }
.sev-high { color: #f08a8a; } .sev-action { color: #f0c06a; } .sev-info { color: #7cb9f5; }
</style>
</head>
<body>
<div id="app">
  <header>
    <div class="logo"><span class="mark">◉</span> FutureOS <span style="color:var(--tx-3)">·</span> <span style="color:var(--acc)">Loop</span><span class="sub">control plane</span></div>
    <nav class="tabs">
      <button class="tab active" data-view="overview" onclick="showView('overview')">Overview</button>
      <button class="tab" data-view="detail" id="tab-detail" style="display:none" onclick="showView('detail')">Goal</button>
    </nav>
    <div class="hdr-right">
      <span id="root-path" class="mono" title="loop state root"></span>
      <span class="live" id="live"><span class="dot"></span><span id="live-txt">connecting</span></span>
      <span id="clock" class="mono"></span>
    </div>
  </header>
  <main>
    <div class="view" id="view-overview"></div>
    <div class="view hidden" id="view-detail"></div>
  </main>
</div>
<div class="drawer" id="drawer"><div class="dhead2" id="drawer-head"></div><div class="dbody" id="drawer-body"></div></div>
<div id="toast-root"></div>

<script>
"use strict";
/* ── helpers ─────────────────────────────────────────── */
const $ = s => document.querySelector(s);
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
const now = () => Math.floor(Date.now()/1000);
function ago(ts){ if(!ts) return "—"; const d = now()-ts; if(d<0) return "in "+fmtDur(-d); return fmtDur(d)+" ago"; }
function fmtDur(s){ s=Math.floor(s); if(s<60) return s+"s"; if(s<3600) return Math.floor(s/60)+"m "+(s%60)+"s";
  if(s<86400) return Math.floor(s/3600)+"h "+Math.floor((s%3600)/60)+"m"; return Math.floor(s/86400)+"d "+Math.floor((s%86400)/3600)+"h"; }
function tsLocal(ts){ if(!ts) return "—"; return new Date(ts*1000).toLocaleString(); }
function money(c){ if(!c || Math.abs(c) < 1e-9) return "$0.00"; return c<0.01 ? "$"+c.toFixed(4) : "$"+c.toFixed(2); }
function tok(n){ if(!n) return "0"; if(n>=1e6) return (n/1e6).toFixed(2)+"M"; if(n>=1e3) return (n/1e3).toFixed(1)+"k"; return String(n); }
function shorten(p){ if(!p) return "—"; return p.replace(/^\/Users\/[^/]+/, "~"); }
function badge(txt, cls){ return `<span class="badge ${cls}">${esc(txt)}</span>`; }
function decBadge(d){ const m = {run:"b-ok", wait:"b-mut", replan:"b-warn", ask:"b-vio", terminal_no_followup:"b-info", skip:"b-mut", error:"b-bad"};
  return badge(d||"?", m[d]||"b-mut"); }
function statusBadge(s){ const m = {open:"b-info", done:"b-ok", superseded:"b-mut", deferred:"b-warn", blocked:"b-bad",
  active:"b-ok", cancelled:"b-mut", delivered:"b-warn", verified:"b-ok", failed:"b-bad", rework:"b-vio"}; return badge(s||"?", m[s]||"b-mut"); }
function classBadge(c){ const m = {advancement:"b-info", user_gate:"b-vio", user_action:"b-vio", monitor:"b-warn", blocker:"b-bad"};
  return badge((c||"").replace(/_/g," "), m[c]||"b-mut"); }
function sevBadge(s){ const m = {high:"b-bad", action:"b-warn", info:"b-info"}; return badge(s||"—", m[s]||"b-mut"); }
function toast(msg, isErr){ const t = document.createElement("div"); t.className = "toast"+(isErr?" err":""); t.textContent = msg;
  $("#toast-root").appendChild(t); setTimeout(()=>t.remove(), 3600); }
async function api(path, opts){ const r = await fetch(path, opts); const j = await r.json().catch(()=>({ok:false,error:"bad response"}));
  if(!j.ok) throw new Error(j.error||r.statusText); return j.data ?? j; }

/* ── state ───────────────────────────────────────────── */
let OVERVIEW = null, DETAIL = null, DETAIL_ID = null, EVENTS = null;
let graphSel = null;

/* ── live updates (SSE with polling fallback) ────────── */
function setLive(on, txt){ const l = $("#live"); l.className = "live "+(on?"on":"off"); $("#live-txt").textContent = txt||(on?"live":"reconnecting"); }
function connect(){
  let es;
  try { es = new EventSource("/api/stream"); } catch(e){ return pollLoop(); }
  es.addEventListener("overview", ev => { OVERVIEW = JSON.parse(ev.data); setLive(true); renderOverview(); syncDetailTab(); });
  es.addEventListener("goals", ev => { if(OVERVIEW) OVERVIEW.goals = JSON.parse(ev.data);
    if(DETAIL_ID) loadDetail(DETAIL_ID, true); });
  es.onerror = () => { setLive(false); es.close(); setTimeout(connect, 3000); };
}
let polling = false;
function pollLoop(){ if(polling) return; polling = true;
  const tick = async () => { try { OVERVIEW = await api("/api/overview"); setLive(true,"polling"); renderOverview(); }
    catch(e){ setLive(false); } setTimeout(tick, 4000); }; tick(); }

/* ── views ───────────────────────────────────────────── */
function showView(v){ for(const t of document.querySelectorAll(".tab")) t.classList.toggle("active", t.dataset.view===v);
  $("#view-overview").classList.toggle("hidden", v!=="overview");
  $("#view-detail").classList.toggle("hidden", v!=="detail");
  if(v==="overview"){ renderOverview(); } }
function syncDetailTab(){ const t = $("#tab-detail"); if(DETAIL_ID){ t.style.display=""; t.textContent = "Goal · "+DETAIL_ID.slice(0,14); } }

function openGoal(id){ location.hash = "#/goal/"+id; }
async function route(){ const m = location.hash.match(/^#\/goal\/(.+)$/);
  if(m){ DETAIL_ID = decodeURIComponent(m[1]); syncDetailTab(); showView("detail"); await loadDetail(DETAIL_ID); }
  else { DETAIL_ID = null; syncDetailTab(); $("#tab-detail").style.display="none"; showView("overview"); } }
window.addEventListener("hashchange", route);

/* ── overview render ─────────────────────────────────── */
function renderOverview(){
  const o = OVERVIEW; if(!o) return;
  $("#root-path").textContent = shorten(o.root);
  const t = o.totals;
  const goalRows = o.goals.map(g => {
    const pct = g.todos_total ? Math.round(100*g.todos_done/g.todos_total) : 0;
    return `<div class="gcard ${g.terminal?"terminal":""} ${g.cancelled?"cancelled":""}" onclick="openGoal('${encodeURIComponent(g.goal_id)}')">
      <div class="top">${statusBadge(g.cancelled?"cancelled":(g.terminal?"done":"active"))}
        ${g.open_gates? badge(g.open_gates+" gate"+(g.open_gates>1?"s":""),"b-vio"):""}
        <span class="gid">${esc(g.goal_id)}</span></div>
      <div class="obj">${esc(g.objective)}</div>
      <div class="meta"><span>todos <b>${g.todos_done}/${g.todos_total}</b></span><span>runs <b>${g.runs_total}</b></span>
        <span>cost <b>${money(g.cost_total)}</b></span><span>last run <b>${ago(g.last_run_at)}</b></span></div>
      <div class="dec">${decBadge(g.decision)}<span class="why" title="${esc(g.decision_reason)}">${esc(g.decision_reason)}</span></div>
      <div class="bar"><i style="width:${pct}%"></i></div>
    </div>`; }).join("");
  const attn = o.attention.items.length ? `<div class="sect"><h2>Attention queue <span class="count">${o.attention.item_count}</span></h2>
    <div class="attn card">${o.attention.items.map(i => `<div class="attn-row" onclick="openGoal('${encodeURIComponent(i.goal_id)}')">
      ${sevBadge(i.severity)}<span class="goal">${esc(i.goal_id)}</span>
      <span class="what">${esc(i.status.replace(/_/g," "))} · waits on <b>${esc(i.waiting_on)}</b></span>
      <span class="rec">${esc(i.recommended_action)}</span></div>`).join("")}</div></div>` : "";
  $("#view-overview").innerHTML = `
    <div class="stats">
      <div class="stat accent"><div class="v">${t.active}</div><div class="k">active goals</div></div>
      <div class="stat ok"><div class="v">${t.terminal}</div><div class="k">terminal</div></div>
      <div class="stat"><div class="v">${t.cancelled}</div><div class="k">cancelled</div></div>
      <div class="stat ${t.open_gates?"warn":""}"><div class="v">${t.open_gates}</div><div class="k">open gates</div></div>
      <div class="stat"><div class="v">${t.open_todos}</div><div class="k">open todos</div></div>
      <div class="stat"><div class="v">${t.runs_24h}</div><div class="k">runs · 24h</div></div>
      <div class="stat"><div class="v">${t.runs_7d}</div><div class="k">runs · 7d</div></div>
      <div class="stat"><div class="v">${money(t.cost_24h)}</div><div class="k">cost · 24h</div></div>
      <div class="stat"><div class="v">${money(t.cost_7d)}</div><div class="k">cost · 7d</div></div>
      <div class="stat"><div class="v">${t.slots_7d}</div><div class="k">quota slots · 7d</div></div>
    </div>
    ${attn}
    <div class="sect"><h2>Goals <span class="count">${t.goals}</span></h2>
      ${o.goals.length? `<div class="goals">${goalRows}</div>` : `<div class="empty card">No goals yet — <code>future loop goal init --objective "…"</code></div>`}
    </div>`;
}

/* ── goal detail ─────────────────────────────────────── */
async function loadDetail(id, soft){
  try { DETAIL = await api("/api/goals/"+encodeURIComponent(id)); } catch(e){ if(!soft) toast(e.message, true); return; }
  renderDetail();
  try { EVENTS = await api("/api/goals/"+encodeURIComponent(id)+"/events?limit=150"); } catch(e){ EVENTS = null; }
  renderEvents();
}
async function gateResolve(todoId, question){
  openDrawer(`Resolve gate`, `
    <div class="fgroup"><div class="fk">gate question</div><div class="fv">${esc(question||todoId)}</div></div>
    <div class="fgroup"><div class="fk">decision *</div><textarea id="gate-dec" rows="3" placeholder="approve / reject / a concrete decision…"></textarea></div>
    <div class="fgroup"><div class="fk">note (optional)</div><input type="text" id="gate-note" placeholder="context for the record"></div>
    <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:6px">
      <button class="btn" onclick="closeDrawer()">Cancel</button>
      <button class="btn primary" onclick="doGateResolve('${esc(todoId)}')">Resolve gate</button></div>`);
}
async function doGateResolve(todoId){
  const decision = $("#gate-dec").value.trim(); if(!decision) return toast("decision is required", true);
  const note = $("#gate-note").value.trim();
  try { const r = await api("/api/goals/"+encodeURIComponent(DETAIL_ID)+"/gate",
    {method:"POST", headers:{"content-type":"application/json"}, body: JSON.stringify({todo_id: todoId, decision, note: note||undefined})});
    toast(r.message||"gate resolved"); closeDrawer(); loadDetail(DETAIL_ID, true);
  } catch(e){ toast(e.message, true); }
}
async function goalCancel(){
  if(!confirm("Cancel this goal? Automation stops; state is retained.")) return;
  try { const r = await api("/api/goals/"+encodeURIComponent(DETAIL_ID)+"/lifecycle",
    {method:"POST", headers:{"content-type":"application/json"}, body: JSON.stringify({action:"cancel", reason:"cancelled from web ui"})});
    toast(r.message); loadDetail(DETAIL_ID, true);
  } catch(e){ toast(e.message, true); }
}
function openDrawer(title, html){ $("#drawer-head").innerHTML = `<b style="font-size:13px">${title}</b><button class="x" onclick="closeDrawer()">✕</button>`;
  $("#drawer-body").innerHTML = html; $("#drawer").classList.add("open"); }
function closeDrawer(){ $("#drawer").classList.remove("open"); }

function sparkline(runs){
  const days = []; for(let i=13;i>=0;i--){ const d0 = now()-i*86400; const dayStart = d0-(d0%86400); days.push({start:dayStart, n:0}); }
  for(const r of runs){ const idx = Math.floor((now()-r.recorded_at)/86400); if(idx>=0 && idx<14) days[13-idx].n++; }
  const mx = Math.max(1, ...days.map(d=>d.n));
  return `<div class="spark">${days.map(d=>`<i class="${d.n?"hot":""}" style="height:${Math.max(6,Math.round(100*d.n/mx))}%" title="${d.n} runs"></i>`).join("")}</div>
    <div class="legend"><span>runs/day · last 14d</span><span>peak <b>${mx}</b></span></div>`;
}

function renderDetail(){
  const g = DETAIL; if(!g) return;
  const d = g.decision;
  const openTodos = g.todos.filter(t=>t.status==="open");
  const gates = openTodos.filter(t=>t.class==="user_gate");
  const spend = g.spend;
  const unvalidated = new Set(g.unvalidated_deliveries||[]);
  const deliveriesByTodo = {}; for(const dv of g.deliveries) deliveriesByTodo[dv.todo_id] = dv;

  const todoRows = g.todos.map(t => {
    const dv = deliveriesByTodo[t.id];
    const lease = t.claimed_by ? `<div class="sub">lease ${esc(t.claimed_by)} · exp ${ago(t.lease_expires_at)}${t.holder_alive===false?" · <span class='sev-high'>holder dead</span>":""}</div>` : "";
    const val = t.validator ? `<div class="sub mono" title="verify command">✓ ${esc(t.validator)}${t.passed_validation?" · passed":(t.status==="done"?" · <span class='sev-high'>UNVALIDATED</span>":"")}</div>` : "";
    const gateQ = t.gate_question ? `<div class="sub">❓ ${esc(t.gate_question)}</div>` : "";
    const dec = t.decision ? `<div class="sub">→ ${esc(t.decision)}</div>` : "";
    const blockedBy = t.blocked ? `<div class="sub">blocked by ${t.blocked_by.map(esc).join(", ")}</div>` : "";
    return `<tr class="clickable" onclick='inspectTodo(${JSON.stringify(t.id)})'>
      <td class="mono" style="white-space:nowrap">${esc(t.id)}</td>
      <td><span class="prio prio-${esc(t.priority)}">${esc(t.priority)}</span></td>
      <td class="ttitle"><div class="t" title="${esc(t.text)}">${esc(t.title||t.text)}</div>${gateQ}${dec}${lease}${val}${blockedBy}</td>
      <td>${classBadge(t.class)}</td><td>${statusBadge(t.status)}</td>
      <td>${dv? statusBadge(dv.outcome):""}${unvalidated.has(t.id)? badge("unvalidated","b-bad"):""}</td>
      <td style="white-space:nowrap">${t.status==="open"&&t.class==="user_gate"? `<button class="btn primary" onclick="event.stopPropagation();gateResolve('${esc(t.id)}', ${JSON.stringify(t.gate_question||t.text)})">Resolve</button>`:""}
        ${t.failed_attempts? `<span class="chip" title="failed validation attempts">${t.failed_attempts}/${t.max_validation_attempts}</span>`:""}</td></tr>`;
  }).join("");

  const runRows = g.runs.slice(0,40).map(r => {
    const v = r.validation ? `<span class="chip" title="${esc(r.validation.summary||"")}">${esc(r.validation.status)}${r.validation.exit_code!=null?" · exit "+r.validation.exit_code:""}</span>` : "";
    const fk = r.failure_kind && r.failure_kind!=="none" ? badge(r.failure_kind.replace(/_/g," "),"b-bad") : "";
    return `<tr><td class="mono">#${r.turn}</td><td class="mono">${esc(r.todo_id)}</td>
      <td class="mono" title="${esc(r.run_id)}">${esc((r.run_id||"").slice(0,14))}</td>
      <td>${badge(r.terminal_state||"?", (r.terminal_state||"").includes("succe")||(r.terminal_state||"").includes("complet")?"b-ok":((r.terminal_state||"").includes("error")||(r.terminal_state||"").includes("fail")?"b-bad":"b-mut"))} ${v} ${fk}</td>
      <td class="mono">${tok(r.tokens_in_delta)}/${tok(r.tokens_out_delta)}</td>
      <td class="mono">${money(r.cost_delta)}</td>
      <td class="ttitle"><div class="t" title="${esc(r.evidence)}">${esc(r.evidence||"—")}</div></td>
      <td class="mono" title="${tsLocal(r.recorded_at)}">${ago(r.recorded_at)}</td></tr>`;
  }).join("");

  const agentRows = g.agents.map(a => `<tr><td class="mono">${esc(a.id)}</td>
    <td>${a.capabilities.length? a.capabilities.map(c=>`<span class="chip">${esc(c)}</span> `).join(""):"—"}</td>
    <td class="mono">${a.active_leases.length? a.active_leases.map(esc).join(", "):"—"}</td>
    <td class="mono">${a.last_heartbeat? ago(a.last_heartbeat):"—"}</td></tr>`).join("");

  const delivRows = g.deliveries.map(dv => `<tr><td class="mono">${esc(dv.todo_id)}</td><td>${statusBadge(dv.outcome)}</td>
    <td class="mono">turn ${dv.delivered_turn}</td><td>${esc(dv.note||"—")}</td>
    <td class="mono">${dv.followthrough_todo_id? "→ "+esc(dv.followthrough_todo_id):"—"}</td><td class="mono">${ago(dv.updated_at)}</td></tr>`).join("");

  const oblRows = g.replan_obligations.map(o => `<tr><td>${badge(o.kind.replace(/_/g," "), o.cleared?"b-mut":"b-warn")}</td>
    <td class="mono">${esc(o.todo_id||"—")}</td><td class="ttitle"><div class="t" title="${esc(o.evidence)}">${esc(o.evidence)}</div></td>
    <td>${o.cleared? badge("cleared","b-ok") : badge("open","b-warn")}</td><td class="mono">${ago(o.raised_at)}</td></tr>`).join("");

  const accRows = g.acceptance.map(a => `<tr><td class="mono">${esc(a.id)}</td><td>${esc(a.description)}</td>
    <td>${a.satisfied? badge("satisfied","b-ok"):badge("open","b-warn")}</td></tr>`).join("");

  const semRows = (g.semantic_history||[]).slice(-12).reverse().map(s => `<tr>
    <td class="mono">${ago(s.ts)}</td><td>${badge(s.kind||"evt","b-mut")}</td><td class="ttitle"><div class="t">${esc(s.summary||s.text||JSON.stringify(s))}</div></td></tr>`).join("");

  const termJ = g.frontier && g.frontier.terminal_judgement;
  const alerts = (g.liveness_alerts||[]).map(a => `<tr><td class="mono">${esc(a.agent_id)}</td>
    <td class="mono">${fmtDur(a.elapsed_secs)} / ${fmtDur(a.threshold_secs)}</td><td class="mono">#${a.consecutive}</td><td class="mono">${ago(a.ts)}</td></tr>`).join("");

  $("#view-detail").innerHTML = `
    <div class="backrow">
      <button class="btn" onclick="location.hash=''">← Overview</button>
      ${statusBadge(g.status)} ${g.terminal? badge("terminal closure","b-info"):""}
      <span class="chip">${esc(g.goal_id)}</span>
      <span style="flex:1"></span>
      ${g.status!=="cancelled" && !g.terminal ? `<button class="btn danger" onclick="goalCancel()">Cancel goal</button>`:""}
    </div>
    <div class="dhead"><div class="obj">${esc(g.objective)}<div class="path">${esc(shorten(g.cwd))} · created ${tsLocal(g.created_at)} · ${g.event_count} ledger events</div></div></div>

    <div class="grid3" style="margin-bottom:14px">
      <div class="panel"><h3>Kernel decision</h3>
        <div style="display:flex;gap:8px;align-items:center;margin-bottom:8px">${decBadge(d.decision)} ${badge(d.mode,"b-mut")} ${d.should_run? badge("should_run","b-ok"):badge("no run","b-mut")}</div>
        <div style="font-size:12.5px;margin-bottom:8px">${esc(d.reason)}</div>
        <dl class="kv">
          <dt>reason code</dt><dd>${esc(d.reason_code)}</dd>
          <dt>state</dt><dd>${esc(d.state)}</dd>
          <dt>waiting on</dt><dd>${esc(d.waiting_on)}</dd>
          <dt>recommended</dt><dd>${esc(d.recommended_action)}</dd>
          <dt>lifecycle</dt><dd>${esc(d.lifecycle_phase)}${d.lifecycle_flags&&d.lifecycle_flags.length?" · "+esc(d.lifecycle_flags.join(", ")):""}</dd>
          <dt>open todos</dt><dd>${d.open_count}</dd>
        </dl></div>
      <div class="panel"><h3>Next action & attention</h3>
        <div style="font-size:12.5px;margin-bottom:10px">${esc(g.next_action||"—")}</div>
        ${g.attention? `<div style="display:flex;gap:8px;align-items:center;margin-bottom:8px">${sevBadge(g.attention.severity)}${badge(g.attention.waiting_on,"b-mut")}</div>
          <div style="font-size:12px;color:var(--tx-2)">${esc(g.attention.recommended_action)}</div>` : `<div style="color:var(--tx-3);font-size:12px">No attention item — nothing waiting on the operator.</div>`}
        ${termJ? `<div style="margin-top:10px"><h3 style="margin-bottom:6px">Terminal judgement</h3><dl class="kv">
          <dt>terminal</dt><dd>${termJ.is_terminal??termJ.terminal??"—"}</dd></dl></div>`:""}
      </div>
      <div class="panel"><h3>Spend & throughput</h3>
        ${sparkline(g.runs)}
        <dl class="kv" style="margin-top:10px">
          <dt>24h</dt><dd>${spend.runs_24h.runs} runs · ${money(spend.runs_24h.cost)} · ${tok(spend.runs_24h.tokens_in)}↓ ${tok(spend.runs_24h.tokens_out)}↑</dd>
          <dt>7d</dt><dd>${spend.runs_7d.runs} runs · ${money(spend.runs_7d.cost)} · ${spend.runs_7d.slots} slots</dd>
          <dt>all time</dt><dd>${spend.total.runs} runs · ${money(spend.total.cost)}</dd>
          <dt>7d outcomes</dt><dd>${spend.outcomes_7d.succeeded} ok · ${spend.outcomes_7d.verify_failed} verify-fail · ${spend.outcomes_7d.infra_failed} infra · ${spend.outcomes_7d.errored} err</dd>
        </dl></div>
    </div>

    ${gates.length? `<div class="sect"><h2>Open gates <span class="count">${gates.length} — all work frozen</span></h2>
      <div class="twrap"><table><thead><tr><th>id</th><th>question</th><th></th></tr></thead><tbody>
      ${gates.map(t=>`<tr><td class="mono">${esc(t.id)}</td><td>${esc(t.gate_question||t.text)}</td>
        <td><button class="btn primary" onclick='gateResolve(${JSON.stringify(t.id)}, ${JSON.stringify(t.gate_question||t.text)})'>Resolve</button></td></tr>`).join("")}
      </tbody></table></div></div>`:""}

    <div class="sect"><h2>Dependency graph <span class="count">${g.todos.length} nodes</span></h2>
      <div class="panel" style="padding:6px"><div id="graph"></div></div></div>

    <div class="sect"><h2>Todos <span class="count">${g.todos.length} · ${openTodos.length} open</span></h2>
      <div class="twrap"><table><thead><tr><th>id</th><th>pri</th><th>todo</th><th>class</th><th>status</th><th>delivery</th><th></th></tr></thead>
      <tbody>${todoRows}</tbody></table></div></div>

    <div class="grid2">
      <div class="sect"><h2>Agents <span class="count">${g.agents.length}</span></h2>
        ${g.agents.length? `<div class="twrap"><table><thead><tr><th>agent</th><th>capabilities</th><th>active leases</th><th>heartbeat</th></tr></thead><tbody>${agentRows}</tbody></table></div>`:`<div class="empty card">No agents registered</div>`}
        ${alerts? `<h2 style="margin-top:14px">Liveness alerts</h2><div class="twrap"><table><thead><tr><th>agent</th><th>silent / threshold</th><th>seq</th><th>when</th></tr></thead><tbody>${alerts}</tbody></table></div>`:""}
      </div>
      <div class="sect"><h2>Delivery closure <span class="count">${g.deliveries.length}</span></h2>
        ${g.deliveries.length? `<div class="twrap"><table><thead><tr><th>todo</th><th>outcome</th><th>delivered</th><th>note</th><th>follow-through</th><th>updated</th></tr></thead><tbody>${delivRows}</tbody></table></div>`:`<div class="empty card">No deliveries recorded</div>`}
      </div>
    </div>

    <div class="grid2">
      <div class="sect"><h2>Replan obligations <span class="count">${g.replan_obligations.filter(o=>!o.cleared).length} open</span></h2>
        ${oblRows? `<div class="twrap"><table><thead><tr><th>kind</th><th>todo</th><th>evidence</th><th>state</th><th>raised</th></tr></thead><tbody>${oblRows}</tbody></table></div>`:`<div class="empty card">None</div>`}
        ${accRows? `<h2 style="margin-top:14px">Acceptance gaps</h2><div class="twrap"><table><thead><tr><th>id</th><th>condition</th><th>state</th></tr></thead><tbody>${accRows}</tbody></table></div>`:""}
      </div>
      <div class="sect"><h2>Semantic history <span class="count">latest</span></h2>
        ${semRows? `<div class="twrap"><table><tbody>${semRows}</tbody></table></div>`:`<div class="empty card">No semantic events</div>`}
      </div>
    </div>

    <div class="sect"><h2>Run ledger <span class="count">latest ${Math.min(40,g.runs.length)} of ${g.runs.length}</span></h2>
      ${g.runs.length? `<div class="twrap"><table><thead><tr><th>turn</th><th>todo</th><th>run</th><th>outcome</th><th>tok in/out</th><th>cost</th><th>evidence</th><th>when</th></tr></thead><tbody>${runRows}</tbody></table></div>`:`<div class="empty card">No runs recorded yet</div>`}
    </div>

    <div class="sect"><h2>Event ledger <span class="count">newest first · ${g.event_count} total</span></h2>
      <div id="events"><div class="empty card">loading…</div></div></div>`;

  renderGraph(g);
}

function renderEvents(){
  const el = $("#events"); if(!el) return;
  if(!EVENTS || !EVENTS.length){ el.innerHTML = `<div class="empty card">No events</div>`; return; }
  el.innerHTML = `<div class="twrap"><table><thead><tr><th>when</th><th>kind</th><th>event id</th><th>payload</th></tr></thead><tbody>
    ${EVENTS.map(e => `<tr><td class="mono" title="${tsLocal(e.ts)}">${ago(e.ts)}</td><td>${badge(e.kind,"b-mut")}</td>
      <td class="mono">${esc((e.event_id||"").slice(0,16))}</td>
      <td><details><summary class="mono" style="cursor:pointer;color:var(--tx-2)">${esc(summarizeEvent(e))}</summary>
        <pre class="raw">${esc(JSON.stringify(e.event,null,1))}</pre></details></td></tr>`).join("")}
  </tbody></table></div>`;
}
function summarizeEvent(e){
  const v = e.event||{}; const f = k => v[k] ? String(v[k]) : null;
  return [f("todo_id"), f("agent_id"), f("decision"), f("terminal_state"), f("outcome"), f("reason")]
    .filter(Boolean).join(" · ") || e.kind;
}

/* ── todo inspector ──────────────────────────────────── */
function inspectTodo(id){
  const t = (DETAIL.todos||[]).find(x=>x.id===id); if(!t) return;
  const row = (k,v) => v!=null && v!=="" && v!==false ? `<div class="fgroup"><div class="fk">${k}</div><div class="fv">${v}</div></div>` : "";
  const monos = (k,v) => v!=null && v!=="" ? `<div class="fgroup"><div class="fk">${k}</div><div class="fv mono">${esc(v)}</div></div>` : "";
  openDrawer(`Todo · ${esc(t.id)}`, `
    <div style="display:flex;gap:6px;flex-wrap:wrap;margin-bottom:12px">${statusBadge(t.status)}${classBadge(t.class)}
      <span class="prio prio-${esc(t.priority)}">${esc(t.priority)}</span>${badge(t.role,"b-mut")}
      ${t.blocked?badge("blocked","b-bad"):""}${t.archive_state==="archived"?badge("archived","b-mut"):""}</div>
    <div class="fgroup"><div class="fk">text</div><div class="fv">${esc(t.text)}</div></div>
    ${row("gate question", esc(t.gate_question))}${row("decision", esc(t.decision))}${row("note", esc(t.note))}
    ${row("blocked by", t.blocked_by.map(esc).join(", "))}
    ${row("successors", t.successor_ids.map(esc).join(", "))}${row("closure intent", t.no_follow_up?"no follow-up":"")}
    ${monos("verify command", t.validator)}${row("validation", t.validator? (t.passed_validation?"passed":"not passed") : null)}
    ${row("acceptance tokens", esc(t.acceptance))}
    ${row("failed attempts", t.failed_attempts? t.failed_attempts+" / "+t.max_validation_attempts : null)}
    ${monos("monitor target", t.monitor_target)}${row("monitor cadence", t.monitor_cadence)}
    ${row("monitor due", t.monitor_due_at? tsLocal(t.monitor_due_at)+" ("+ago(t.monitor_due_at)+")" : null)}
    ${row("no-change polls", t.consecutive_no_change || null)}
    ${row("resume when", esc(t.resume_when_text))}
    ${row("lease", t.claimed_by? t.claimed_by+" · expires "+tsLocal(t.lease_expires_at)+" ("+ago(t.lease_expires_at)+")"+(t.holder_alive===false?" · HOLDER DEAD":"") : null)}
    ${row("evidence", esc(t.evidence))}
    <div class="fgroup"><div class="fk">timestamps</div><div class="fv mono">updated ${tsLocal(t.updated_at)}${t.completed_at?" · completed "+tsLocal(t.completed_at):""}</div></div>
    ${t.status==="open"&&t.class==="user_gate"? `<button class="btn primary" onclick='gateResolve(${JSON.stringify(t.id)}, ${JSON.stringify(t.gate_question||t.text)})'>Resolve gate</button>`:""}
  `);
}

/* ── dependency graph (layered DAG, no libs) ─────────── */
function renderGraph(g){
  const el = $("#graph"); if(!el) return;
  const todos = g.todos.filter(t=>t.archive_state!=="archived");
  if(!todos.length){ el.innerHTML = `<div class="empty">No todos</div>`; return; }
  const edges = [];
  for(const t of todos){ for(const pred of (t.blocked_by||[])){ if(todos.find(x=>x.id===pred)) edges.push([pred, t.id]); }
    for(const s of (t.successor_ids||[])){ if(todos.find(x=>x.id===s)) edges.push([t.id, s]); } }
  // longest-path layering over the DAG (fallback: index order on cycles)
  const layer = {}; const byId = Object.fromEntries(todos.map(t=>[t.id,t]));
  const indeg = {}; todos.forEach(t=>indeg[t.id]=0); edges.forEach(([a,b])=>indeg[b]=(indeg[b]||0)+1);
  const memo = {};
  function depth(id, seen){ if(memo[id]!=null) return memo[id]; if(seen.has(id)) return 0;
    seen.add(id); const preds = edges.filter(e=>e[1]===id).map(e=>e[0]);
    const d = preds.length? Math.max(...preds.map(p=>depth(p,seen)))+1 : 0; seen.delete(id); memo[id]=d; return d; }
  todos.forEach(t=>{ layer[t.id] = edges.length? depth(t.id, new Set()) : 0; });
  const layers = []; todos.forEach(t=>{ (layers[layer[t.id]] = layers[layer[t.id]]||[]).push(t); });
  layers.forEach(l=>l.sort((a,b)=>a.index-b.index));
  const W = 218, H = 54, GX = 64, GY = 22;
  const MAX_COLS = 5; // serpentine wrap so a long chain never runs off-screen
  const pos = {}; layers.forEach((l,li)=>l.forEach((t,ri)=>{
    const row = Math.floor(li/MAX_COLS), colRaw = li%MAX_COLS;
    const col = row%2 ? MAX_COLS-1-colRaw : colRaw; // snake: even rows L→R, odd R→L
    pos[t.id] = {x: col*(W+GX)+10, y: (row*4+ri)*(H+GY)+10};
  }));
  const totalRows = Math.ceil(layers.length/MAX_COLS);
  const width = Math.min(layers.length,MAX_COLS)*(W+GX)-GX+20, height = totalRows*4*(H+GY)-GY+20;
  const stColor = s => ({open:"#4da3ff", done:"#3fb96c", superseded:"#5d6f80", deferred:"#e0a53c", blocked:"#e05c5c"}[s]||"#5d6f80");
  const clsGlyph = c => ({user_gate:"◆", user_action:"◇", monitor:"◔", blocker:"✕", advancement:"▸"}[c]||"▸");
  const nodes = todos.map(t => { const p = pos[t.id]; const label = (t.title||t.text||t.id);
    const short = label.length>34? label.slice(0,33)+"…" : label;
    return `<g class="gnode" data-id="${esc(t.id)}" transform="translate(${p.x},${p.y})" onclick="inspectTodo('${esc(t.id)}')">
      <rect width="${W}" height="${H}" style="stroke:${stColor(t.status)};stroke-width:${t.status==="open"?1.8:1}"></rect>
      <text x="10" y="21">${clsGlyph(t.class)} ${esc(short)}</text>
      <text class="gsub" x="10" y="39">${esc(t.id)} · ${esc(t.priority)} · ${esc(t.status)}${t.claimed_by?" · "+esc(t.claimed_by):""}</text></g>`; }).join("");
  const paths = edges.map(([a,b]) => { const p1 = pos[a], p2 = pos[b]; if(!p1||!p2) return "";
    const x1 = p1.x+W, y1 = p1.y+H/2, x2 = p2.x, y2 = p2.y+H/2; const mx = (x1+x2)/2;
    return `<path class="gedge" d="M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}"/>`; }).join("");
  el.innerHTML = `<div style="overflow-x:auto"><svg width="${Math.max(width,300)}" height="${Math.max(height,60)}">
    <defs><marker id="arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto">
    <path d="M0,0 L8,4 L0,8 z" fill="#2e3d4d"/></marker></defs>${paths}${nodes}</svg></div>`;
}

/* ── boot ────────────────────────────────────────────── */
setInterval(()=>{ $("#clock").textContent = new Date().toLocaleTimeString(); }, 1000);
connect();
route();
</script>
</body>
</html>
"##;
