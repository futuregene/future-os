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
.hdr-model { display: flex; align-items: baseline; gap: 6px; }
.hdr-model .mv { color: var(--acc); font-family: var(--mono); font-size: 11.5px; }
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
.sect > h2 .count { color: var(--tx-3); font-weight: 400; text-transform: none; letter-spacing: 0; }
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
.gcard .meta { display: flex; gap: 14px; color: var(--tx-3); font-size: 11px; margin-bottom: 10px; flex-wrap: wrap; }
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
.kv { display: grid; grid-template-columns: 104px 1fr; gap: 4px 14px; font-size: 12px; }
.kv dt { color: var(--tx-3); white-space: nowrap; }
.kv dd { color: var(--tx); font-family: var(--mono); font-size: 11.5px; word-break: break-word; min-width: 0; }

/* ── detail tabs ────────────────────────────────────── */
.dtabs { display: flex; gap: 2px; border-bottom: 1px solid var(--line); margin: 4px 0 16px; flex-wrap: wrap; }
.dtab { padding: 7px 14px; background: none; border: 0; border-bottom: 2px solid transparent; color: var(--tx-2); font-size: 12.5px; }
.dtab:hover { color: var(--tx); }
.dtab.active { color: var(--acc); border-bottom-color: var(--acc); font-weight: 600; }
.dtab .pill { margin-left: 6px; padding: 0 6px; border-radius: 10px; background: var(--bg-3); color: var(--tx-2); font: 600 10px/1.7 var(--mono); }
.dtab .pill.hot { background: var(--bad-bg); color: #f08a8a; }

/* ── tables ─────────────────────────────────────────── */
table { width: 100%; border-collapse: collapse; font-size: 12px; }
th { text-align: left; color: var(--tx-3); font-weight: 550; font-size: 10.5px; text-transform: uppercase;
  letter-spacing: .7px; padding: 8px 12px; border-bottom: 1px solid var(--line); white-space: nowrap; }
td { padding: 8px 12px; border-bottom: 1px solid var(--line); vertical-align: top; }
tr:last-child td { border-bottom: 0; }
tbody tr:hover { background: var(--bg-2); }
td .sub { color: var(--tx-3); font-size: 11px; margin-top: 2px; }
.twrap { background: var(--bg-1); border: 1px solid var(--line); border-radius: 10px; overflow-x: auto; }
.mono { font-family: var(--mono); font-size: 11.5px; }
.ttitle { max-width: 420px; }
.ttitle .t { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.clickable { cursor: pointer; }
th.tip { text-decoration: underline dotted var(--tx-3); text-underline-offset: 3px; cursor: help; }

/* ── graph ──────────────────────────────────────────── */
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

/* ── tooltip ────────────────────────────────────────── */
#tip { position: fixed; z-index: 200; max-width: 320px; background: #0d141b; border: 1px solid var(--line-2);
  border-radius: 8px; padding: 8px 11px; font-size: 11.5px; line-height: 1.45; color: var(--tx);
  box-shadow: 0 10px 34px rgba(0,0,0,.6); pointer-events: none; opacity: 0; transition: opacity .12s; }
#tip.show { opacity: 1; }
#tip .tt { font-weight: 650; color: var(--acc); margin-bottom: 2px; }

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
.num { font-family: var(--mono); font-size: 11.5px; text-align: right; white-space: nowrap; }
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
      <span class="hdr-model" data-tip="Model used for NEW `future loop run` turns (from agent settings.json default_model or --model flag). Past runs record only token/cost deltas, not the model.">model <span class="mv" id="hdr-model">—</span></span>
      <span id="root-path" class="mono" data-tip="Loop state root: the project-local .future/loop directory this dashboard reads (override with FUTURE_LOOP_ROOT or --root)."></span>
      <span class="live" id="live" data-tip="Live connection to the dashboard server (SSE push; falls back to polling). Green = receiving updates."><span class="dot"></span><span id="live-txt">connecting</span></span>
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
<div id="tip"></div>

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
function badge(txt, cls, tip){ return `<span class="badge ${cls}"${tip?` data-tip="${esc(tip)}"`:""}>${esc(txt)}</span>`; }
function decBadge(d){ const m = {run:"b-ok", wait:"b-mut", replan:"b-warn", ask:"b-vio", terminal_no_followup:"b-info", skip:"b-mut", error:"b-bad"};
  const tip = {run:"kernel: a runnable todo is selected — a worker should run one bounded turn",
    wait:"kernel: nothing to do right now (e.g. waiting for a monitor cadence)",
    replan:"kernel: state drifted — a replan obligation must be acked before more spend",
    ask:"kernel: a user gate needs a human decision; work is frozen until resolved",
    terminal_no_followup:"kernel: validated terminal closure — goal complete",
    skip:"kernel: deliberately not running (e.g. goal cancelled)"}[d];
  return badge(d||"?", m[d]||"b-mut", tip); }
function statusBadge(s){ const m = {open:"b-info", done:"b-ok", superseded:"b-mut", deferred:"b-warn", blocked:"b-bad",
  active:"b-ok", cancelled:"b-mut", delivered:"b-warn", verified:"b-ok", failed:"b-bad", rework:"b-vio"};
  const tip = {open:"not started / runnable", done:"completed with evidence", superseded:"replaced by a better route — no longer blocks closure",
    deferred:"held until its resume condition", blocked:"held by a gate/blocker/predecessor",
    delivered:"delivered, awaiting operator verification", verified:"operator-confirmed delivery",
    failed:"delivery rejected", rework:"sent back for rework"}[s];
  return badge(s||"?", m[s]||"b-mut", tip); }
function classBadge(c){ const m = {advancement:"b-info", user_gate:"b-vio", user_action:"b-vio", monitor:"b-warn", blocker:"b-bad"};
  const tip = {advancement:"runnable agent work", user_gate:"a human decision that FREEZES dependent work until resolved",
    user_action:"a human to-do that does NOT freeze the agent", monitor:"periodic read-only observation of an external target",
    blocker:"an external blocker gating dependent todos"}[c];
  return badge((c||"").replace(/_/g," "), m[c]||"b-mut", tip); }
function sevBadge(s){ const m = {high:"b-bad", action:"b-warn", info:"b-info"};
  const tip = {high:"needs a human now", action:"needs the agent/controller", info:"informational"}[s];
  return badge(s||"—", m[s]||"b-mut", tip); }
function toast(msg, isErr){ const t = document.createElement("div"); t.className = "toast"+(isErr?" err":""); t.textContent = msg;
  $("#toast-root").appendChild(t); setTimeout(()=>t.remove(), 3600); }
async function api(path, opts){ const r = await fetch(path, opts); const j = await r.json().catch(()=>({ok:false,error:"bad response"}));
  if(!j.ok) throw new Error(j.error||r.statusText); return j.data ?? j; }

/* ── tooltip (delegated; works for dynamically-added nodes) ── */
(function(){
  const tip = $("#tip");
  let cur = null;
  document.addEventListener("mouseover", e => {
    const el = e.target && e.target.closest ? e.target.closest("[data-tip]") : null;
    if(el === cur) return; cur = el;
    if(!el){ tip.classList.remove("show"); return; }
    tip.textContent = el.dataset.tip;
    tip.classList.add("show");
    const r = el.getBoundingClientRect();
    const tw = tip.offsetWidth, thh = tip.offsetHeight;
    let x = Math.min(r.left, window.innerWidth - tw - 12);
    let y = r.bottom + 8; if(y + thh > window.innerHeight - 8) y = r.top - thh - 8;
    tip.style.left = Math.max(8, x) + "px"; tip.style.top = Math.max(8, y) + "px";
  });
  document.addEventListener("mouseout", e => { const rt = e.relatedTarget; if(!rt || !rt.closest || !rt.closest("[data-tip]")) { cur = null; tip.classList.remove("show"); } });
})();

/* ── state ───────────────────────────────────────────── */
let OVERVIEW = null, DETAIL = null, DETAIL_ID = null, EVENTS = null;
let dTab = "board";

/* ── live updates (SSE with polling fallback) ────────── */
function setLive(on, txt){ const l = $("#live"); l.className = "live "+(on?"on":"off"); $("#live-txt").textContent = txt||(on?"live":"reconnecting"); }
function connect(){
  let es;
  try { es = new EventSource("/api/stream"); } catch(e){ return pollLoop(); }
  es.addEventListener("overview", ev => { OVERVIEW = JSON.parse(ev.data); setLive(true);
    if(OVERVIEW.model && OVERVIEW.model.default_model) $("#hdr-model").textContent = OVERVIEW.model.default_model;
    renderOverview(); syncDetailTab(); });
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
const TIP = {
  activeGoals: "Goals whose automation is still running (not terminal, not cancelled)",
  terminal: "Goals that reached validated closure: all todos done/superseded + closure intent + no acceptance gaps",
  cancelled: "Goals stopped by the operator (state retained for audit, automation off)",
  openGates: "Unresolved user gates — every open gate FREEZES its goal's work until a human decides",
  openTodos: "Todos not yet done across all goals",
  runs24: "Bounded turns executed in the last 24 hours",
  runs7: "Bounded turns executed in the last 7 days",
  cost24: "LLM cost burned in the last 24 hours (sum over run ledger)",
  cost7: "LLM cost over the last 7 days",
  slots7: "Quota slots spent in 7 days — the kernel's spend unit (run / agent / heartbeat classified)",
  attnQueue: "One item per goal that needs something: a human decision (gate), the agent (advancement/replan), or a monitor poll",
  severity: "high = blocked on a human · action = agent work · info = monitor signal",
  waitingOn: "Who the goal is waiting on: user_or_controller / codex (agent) / monitor_signal / external_evidence",
  recAction: "The single most useful next step the control plane recommends",
};
function renderOverview(){
  const o = OVERVIEW; if(!o) return;
  $("#root-path").textContent = shorten(o.root);
  if(o.model && o.model.default_model) $("#hdr-model").textContent = o.model.default_model;
  const t = o.totals;
  const goalRows = o.goals.map(g => {
    const pct = g.todos_total ? Math.round(100*g.todos_done/g.todos_total) : 0;
    return `<div class="gcard ${g.terminal?"terminal":""} ${g.cancelled?"cancelled":""}" onclick="openGoal('${encodeURIComponent(g.goal_id)}')">
      <div class="top">${statusBadge(g.cancelled?"cancelled":(g.terminal?"done":"active"))}
        ${g.open_gates? badge(g.open_gates+" gate"+(g.open_gates>1?"s":""),"b-vio", TIP.openGates):""}
        <span class="gid">${esc(g.goal_id)}</span></div>
      <div class="obj">${esc(g.objective)}</div>
      <div class="meta"><span data-tip="done / total todos">todos <b>${g.todos_done}/${g.todos_total}</b></span><span data-tip="bounded turns recorded">runs <b>${g.runs_total}</b></span>
        <span data-tip="total LLM cost">cost <b>${money(g.cost_total)}</b></span><span data-tip="time of the most recent run">last run <b>${ago(g.last_run_at)}</b></span></div>
      <div class="dec">${decBadge(g.decision)}<span class="why" data-tip="${esc(g.decision_reason)}">${esc(g.decision_reason)}</span></div>
      <div class="bar" data-tip="${pct}% of todos done"><i style="width:${pct}%"></i></div>
    </div>`; }).join("");
  const attn = o.attention.items.length ? `<div class="sect"><h2 data-tip="${esc(TIP.attnQueue)}">Attention queue <span class="count">${o.attention.item_count}</span></h2>
    <div class="attn card">${o.attention.items.map(i => `<div class="attn-row" onclick="openGoal('${encodeURIComponent(i.goal_id)}')">
      <span data-tip="${esc(TIP.severity)}">${sevBadge(i.severity)}</span><span class="goal">${esc(i.goal_id)}</span>
      <span class="what" data-tip="${esc(TIP.waitingOn)}">${esc(i.status.replace(/_/g," "))} · waits on <b>${esc(i.waiting_on)}</b></span>
      <span class="rec" data-tip="${esc(TIP.recAction)}">${esc(i.recommended_action)}</span></div>`).join("")}</div></div>` : "";
  $("#view-overview").innerHTML = `
    <div class="stats">
      <div class="stat accent" data-tip="${esc(TIP.activeGoals)}"><div class="v">${t.active}</div><div class="k">active goals</div></div>
      <div class="stat ok" data-tip="${esc(TIP.terminal)}"><div class="v">${t.terminal}</div><div class="k">terminal</div></div>
      <div class="stat" data-tip="${esc(TIP.cancelled)}"><div class="v">${t.cancelled}</div><div class="k">cancelled</div></div>
      <div class="stat ${t.open_gates?"warn":""}" data-tip="${esc(TIP.openGates)}"><div class="v">${t.open_gates}</div><div class="k">open gates</div></div>
      <div class="stat" data-tip="${esc(TIP.openTodos)}"><div class="v">${t.open_todos}</div><div class="k">open todos</div></div>
      <div class="stat" data-tip="${esc(TIP.runs24)}"><div class="v">${t.runs_24h}</div><div class="k">runs · 24h</div></div>
      <div class="stat" data-tip="${esc(TIP.runs7)}"><div class="v">${t.runs_7d}</div><div class="k">runs · 7d</div></div>
      <div class="stat" data-tip="${esc(TIP.cost24)}"><div class="v">${money(t.cost_24h)}</div><div class="k">cost · 24h</div></div>
      <div class="stat" data-tip="${esc(TIP.cost7)}"><div class="v">${money(t.cost_7d)}</div><div class="k">cost · 7d</div></div>
      <div class="stat" data-tip="${esc(TIP.slots7)}"><div class="v">${t.slots_7d}</div><div class="k">quota slots · 7d</div></div>
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
function setDTab(t){ dTab = t; renderDetail(); }

function sparkline(runs){
  const days = []; for(let i=13;i>=0;i--){ const d0 = now()-i*86400; const dayStart = d0-(d0%86400); days.push({start:dayStart, n:0}); }
  for(const r of runs){ const idx = Math.floor((now()-r.recorded_at)/86400); if(idx>=0 && idx<14) days[13-idx].n++; }
  const mx = Math.max(1, ...days.map(d=>d.n));
  return `<div class="spark" data-tip="bounded turns per day over the last 14 days">${days.map(d=>`<i class="${d.n?"hot":""}" style="height:${Math.max(6,Math.round(100*d.n/mx))}%" data-tip="${d.n} runs that day"></i>`).join("")}</div>
    <div class="legend"><span>runs/day · last 14d</span><span>peak <b>${mx}</b></span></div>`;
}

/* Column header with tooltip */
const TH = (label, tip) => `<th class="tip" data-tip="${esc(tip)}">${label}</th>`;

function renderDetail(){
  const g = DETAIL; if(!g) return;
  const d = g.decision;
  const openTodos = g.todos.filter(t=>t.status==="open");
  const gates = openTodos.filter(t=>t.class==="user_gate");
  const unvalidated = new Set(g.unvalidated_deliveries||[]);
  const deliveriesByTodo = {}; for(const dv of g.deliveries) deliveriesByTodo[dv.todo_id] = dv;
  const openObl = g.replan_obligations.filter(o=>!o.cleared).length;

  // tab bar with live counters
  const tabs = [
    ["board","Board", null],
    ["todos","Todos", g.todos.length, gates.length],
    ["agents","Workers", g.agents.length, null],
    ["runs","Runs", g.runs.length, null],
    ["events","Events", g.event_count, null],
  ].map(([id,label,n,hot]) => `<button class="dtab ${dTab===id?"active":""}" onclick="setDTab('${id}')">${label}${n!=null?`<span class="pill ${hot?"hot":""}">${hot||n}</span>`:""}</button>`).join("");

  $("#view-detail").innerHTML = `
    <div class="backrow">
      <button class="btn" onclick="location.hash=''">← Overview</button>
      ${statusBadge(g.status)} ${g.terminal? badge("terminal closure","b-info","validated closure: every todo done/superseded, closure intent declared, no acceptance gaps"):""}
      <span class="chip" data-tip="goal id">${esc(g.goal_id)}</span>
      <span style="flex:1"></span>
      ${g.status!=="cancelled" && !g.terminal ? `<button class="btn danger" data-tip="stop automation for this goal (state retained for audit)" onclick="goalCancel()">Cancel goal</button>`:""}
    </div>
    <div class="dhead"><div class="obj">${esc(g.objective)}<div class="path" data-tip="goal working directory · created at · ledger length">${esc(shorten(g.cwd))} · created ${tsLocal(g.created_at)} · ${g.event_count} ledger events</div></div></div>
    <div class="dtabs">${tabs}</div>
    <div id="dbody"></div>`;
  renderDTab(g, d, gates, unvalidated, deliveriesByTodo, openObl);
  lucideScrollFix();
}
function lucideScrollFix(){ /* keep the active tab in view on data refresh */ }

function renderDTab(g, d, gates, unvalidated, deliveriesByTodo, openObl){
  const el = $("#dbody"); if(!el) return;

  if(dTab === "board"){
    const spend = g.spend;
    el.innerHTML = `
    <div class="grid3" style="margin-bottom:14px">
      <div class="panel"><h3 data-tip="The deterministic should-run kernel's current verdict for this goal">Kernel decision</h3>
        <div style="display:flex;gap:8px;align-items:center;margin-bottom:8px">${decBadge(d.decision)} ${badge(d.mode,"b-mut","turn mode: normal / monitor_poll / terminal")} ${d.should_run? badge("should_run","b-ok","a worker should run one bounded turn now"):badge("no run","b-mut","nothing for a worker to do right now")}</div>
        <div style="font-size:12.5px;margin-bottom:8px">${esc(d.reason)}</div>
        <dl class="kv">
          <dt data-tip="stable machine-readable reason code">reason code</dt><dd>${esc(d.reason_code)}</dd>
          <dt data-tip="kernel state bucket">state</dt><dd>${esc(d.state)}</dd>
          <dt data-tip="who the goal waits on">waiting on</dt><dd>${esc(d.waiting_on)}</dd>
          <dt data-tip="the single most useful next step">recommended</dt><dd>${esc(d.recommended_action)}</dd>
          <dt data-tip="lifecycle phase + flags">lifecycle</dt><dd>${esc(d.lifecycle_phase)}${d.lifecycle_flags&&d.lifecycle_flags.length?" · "+esc(d.lifecycle_flags.join(", ")):""}</dd>
          <dt data-tip="todos still open">open todos</dt><dd>${d.open_count}</dd>
        </dl></div>
      <div class="panel"><h3>Next action & attention</h3>
        <div style="font-size:12.5px;margin-bottom:10px">${esc(g.next_action||"—")}</div>
        ${g.attention? `<div style="display:flex;gap:8px;align-items:center;margin-bottom:8px">${sevBadge(g.attention.severity)}${badge(g.attention.waiting_on,"b-mut",TIP.waitingOn)}</div>
          <div style="font-size:12px;color:var(--tx-2)">${esc(g.attention.recommended_action)}</div>` : `<div style="color:var(--tx-3);font-size:12px">No attention item — nothing waiting on the operator.</div>`}
      </div>
      <div class="panel"><h3 data-tip="LLM cost / token / quota-slot spend over 24h, 7d and all time">Spend & throughput</h3>
        ${sparkline(g.runs)}
        <dl class="kv" style="margin-top:10px">
          <dt data-tip="last 24 hours">24h</dt><dd>${spend.runs_24h.runs} runs · ${money(spend.runs_24h.cost)} · ${tok(spend.runs_24h.tokens_in)}↓ ${tok(spend.runs_24h.tokens_out)}↑</dd>
          <dt data-tip="last 7 days">7d</dt><dd>${spend.runs_7d.runs} runs · ${money(spend.runs_7d.cost)} · ${spend.runs_7d.slots} slots</dd>
          <dt data-tip="all time">all time</dt><dd>${spend.total.runs} runs · ${money(spend.total.cost)}</dd>
          <dt data-tip="turn outcomes over 7d: ok / verify-gate failure / recoverable infra (e.g. 429) / hard error">7d outcomes</dt><dd>${spend.outcomes_7d.succeeded} ok · ${spend.outcomes_7d.verify_failed} verify-fail · ${spend.outcomes_7d.infra_failed} infra · ${spend.outcomes_7d.errored} err</dd>
        </dl></div>
    </div>
    ${gates.length? `<div class="sect"><h2 data-tip="${esc(TIP.openGates)}">Open gates <span class="count">${gates.length} — all work frozen</span></h2>
      <div class="twrap"><table><thead><tr>${TH("id","gate todo id")}${TH("question","the concrete decision a human must make")}<th></th></tr></thead><tbody>
      ${gates.map(t=>`<tr><td class="mono">${esc(t.id)}</td><td>${esc(t.gate_question||t.text)}</td>
        <td><button class="btn primary" onclick='gateResolve(${JSON.stringify(t.id)}, ${JSON.stringify(t.gate_question||t.text)})'>Resolve</button></td></tr>`).join("")}
      </tbody></table></div></div>`:""}
    <div class="sect"><h2 data-tip="Todo dependency DAG — an arrow A→B means A blocks B (B cannot run until A is done/superseded). Click a node for full detail.">Dependency graph <span class="count">${g.todos.length} nodes</span></h2>
      <div class="panel" style="padding:6px"><div id="graph"></div></div></div>`;
    renderGraph(g);
    return;
  }

  if(dTab === "todos"){
    const rows = g.todos.map(t => {
      const dv = deliveriesByTodo[t.id];
      const lease = t.claimed_by ? `<div class="sub" data-tip="current lease holder + expiry${t.holder_alive===false?" · the holding process is DEAD (auto-reclaimed on next claim)":""}">lease ${esc(t.claimed_by)} · exp ${ago(t.lease_expires_at)}${t.holder_alive===false?" · <span class='sev-high'>holder dead</span>":""}</div>` : "";
      const val = t.validator ? `<div class="sub mono" data-tip="independent verify command run after each turn; exit 0 = validated${t.passed_validation?" · passed":""}">✓ ${esc(t.validator)}${t.passed_validation?" · passed":(t.status==="done"?" · <span class='sev-high'>UNVALIDATED</span>":"")}</div>` : "";
      const gateQ = t.gate_question ? `<div class="sub">❓ ${esc(t.gate_question)}</div>` : "";
      const dec = t.decision ? `<div class="sub">→ ${esc(t.decision)}</div>` : "";
      const blockedBy = t.blocked ? `<div class="sub" data-tip="predecessor todos that must finish first">blocked by ${t.blocked_by.map(esc).join(", ")}</div>` : "";
      const span = t.first_run_at ? `<span data-tip="first run: ${tsLocal(t.first_run_at)} · latest run: ${tsLocal(t.last_run_at)}">${ago(t.first_run_at)} → ${ago(t.last_run_at)}</span>` : "—";
      return `<tr class="clickable" onclick='inspectTodo(${JSON.stringify(t.id)})'>
        <td class="mono" style="white-space:nowrap">${esc(t.id)}</td>
        <td><span class="prio prio-${esc(t.priority)}" data-tip="priority: P0 highest — the kernel sorts the frontier by priority first">${esc(t.priority)}</span></td>
        <td class="ttitle"><div class="t" data-tip="${esc(t.text)}">${esc(t.title||t.text)}</div>${gateQ}${dec}${lease}${val}${blockedBy}</td>
        <td>${classBadge(t.class)}</td><td>${statusBadge(t.status)}</td>
        <td class="num" data-tip="turns on this todo">${t.runs||"—"}</td>
        <td class="num" data-tip="input tokens (LLM in)">${t.runs? tok(t.tokens_in):"—"}</td>
        <td class="num" data-tip="output tokens (LLM out)">${t.runs? tok(t.tokens_out):"—"}</td>
        <td class="num" data-tip="LLM cost spent on this todo">${t.runs? money(t.cost):"—"}</td>
        <td class="mono" style="white-space:nowrap">${span}</td>
        <td>${dv? statusBadge(dv.outcome):""}${unvalidated.has(t.id)? badge("unvalidated","b-bad","completed past its --verify gate (never exited 0) — does NOT count toward terminal closure"):""}</td>
        <td style="white-space:nowrap">${t.status==="open"&&t.class==="user_gate"? `<button class="btn primary" onclick="event.stopPropagation();gateResolve('${esc(t.id)}', ${JSON.stringify(t.gate_question||t.text)})">Resolve</button>`:""}
          ${t.failed_attempts? `<span class="chip" data-tip="failed validation attempts / budget before replan">${t.failed_attempts}/${t.max_validation_attempts}</span>`:""}</td></tr>`;
    }).join("");
    el.innerHTML = `<div class="sect"><h2>Todos <span class="count">${g.todos.length} · ${g.todos.filter(t=>t.status==="open").length} open</span></h2>
      <div class="twrap"><table><thead><tr>
        ${TH("id","todo id (goal-scoped)")}${TH("pri","priority P0/P1/P2 — the decision kernel sorts by it first")}${TH("todo","title / gate question / lease / verify / blockers")}${TH("class","advancement · user_gate · user_action · monitor · blocker")}${TH("status","open · done · superseded · deferred · blocked")}
        ${TH("runs","bounded turns spent on this todo")}${TH("tok in","input tokens (LLM)")}${TH("tok out","output tokens (LLM)")}${TH("cost","LLM cost on this todo")}${TH("first → latest run","when work on this todo started / last happened")}${TH("delivery","post-delivery outcome: delivered → verified/failed/rework")}<th></th></tr></thead>
      <tbody>${rows}</tbody></table></div></div>`;
    return;
  }

  if(dTab === "agents"){
    const agentRows = g.agents.map(a => `<tr><td class="mono">${esc(a.id)}</td>
      <td><span class="chip" data-tip="model for NEW runs by this worker (loop-wide default: agent settings.json default_model or --model flag; not recorded per past run)">${esc(a.model||"—")}</span></td>
      <td>${a.capabilities.length? a.capabilities.map(c=>`<span class="chip" data-tip="declared capability">${esc(c)}</span> `).join(""):"—"}</td>
      <td class="mono">${a.active_leases.length? a.active_leases.map(x=>`<span class="chip" data-tip="todo currently leased to this worker">${esc(x)}</span>`).join(" "):"—"}</td>
      <td class="num" data-tip="turns executed">${a.runs||"—"}</td>
      <td class="num" data-tip="input tokens">${a.runs? tok(a.tokens_in):"—"}</td>
      <td class="num" data-tip="output tokens">${a.runs? tok(a.tokens_out):"—"}</td>
      <td class="num" data-tip="LLM cost">${a.runs? money(a.cost):"—"}</td>
      <td class="mono" style="white-space:nowrap">${a.first_run_at? `<span data-tip="first run ${tsLocal(a.first_run_at)} · latest ${tsLocal(a.last_run_at)}">${ago(a.first_run_at)} → ${ago(a.last_run_at)}</span>`:"—"}</td>
      <td class="mono" data-tip="last scheduler heartbeat">${a.last_heartbeat? ago(a.last_heartbeat):"—"}</td></tr>`).join("");
    const alerts = (g.liveness_alerts||[]).map(a => `<tr><td class="mono">${esc(a.agent_id)}</td>
      <td class="mono" data-tip="silent for / threshold">${fmtDur(a.elapsed_secs)} / ${fmtDur(a.threshold_secs)}</td><td class="mono" data-tip="consecutive breach ordinal">#${a.consecutive}</td><td class="mono">${ago(a.ts)}</td></tr>`).join("");
    const delivRows = g.deliveries.map(dv => `<tr><td class="mono">${esc(dv.todo_id)}</td><td>${statusBadge(dv.outcome)}</td>
      <td class="mono" data-tip="run-turn counter at delivery time">turn ${dv.delivered_turn}</td><td>${esc(dv.note||"—")}</td>
      <td class="mono" data-tip="auto-created follow-up after an unverified delivery">${dv.followthrough_todo_id? "→ "+esc(dv.followthrough_todo_id):"—"}</td><td class="mono">${ago(dv.updated_at)}</td></tr>`).join("");
    const oblRows = g.replan_obligations.map(o => `<tr><td>${badge(o.kind.replace(/_/g," "), o.cleared?"b-mut":"b-warn","why a replan is required")}</td>
      <td class="mono">${esc(o.todo_id||"—")}</td><td class="ttitle"><div class="t" data-tip="${esc(o.evidence)}">${esc(o.evidence)}</div></td>
      <td>${o.cleared? badge("cleared","b-ok","acked with a frontier delta") : badge("open","b-warn","blocks further spend until acked")}</td><td class="mono">${ago(o.raised_at)}</td></tr>`).join("");
    const accRows = g.acceptance.map(a => `<tr><td class="mono">${esc(a.id)}</td><td>${esc(a.description)}</td>
      <td>${a.satisfied? badge("satisfied","b-ok"):badge("open","b-warn","terminal closure requires every gap satisfied")}</td></tr>`).join("");
    el.innerHTML = `
      <div class="sect"><h2 data-tip="Registered worker identities for this goal (one lane per --agent-id). Model/cost/tokens aggregated from the run ledger.">Workers <span class="count">${g.agents.length}</span></h2>
        ${g.agents.length? `<div class="twrap"><table><thead><tr>
          ${TH("worker","agent id (registered peer)")}${TH("model","model for new runs — loop-wide default, not recorded per past run")}${TH("capabilities","declared capabilities (metadata)")}${TH("active leases","todos this worker currently holds")}${TH("runs","turns executed")}${TH("tok in","input tokens")}${TH("tok out","output tokens")}${TH("cost","LLM cost")}${TH("first → latest run","activity window")}${TH("heartbeat","last scheduler heartbeat — silence past threshold raises a liveness alert")}
          </tr></thead><tbody>${agentRows}</tbody></table></div>`:`<div class="empty card">No agents registered</div>`}
      </div>
      ${alerts? `<div class="sect"><h2 data-tip="scheduler heartbeat went silent past the threshold — the host automation may be dead">Liveness alerts</h2><div class="twrap"><table><thead><tr>${TH("agent","worker id")}${TH("silent / threshold","elapsed silence vs alert threshold")}${TH("seq","consecutive breach count")}${TH("when","alert time")}</tr></thead><tbody>${alerts}</tbody></table></div></div>`:""}
      <div class="grid2">
        <div class="sect"><h2 data-tip="post-delivery outcome closure: delivered → verified / failed / rework">Delivery closure <span class="count">${g.deliveries.length}</span></h2>
          ${g.deliveries.length? `<div class="twrap"><table><thead><tr>${TH("todo","work item")}${TH("outcome","delivered → verified/failed/rework")}${TH("delivered","turn at delivery")}${TH("note","operator note")}${TH("follow-through","auto follow-up todo for unverified deliveries")}${TH("updated","last outcome change")}</tr></thead><tbody>${delivRows}</tbody></table></div>`:`<div class="empty card">No deliveries recorded</div>`}
        </div>
        <div class="sect"><h2 data-tip="state drift the kernel wants replanned before more spend (e.g. a completion without closure intent)">Replan obligations <span class="count">${openObl} open</span></h2>
          ${oblRows? `<div class="twrap"><table><thead><tr>${TH("kind","obligation kind")}${TH("todo","related todo")}${TH("evidence","why it was raised")}${TH("state","open / cleared")}${TH("raised","when")}</tr></thead><tbody>${oblRows}</tbody></table></div>`:`<div class="empty card">None</div>`}
          ${accRows? `<div class="sect"><h2 data-tip="acceptance conditions that must ALL be satisfied for terminal closure">Acceptance gaps</h2><div class="twrap"><table><thead><tr>${TH("id","gap id")}${TH("condition","what must be true")}${TH("state","satisfied / open")}</tr></thead><tbody>${accRows}</tbody></table></div></div>`:""}
        </div>
      </div>`;
    return;
  }

  if(dTab === "runs"){
    const runRows = g.runs.map(r => {
      const v = r.validation ? `<span class="chip" data-tip="independent verify receipt: ${esc(r.validation.summary||"")}${r.validation.exit_code!=null?" (exit "+r.validation.exit_code+")":""}">${esc(r.validation.status)}</span>` : "";
      const fk = r.failure_kind && r.failure_kind!=="none" ? badge(r.failure_kind.replace(/_/g," "),"b-bad","why the turn did not deliver a verified outcome") : "";
      return `<tr><td class="mono" data-tip="turn ordinal">#${r.turn}</td><td class="mono">${esc(r.todo_id)}</td>
        <td class="mono" data-tip="${esc(r.run_id)}">${esc((r.run_id||"").slice(0,14))}</td>
        <td>${badge(r.terminal_state||"?", (r.terminal_state||"").includes("succe")||(r.terminal_state||"").includes("complet")?"b-ok":((r.terminal_state||"").includes("error")||(r.terminal_state||"").includes("fail")?"b-bad":"b-mut"),"turn terminal state")} ${v} ${fk}</td>
        <td class="num" data-tip="input / output tokens this turn">${tok(r.tokens_in_delta)} / ${tok(r.tokens_out_delta)}</td>
        <td class="num" data-tip="cost this turn">${money(r.cost_delta)}</td>
        <td class="ttitle"><div class="t" data-tip="${esc(r.evidence)}">${esc(r.evidence||"—")}</div></td>
        <td class="mono" data-tip="${tsLocal(r.recorded_at)}" style="white-space:nowrap">${ago(r.recorded_at)}</td></tr>`;
    }).join("");
    const semRows = (g.semantic_history||[]).slice(-30).reverse().map(s => `<tr>
      <td class="mono" style="white-space:nowrap">${ago(s.ts)}</td><td>${badge(s.kind||"evt","b-mut","semantic event kind")}</td><td class="ttitle"><div class="t" data-tip="${esc(s.summary||s.text||"")}">${esc(s.summary||s.text||JSON.stringify(s))}</div></td></tr>`).join("");
    el.innerHTML = `
      <div class="sect"><h2 data-tip="every bounded turn: validation receipt, failure classification, tokens, cost, evidence">Run ledger <span class="count">${g.runs.length}</span></h2>
        ${g.runs.length? `<div class="twrap"><table><thead><tr>
          ${TH("turn","turn ordinal")}${TH("todo","todo worked")}${TH("run","run id")}${TH("outcome","terminal state + verify receipt + failure kind")}${TH("tok in/out","input / output tokens")}${TH("cost","turn cost")}${TH("evidence","what landed")}${TH("when","recorded at")}
        </tr></thead><tbody>${runRows}</tbody></table></div>`:`<div class="empty card">No runs recorded yet</div>`}
      </div>
      <div class="sect"><h2 data-tip="bounded goal-level semantic history (recent public-safe event summaries)">Semantic history</h2>
        ${semRows? `<div class="twrap"><table><tbody>${semRows}</tbody></table></div>`:`<div class="empty card">No semantic events</div>`}
      </div>`;
    return;
  }

  if(dTab === "events"){
    el.innerHTML = `<div class="sect"><h2 data-tip="the canonical event-sourced ledger — goal state is a replay of these events">Event ledger <span class="count">newest first · ${g.event_count} total</span></h2>
      <div id="events"><div class="empty card">loading…</div></div></div>`;
    renderEvents();
    return;
  }
}

function renderEvents(){
  const el = $("#events"); if(!el) return;
  if(!EVENTS || !EVENTS.length){ el.innerHTML = `<div class="empty card">No events</div>`; return; }
  el.innerHTML = `<div class="twrap"><table><thead><tr>${TH("when","event timestamp")}${TH("kind","event type")}${TH("event id","content-derived id (idempotent append)")}${TH("payload","full event body")}</tr></thead><tbody>
    ${EVENTS.map(e => `<tr><td class="mono" data-tip="${tsLocal(e.ts)}" style="white-space:nowrap">${ago(e.ts)}</td><td>${badge(e.kind,"b-mut","event type")}</td>
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
  const cost = t.runs? `<div class="fgroup"><div class="fk" data-tip="aggregated over ${t.runs} turn(s) on this todo">runs · tokens · cost</div>
    <div class="fv mono">${t.runs} runs · ${tok(t.tokens_in)}↓ ${tok(t.tokens_out)}↑ · ${money(t.cost)}</div></div>` : "";
  const span = t.first_run_at? `<div class="fgroup"><div class="fk">activity window</div><div class="fv mono">${tsLocal(t.first_run_at)} → ${tsLocal(t.last_run_at)}</div></div>` : "";
  openDrawer(`Todo · ${esc(t.id)}`, `
    <div style="display:flex;gap:6px;flex-wrap:wrap;margin-bottom:12px">${statusBadge(t.status)}${classBadge(t.class)}
      <span class="prio prio-${esc(t.priority)}" data-tip="priority P0/P1/P2">${esc(t.priority)}</span>${badge(t.role,"b-mut","owner role: agent or user")}
      ${t.blocked?badge("blocked","b-bad","held by an open gate/blocker/predecessor"):""}${t.archive_state==="archived"?badge("archived","b-mut"):""}</div>
    <div class="fgroup"><div class="fk">text</div><div class="fv">${esc(t.text)}</div></div>
    ${cost}${span}
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

/* ── dependency graph (layered DAG, auto-height, no libs) ── */
function renderGraph(g){
  const el = $("#graph"); if(!el) return;
  const todos = g.todos.filter(t=>t.archive_state!=="archived");
  if(!todos.length){ el.innerHTML = `<div class="empty">No todos</div>`; return; }
  const edges = [];
  for(const t of todos){ for(const pred of (t.blocked_by||[])){ if(todos.find(x=>x.id===pred)) edges.push([pred, t.id]); }
    for(const s of (t.successor_ids||[])){ if(todos.find(x=>x.id===s)) edges.push([t.id, s]); } }
  const memo = {};
  function depth(id, seen){ if(memo[id]!=null) return memo[id]; if(seen.has(id)) return 0;
    seen.add(id); const preds = edges.filter(e=>e[1]===id).map(e=>e[0]);
    const d = preds.length? Math.max(...preds.map(p=>depth(p,seen)))+1 : 0; seen.delete(id); memo[id]=d; return d; }
  const layer = {}; todos.forEach(t=>{ layer[t.id] = edges.length? depth(t.id, new Set()) : 0; });
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
    const tipTxt = `${t.id} · ${t.priority} · ${t.status} · ${t.class.replace(/_/g," ")}${t.text? "\n"+t.text:""}`;
    return `<g class="gnode" data-id="${esc(t.id)}" transform="translate(${p.x},${p.y})" data-tip="${esc(tipTxt)}" onclick="inspectTodo('${esc(t.id)}')">
      <rect width="${W}" height="${H}" style="stroke:${stColor(t.status)};stroke-width:${t.status==="open"?1.8:1}"></rect>
      <text x="10" y="21">${clsGlyph(t.class)} ${esc(short)}</text>
      <text class="gsub" x="10" y="39">${esc(t.id)} · ${esc(t.priority)} · ${esc(t.status)}${t.claimed_by?" · "+esc(t.claimed_by):""}</text></g>`; }).join("");
  const paths = edges.map(([a,b]) => { const p1 = pos[a], p2 = pos[b]; if(!p1||!p2) return "";
    const x1 = p1.x+W, y1 = p1.y+H/2, x2 = p2.x, y2 = p2.y+H/2; const mx = (x1+x2)/2;
    return `<path class="gedge" d="M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}"/>`; }).join("");
  el.innerHTML = `<div style="overflow:auto"><svg width="${Math.max(width,300)}" height="${Math.max(height,60)}" style="display:block">
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
