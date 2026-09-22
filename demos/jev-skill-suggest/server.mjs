#!/usr/bin/env node
// Local web demo: skill recommendation while you type.
//
//   TYPESAFE_API_KEY=jev_... node server.mjs [--port 8787]
//
// The API key stays in this process; the browser never sees it.
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { loadRoster } from "./roster.mjs";
import { Suggester, MIN_QUERY_CHARS, SHORTLIST, NONE_GATE_THRESHOLD } from "./suggest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const argOf = (flag) => {
  const i = argv.indexOf(flag);
  return i === -1 ? undefined : argv[i + 1];
};

const PORT = Number(argOf("--port") ?? process.env.PORT ?? 8787);
const SKILLS_ROOT = path.resolve(
  argOf("--skills") ?? process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "skills"),
);
const PUBLIC_DIR = path.join(here, "public");

const roster = loadRoster(SKILLS_ROOT);
const suggester = new Suggester(roster, {
  apiKey: process.env.TYPESAFE_API_KEY,
  baseUrl: process.env.TYPESAFE_BASE_URL,
  model: process.env.TYPESAFE_MODEL,
  insecureLocalOnly: process.env.LOCAL_ONLY === "1",
});

const json = (res, status, body) => {
  const payload = JSON.stringify(body);
  res.writeHead(status, { "content-type": "application/json; charset=utf-8", "content-length": Buffer.byteLength(payload) });
  res.end(payload);
};

const MIME = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".svg": "image/svg+xml" };

async function serveStatic(res, pathname) {
  const rel = pathname === "/" ? "index.html" : pathname.replace(/^\/+/, "");
  const file = path.join(PUBLIC_DIR, rel);
  if (!file.startsWith(PUBLIC_DIR)) return json(res, 403, { error: "forbidden" });
  try {
    const body = await readFile(file);
    res.writeHead(200, { "content-type": MIME[path.extname(file)] || "application/octet-stream" });
    res.end(body);
  } catch {
    json(res, 404, { error: "not found" });
  }
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host || "localhost"}`);
  const started = Date.now();

  if (req.method === "GET" && url.pathname === "/api/status") {
    return json(res, 200, { ...suggester.status(), minQueryChars: MIN_QUERY_CHARS, shortlist: SHORTLIST, gateThreshold: NONE_GATE_THRESHOLD });
  }

  if (req.method === "GET" && url.pathname === "/api/roster") {
    return json(res, 200, {
      total: roster.skills.length,
      builtin: roster.builtinCount,
      thirdParty: roster.thirdPartyCount,
      skills: roster.skills.map(({ name, builtin, category, nameZh, description, descriptionZh, folder }) => ({
        name,
        builtin,
        category,
        nameZh,
        description,
        descriptionZh,
        folder,
      })),
    });
  }

  if (req.method === "POST" && url.pathname === "/api/suggest") {
    let raw = "";
    for await (const chunk of req) raw += chunk;
    let body;
    try {
      body = JSON.parse(raw || "{}");
    } catch {
      return json(res, 400, { error: "invalid json" });
    }

    const query = String(body.query ?? "").trim();
    if (query.length < MIN_QUERY_CHARS) {
      return json(res, 200, { skipped: true, reason: `输入少于 ${MIN_QUERY_CHARS} 个字符`, backend: suggester.status().mode });
    }

    try {
      // One call, one threshold. The ranking comes back already ordered by the Choice's
      // probability, and the gate reads how much of that probability the model put on
      // none_of_these — so a refused request is the model's own verdict, not a cutoff we applied
      // to a score it does not have a scale for.
      const rank = await suggester.rank(query);
      const gatePasses = (rank.gate.noneProbability ?? 0) < NONE_GATE_THRESHOLD;
      const verdict = gatePasses ? (rank.top[0]?.name ?? null) : null;

      console.log(
        `[${new Date().toISOString()}] backend=${rank.backend} gate=${rank.gate.mean} ` +
          `top1=${rank.top[0]?.name}(${rank.top[0]?.p}) verdict=${verdict ?? "-"} ` +
          `ms=${rank.ms} in=${rank.usage?.input_tokens ?? rank.usage?.prompt_tokens ?? "?"} ` +
          `q="${query.slice(0, 48)}${query.length > 48 ? "…" : ""}"`,
      );

      return json(res, 200, {
        query,
        skipped: false,
        backend: rank.backend,
        status: suggester.status(),
        gate: rank.gate,
        gate_passes: gatePasses,
        gate_threshold: NONE_GATE_THRESHOLD,
        mode: `${rank.backend === "typesafe" ? "one call" : "local fallback"}`,
        verdict,
        top: rank.top,
        ranked: rank.ranked.slice(0, Number(body.limit ?? 12)),
        ranked_total: rank.ranked.length,
        choices_seen: rank.choices_seen,
        rank: { ms: rank.ms, usage: rank.usage, model: rank.model, request: rank.request, attempts: rank.attempts },
        total_ms: Date.now() - started,
      });
    } catch (error) {
      console.error("suggest failed:", error);
      return json(res, 500, { error: error.message });
    }
  }

  if (req.method === "GET") return serveStatic(res, url.pathname);
  json(res, 405, { error: "method not allowed" });
});

server.listen(PORT, "127.0.0.1", async () => {
  const status = await suggester.checkAuth();
  console.log(`skill-reco demo  →  http://127.0.0.1:${PORT}`);
  console.log(`roster: ${roster.skills.length} skills (builtin ${roster.builtinCount} + third-party ${roster.thirdPartyCount}) from ${SKILLS_ROOT}`);
  console.log(`backend: ${status.mode}  ·  ${status.note}`);
  if (status.mode === "typesafe") console.log(`endpoint: POST ${status.baseUrl}/v1/systemone  ·  model ${status.model}`);
});
