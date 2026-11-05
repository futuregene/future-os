/**
 * `remote-cn` surface: the deployed Future platform, exercised over the network.
 *
 * The self-hosted relay driver next to this one proves the wire protocol end to
 * end with no account. This one covers the half it cannot: the **official**
 * relay, where a platform service — not the desktop — issues the NATS credential,
 * per account and per pairing. Those are different code paths in this product
 * (`host/src/remote_host/pairing.rs` calls `/client/v1/remote/...`), and they are
 * the paths a user actually goes through.
 *
 * Two modes, because they cost different things:
 *
 *   contract (default) — read-only. Reaches the platform over TLS, checks that
 *     the remote endpoints exist and still enforce their contract: an
 *     unauthenticated device list answers 401 with `unauthorized`, an
 *     unauthenticated claim is rejected for a missing field rather than 404 or
 *     500. This catches "the platform is down / the route moved / the error shape
 *     changed" on every run, and touches nothing on anyone's account.
 *
 *   full (FUTURE_REMOTE_CN_FULL=1) — pairs a real device through the deployed
 *     platform. It needs the machine to be signed in (`future auth login`), it
 *     creates a device entry on that account, and it removes it again at the end.
 *     Skipped with a stated reason when the account is not signed in.
 *
 * The skip is a first-class result: `signedIn: false` is this product saying
 * "an API key is not a session", and a harness that turned that into a red run
 * would be reporting the environment as a defect.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, cpSync, existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const REPO_ROOT = resolve(import.meta.dirname, "..", "..");
const CLI_HOME = resolve(import.meta.dirname, "..", ".data", "cli");
const PLATFORM = process.env.FUTURE_REMOTE_CN_URL || "https://future-os.cn";

const sleep = ms => new Promise(done => setTimeout(done, ms));

function slug(text) {
  return String(text).replace(/[^\w\u4e00-\u9fa5-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "case";
}

function readJson(text) {
  try {
    return JSON.parse(text);
  }
  catch {
    return null;
  }
}

function run(bin, args, { home, timeout = 60_000 } = {}) {
  try {
    const stdout = execFileSync(bin, args, { encoding: "utf8", timeout, env: { ...process.env, FUTURE_HOME: home } });
    return { exit: 0, stdout, stderr: "" };
  }
  catch (error) {
    return { exit: error.status ?? 1, stdout: error.stdout ?? "", stderr: error.stderr ?? String(error.message) };
  }
}

/** One HTTP call with the timeout and shape a test can assert on. */
async function http(url, { method = "GET", body = null, timeout = 15_000 } = {}) {
  const started = Date.now();
  try {
    const response = await fetch(url, {
      method,
      headers: body ? { "content-type": "application/json" } : {},
      body: body ? JSON.stringify(body) : undefined,
      signal: AbortSignal.timeout(timeout),
    });
    const text = await response.text();
    return { status: response.status, text, ms: Date.now() - started, error: null };
  }
  catch (error) {
    return { status: 0, text: "", ms: Date.now() - started, error: String(error?.message ?? error) };
  }
}

export async function runRemoteCnCase({ testCase, runDir, bin }) {
  const caseDir = join(runDir, slug(testCase.id));
  mkdirSync(caseDir, { recursive: true });
  const home = join(caseDir, "home");
  mkdirSync(home, { recursive: true });
  if (existsSync(CLI_HOME))
    cpSync(CLI_HOME, home, { recursive: true });

  const startedAt = Date.now();
  const checks = [];
  const timeline = [];
  const notes = [];
  let status = "pass";
  let fatal = null;
  let index = 0;

  const note = (entry) => timeline.push({ ...entry, index: ++index, ts: Date.now() - startedAt });
  const check = (message, ok, actual, expected = null) => {
    checks.push({ kind: "check", message, ok, actual, expected });
    note({ action: "check", ok, value: actual, note: message });
    return ok;
  };

  const full = process.env.FUTURE_REMOTE_CN_FULL === "1";
  const platform = PLATFORM.replace(/\/+$/, "");

  try {
    // ── 1. the platform answers over TLS ────────────────────────────────────
    const root = await http(`${platform}/`);
    if (!check(`${platform} 可达（TLS 握手 + HTTP 响应）`, root.status > 0 && root.status < 500, `${root.status}${root.error ? ` ${root.error}` : ""}`, "< 500`"))
      throw new Error(`platform unreachable: ${root.error ?? root.status}`);
    notes.push(`platform ${platform} → HTTP ${root.status} in ${root.ms}ms`);

    // ── 2. the remote endpoints exist and still enforce auth ────────────────
    const devices = await http(`${platform}/client/v1/remote/devices`);
    const devicesBody = readJson(devices.text) ?? {};
    check(
      "未登录访问设备列表被拒绝（路由存在且要求会话）",
      devices.status === 401 && devicesBody.error === "unauthorized",
      `${devices.status} ${devicesBody.error ?? devices.text.slice(0, 120)}`,
      "401 unauthorized",
    );

    const claim = await http(`${platform}/client/v1/remote/pair/claim`, { method: "POST", body: {} });
    const claimBody = claim.text;
    // 422 naming the missing field is the contract: it proves the route is wired
    // to a typed handler, not that a proxy swallowed the request.
    check(
      "未登录认领邀请被按契约拒绝（指出缺少 nonce）",
      claim.status === 422 && claimBody.includes("nonce"),
      `${claim.status} ${claimBody.slice(0, 160)}`,
      "422 且提到 nonce",
    );

    const token = await http(`${platform}/client/v1/remote/auth/token`, { method: "POST", body: {} });
    check(
      "未登录换取中继凭据被拒绝",
      token.status === 401 || token.status === 422,
      `${token.status} ${token.text.slice(0, 120)}`,
      "401 或 422",
    );

    // ── 3. what the local installation can actually do ──────────────────────
    // A copy of the real auth, so the answer is about this machine's credentials
    // rather than about a fabricated file.
    const authPath = resolve(process.env.HOME ?? process.env.USERPROFILE ?? "", ".future/agent/auth.json");
    if (existsSync(authPath)) {
      cpSync(authPath, join(home, "auth.json"));
      const status0 = readJson(run(bin, ["remote", "status", "--json"], { home }).stdout) ?? {};
      check("本机 remote 状态可读", typeof status0.serving === "boolean", JSON.stringify({ signedIn: status0.signedIn, state: status0.state }));
      notes.push(`本机 signedIn=${status0.signedIn}（API key 不等于登录会话；官方中继配对需要 future auth login）`);
    }
    else {
      notes.push("没有找到 auth.json，跳过本机凭据检查");
    }

    // ── 4. optional: pair a real device through the deployed platform ───────
    if (full) {
      const signedIn = readJson(run(bin, ["remote", "status", "--json"], { home }).stdout)?.signedIn === true;
      if (!signedIn) {
        notes.push("FUTURE_REMOTE_CN_FULL=1 但本机未登录官方账号，完整配对已跳过（先 future auth login）");
        check("完整配对需要已登录（已按说明跳过，不计失败）", true, "signedIn=false");
      }
      else {
        await fullPairing({ bin, home, caseDir, checks, notes, platform });
      }
    }
    else {
      notes.push("完整配对未启用（FUTURE_REMOTE_CN_FULL=1 才会在官方中继上真的配一台设备）");
    }

    if (checks.some(item => !item.ok))
      status = "fail";
  }
  catch (error) {
    status = "fail";
    fatal = String(error?.message ?? error);
  }

  const result = {
    id: testCase.id,
    title: testCase.title,
    surface: "remote-cn",    area: testCase.area ?? "remote",
    priority: testCase.priority ?? null,
    description: testCase.description ?? null,
    preconditions: testCase.preconditions ?? null,
    expectations: testCase.expectations ?? [],
    judge: testCase.judge ?? null,
    status,
    fatal,
    startedAt: new Date(startedAt).toISOString(),
    durationMs: Date.now() - startedAt,
    steps: timeline,
    checks,
    perf: { durationMs: Date.now() - startedAt },
    observations: [],
    // There is no screen on this surface. What the model reads is what came back:
    // statuses, bodies, and the notes about which mode ran.
    bodyText: notes.join("\n"),
    console: { errors: [], warnings: [], all: [] },
    exceptions: [],
    network: { failures: [], count: 0 },
    artifacts: { dir: caseDir },
  };
  writeFileSync(join(caseDir, "result.json"), JSON.stringify(result, null, 2));
  return result;
}

/**
 * The full path: host serves through the deployed platform, a second home pairs
 * with its invitation, and the pairing is revoked afterwards.
 *
 * Kept in one function because it is one transaction — a half-run leaves a device
 * slot on the user's account, which is the one side effect this case can have.
 */
async function fullPairing({ bin, home, caseDir, checks, notes, platform }) {
  const clientHome = join(caseDir, "client-home");
  mkdirSync(clientHome, { recursive: true });
  if (existsSync(join(home, "auth.json")))
    cpSync(join(home, "auth.json"), join(clientHome, "auth.json"));

  // The host runs as its own process so the platform sees a real bridge.
  const { spawn } = await import("node:child_process");
  const hostLog = join(caseDir, "host.log");
  const child = spawn(bin, ["host"], { cwd: caseDir, stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, FUTURE_HOME: home } });
  const out = [];
  const append = chunk => {
    out.push(chunk);
    writeFileSync(hostLog, Buffer.concat(out).toString("utf8"));
  };
  child.stdout.on("data", append);
  child.stderr.on("data", append);

  const ready = Date.now() + 90_000;
  let served = false;
  while (Date.now() < ready) {
    const status = readJson(run(bin, ["remote", "status", "--json"], { home }).stdout);
    if (status?.serving) {
      served = true;
      break;
    }
    await sleep(500);
  }
  const cleanups = [() => {
    try {
      child.kill("SIGTERM");
    }
    catch {}
  }];
  try {
    checks.push({ kind: "check", message: "主机在官方平台进入 serving", ok: served, actual: served });
    if (!served) {
      notes.push(`host.log:\n${out.join("").slice(-1500)}`);
      return;
    }
    const invite = readJson(run(bin, ["remote", "devices", "add", "--json"], { home }).stdout);
    checks.push({ kind: "check", message: "官方平台签发邀请", ok: Boolean(invite?.link), actual: invite?.pairId ?? null });
    if (!invite?.link)
      return;
    const paired = readJson(run(bin, ["remote", "peers", "add", invite.link, "--json"], { home: clientHome }).stdout);
    checks.push({ kind: "check", message: "经官方中继配对成功", ok: Boolean(paired?.paired?.pairId), actual: paired?.paired?.pairId ?? null });
    checks.push({ kind: "check", message: "客户端拿到被控端 agent 能力", ok: paired?.paired?.agentAvailable === true, actual: paired?.paired?.agentAvailable ?? null });
    cleanups.push(() => {
      if (paired?.paired?.desktopId)
        run(bin, ["remote", "peers", "remove", paired.paired.desktopId, "--yes"], { home: clientHome });
    });
    // The device slot lives on the account, so revoke it even when the client is gone.
    if (paired?.paired?.pairId)
      run(bin, ["remote", "devices", "remove", paired.paired.pairId, "--yes"], { home });
    notes.push(`platform ${platform}；完整配对完成并已撤销`);
  }
  finally {
    for (const cleanup of cleanups.reverse()) {
      try {
        cleanup();
      }
      catch {}
    }
  }
}
