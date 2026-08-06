#!/usr/bin/env python3
"""Tiny mock Future platform server for the TS <-> Rust CLI diff harness.

Usage:
    python3 mock-platform-server.py <port> [ok|errors|skills]

Serves the endpoints the CLI's commands touch, with fixed bodies so the
TypeScript and Rust CLIs produce byte-identical output when pointed at it via
auth.json `base_url` (or `future auth login --url`):

    GET  /client/v1/skills                     -> 200 {"skills": [...]}
    GET  /client/v1/skills/<id>/versions/<v>/download -> 200 deterministic zip
    GET  /client/v1/account/profile            -> 200 fixed profile (mode "ok")
    GET  /client/v1/account/balance            -> 200 fixed balance  (mode "ok")
    POST /client/v1/oauth/device/code          -> 500 {"message": "device code denied"}
    POST /client/v1/oauth/device/token         -> 500 {"message": "no device"}
    POST /api/v1/mcp                           -> MCP SSE (initialize /
                                                 tools/list / tools/call)

Mode "errors" returns 401 {"error": "bad key"} for the account endpoints so
the harness can exercise the error-message fallback path.

Mode "skills" serves a small skill catalog (two `future-*` builtins plus one
community skill) with deterministic download zips so `skills list/install/
uninstall/update` and `init` can be diffed end to end.

MCP endpoints are served in every mode (tools list/describe/call diff cases
run against the "ok" mock). Responses are fixed — the request body is only
read to pick the tool-specific reply — so both CLIs see identical bytes.
"""

import io
import json
import sys
import threading
import zipfile
from http.server import BaseHTTPRequestHandler, HTTPServer

MODE = sys.argv[2] if len(sys.argv) > 2 else "ok"

PROFILE = {
    "email": "diff@test.local",
    "user_id": "u-12345",
    "email_verified": True,
    "created_at": "2026-01-01T00:00:00Z",
}
BALANCE = {"balance_credits": 1234567890123}

SKILLS = [
    {
        "id": "future-test-a",
        "name": "Test A",
        "description": "A test skill for the diff harness.",
        "category": "test",
        "price": "0",
        "formats": "md",
        "limit": "0",
        "latest_version": "1.0.0",
    },
    {
        "id": "future-test-b",
        "name": "Test B",
        "description": "Second test skill.",
        "category": "test",
        "price": "0",
        "formats": "md",
        "limit": "0",
        "latest_version": "2.1.0",
    },
    {
        "id": "community-x",
        "name": "Community X",
        "description": "A community skill, not builtin.",
        "category": "community",
        "price": "0",
        "formats": "md",
        "limit": "0",
        "latest_version": "0.9.0",
    },
]

MCP_TOOLS = [
    {"name": "search_paper", "description": "Search academic papers."},
    {"name": "web_search", "description": "Search the web."},
    {"name": "mock_special", "description": "A special mock tool for the diff harness."},
]

# tools/call replies per tool name — fixed bytes so both CLIs format the same.
MCP_CALL_RESULTS = {
    "search_paper": {
        "result": {
            "content": [
                {
                    "type": "text",
                    "text": "Mock search paper result.",
                }
            ],
            "structuredContent": {
                "results": [
                    {
                        "query": "mock",
                        "papers": [
                            {
                                "title": "Mock Paper",
                                "authors": "A. Author",
                                "journal": "Mock Journal",
                                "year": "2025",
                                "doi": "10.1/mock",
                                "url": "https://example.com/paper",
                                "ai_summary": "A summary of the mock paper.",
                            }
                        ],
                    }
                ]
            },
        }
    },
    "web_search": {
        "result": {
            "content": [{"type": "text", "text": "Mock web search result."}],
            "structuredContent": {
                "query": "mock",
                "results": [
                    {"title": "Result One", "link": "https://example.com/1", "snippet": "First snippet."},
                    {"title": "Result Two", "link": "https://example.com/2", "snippet": "Second snippet."},
                ],
            },
        }
    },
    "mock_special": {
        "result": {
            "content": [{"type": "text", "text": "Plain text from mock_special."}],
        }
    },
    "mock_error": {
        "error": {"code": 401, "message": "unauthorized"},
    },
    "mock_no_session": {
        "result": {"content": [{"type": "text", "text": "Mock no-session result."}]},
    },
}


def make_skill_zip(version):
    """Deterministic zip: fixed timestamp/attributes so unzip output matches."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
        info = zipfile.ZipInfo("SKILL.md", date_time=(2026, 1, 1, 0, 0, 0))
        info.external_attr = 0o644 << 16
        zf.writestr(info, "---\nname: Test\nversion: %s\n---\n# Test skill\n" % version)
    return buf.getvalue()


def sse(payload):
    """SSE-encode a JSON-RPC payload (the CLI parses `data:` lines)."""
    return ("data: " + json.dumps(payload) + "\n\n").encode("utf-8")


# ── Mock CDP browser (mode "browser") ─────────────────────────────────
#
# Serves /json/version over HTTP and a minimal CDP WebSocket server so the
# `browser` tool's session commands (tabs/open/snapshot/click/type/press/
# screenshot/scroll/console) can be diffed byte-for-byte between the TS and
# Rust CLIs. Responses are fixed/stateful but deterministic; a POST
# /__reset restores the tab state between binary runs.

CDP_STATE = {"pages": [], "next_target": 1, "next_session": 1, "sessions": {}}


def reset_cdp_state():
    CDP_STATE["pages"] = [
        {"targetId": "target-1", "type": "page", "url": "https://example.com/one", "title": "First Page"},
        {"targetId": "target-2", "type": "page", "url": "https://example.com/two", "title": "Second Page"},
    ]
    CDP_STATE["next_target"] = 3
    CDP_STATE["next_session"] = 1
    CDP_STATE["sessions"] = {}


CDP_VERSION = {
    "Browser": "Chrome/999.0.0.0",
    "Protocol-Version": "1.3",
    "User-Agent": "Mozilla/5.0 (Mock CDP)",
    "V8-Version": "12.0.0",
    "WebKit-Version": "537.36",
}

SNAPSHOT_ITEMS = [
    {"ref": "b1", "selector": "#btn-submit", "role": "button", "name": "Submit", "tag": "button",
     "disabled": False, "checked": None, "href": None},
    {"ref": "i1", "selector": "input[data-testid='email']", "role": "textbox", "name": "Email",
     "tag": "input", "disabled": False, "checked": None, "href": None},
    {"ref": "a1", "selector": "#link-home", "role": "link", "name": "Home", "tag": "a",
     "disabled": False, "checked": None, "href": "https://example.com/"},
    {"ref": "t1", "selector": "h1", "role": "text", "name": "Welcome to the Mock Page", "tag": "h1",
     "disabled": False, "checked": None, "href": None},
]

CONSOLE_LOGS = [
    {"level": "log", "text": "Mock console message one", "time": "2026-08-06T12:00:00.000Z"},
    {"level": "warn", "text": "Mock warning message", "time": "2026-08-06T12:00:01.000Z"},
    {"level": "error", "text": "Mock error message", "time": "2026-08-06T12:00:02.000Z"},
]


def evaluate_value(expr):
    """Deterministic Runtime.evaluate value for a given page expression."""
    if "globalThis.__futureConsoleLogs" in expr:
        return CONSOLE_LOGS
    if "scrollBy" in expr:
        return None
    if "element.isConnected" in expr:
        return {"exists": True, "connected": True, "visible": True, "disabled": False,
                "box": {"x": 10, "y": 20, "width": 100, "height": 40}, "obscured": False}
    if "scrollIntoView" in expr:
        return {"x": 10, "y": 20, "width": 100, "height": 40}
    if "hasSubmitter" in expr:
        # Click default-action metadata capture (before mouse dispatch).
        return {"href": None, "hasSubmitter": False}
    if "Boolean(state.defaultPrevented)" in expr:
        # Click event state read-back.
        return {"defaultPrevented": False, "submitSeen": False}
    if "requestSubmit" in expr:
        return None
    if "Phase 1: interactive elements" in expr:
        # Snapshot: honor the trailing limit argument `(<limit>))`.
        import re
        m = re.search(r"\((\d+)(?:\.0)?\)\)$", expr)
        limit = int(m.group(1)) if m else 80
        return {"title": "Mock Page Title", "url": "https://example.com/mock-page",
                "items": SNAPSHOT_ITEMS[:limit]}
    if "document.title" in expr:
        return "Mock Page Title"
    if "location.href" in expr:
        return "https://example.com/mock-page"
    if "focus" in expr:
        return None
    return None


def cdp_reply(rid, result):
    return {"id": rid, "result": result}


def handle_cdp_ws(conn):
    """Serve one WebSocket client connection. Runs in a per-client thread."""
    import base64
    import hashlib
    import struct

    WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

    def read_exact(n):
        buf = b""
        while len(buf) < n:
            chunk = conn.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("eof")
            buf += chunk
        return buf

    def handshake():
        data = b""
        while b"\r\n\r\n" not in data:
            chunk = conn.recv(4096)
            if not chunk:
                raise ConnectionError("closed during handshake")
            data += chunk
        key = ""
        for line in data.decode("latin1").split("\r\n")[1:]:
            if ":" in line:
                k, v = line.split(":", 1)
                if k.strip().lower() == "sec-websocket-key":
                    key = v.strip()
        accept = base64.b64encode(
            hashlib.sha1((key + WS_GUID).encode()).digest()
        ).decode()
        conn.sendall(
            (
                "HTTP/1.1 101 Switching Protocols\r\n"
                "Upgrade: websocket\r\n"
                "Connection: Upgrade\r\n"
                "Sec-WebSocket-Accept: %s\r\n\r\n" % accept
            ).encode()
        )

    def recv_frame():
        header = read_exact(2)
        b1, b2 = header
        opcode = b1 & 0x0F
        masked = b2 & 0x80
        length = b2 & 0x7F
        if length == 126:
            length = struct.unpack(">H", read_exact(2))[0]
        elif length == 127:
            length = struct.unpack(">Q", read_exact(8))[0]
        mask = read_exact(4) if masked else None
        payload = read_exact(length) if length else b""
        if mask:
            payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        return opcode, payload

    def send_frame(text):
        payload = text.encode("utf-8")
        length = len(payload)
        header = bytearray([0x81])
        if length < 126:
            header.append(length)
        elif length < 65536:
            header.append(126)
            header += struct.pack(">H", length)
        else:
            header.append(127)
            header += struct.pack(">Q", length)
        conn.sendall(bytes(header) + payload)

    try:
        handshake()
        while True:
            opcode, payload = recv_frame()
            if opcode == 0x8:  # close — reply with a close frame and stop
                try:
                    conn.sendall(bytes([0x88, 0x00]))
                except OSError:
                    pass
                break
            if opcode == 0x9:  # ping → pong
                conn.sendall(bytes([0x8A]) + bytes([len(payload)]) + payload)
                continue
            if opcode == 0x1:  # text
                raw = payload.decode("utf-8", "replace")
                try:
                    msg = json.loads(raw)
                except Exception:
                    continue
                rid = msg.get("id")
                method = msg.get("method", "")
                params = msg.get("params") or {}
                sid = msg.get("sessionId")
                if method == "Target.setDiscoverTargets":
                    send_frame(json.dumps(cdp_reply(rid, {})))
                elif method == "Target.getTargets":
                    send_frame(json.dumps(cdp_reply(rid, {"targetInfos": CDP_STATE["pages"]})))
                elif method == "Target.attachToTarget":
                    session_id = "session-%d" % CDP_STATE["next_session"]
                    CDP_STATE["next_session"] += 1
                    CDP_STATE["sessions"][params.get("targetId")] = session_id
                    send_frame(json.dumps(cdp_reply(rid, {"sessionId": session_id})))
                elif method == "Target.createTarget":
                    tid = "target-%d" % CDP_STATE["next_target"]
                    CDP_STATE["next_target"] += 1
                    url = params.get("url", "about:blank")
                    send_frame(json.dumps(cdp_reply(rid, {"targetId": tid})))
                    CDP_STATE["pages"].append(
                        {"targetId": tid, "type": "page", "url": url, "title": ""})
                    send_frame(json.dumps({"method": "Target.targetCreated",
                                           "params": {"targetInfo": CDP_STATE["pages"][-1]}}))
                elif method == "Target.closeTarget":
                    tid = params.get("targetId")
                    CDP_STATE["pages"] = [p for p in CDP_STATE["pages"] if p["targetId"] != tid]
                    CDP_STATE["sessions"].pop(tid, None)
                    # Destroyed event BEFORE the response so a follow-up tabs
                    # list is deterministic.
                    send_frame(json.dumps({"method": "Target.targetDestroyed",
                                           "params": {"targetId": tid}}))
                    send_frame(json.dumps(cdp_reply(rid, {"success": True})))
                elif method == "Target.activateTarget":
                    send_frame(json.dumps(cdp_reply(rid, {})))
                elif method == "Page.navigate":
                    send_frame(json.dumps(cdp_reply(rid, {"frameId": "frame-1", "loaderId": "loader-new"})))
                    send_frame(json.dumps({"method": "Page.lifecycleEvent",
                                           "params": {"frameId": "frame-1", "loaderId": "loader-new",
                                                      "name": "DOMContentLoaded"},
                                           "sessionId": sid}))
                elif method == "Runtime.evaluate":
                    expr = params.get("expression", "")
                    send_frame(json.dumps(cdp_reply(rid, {"result": {"value": evaluate_value(expr)}})))
                elif method == "Page.getFrameTree":
                    send_frame(json.dumps(cdp_reply(rid, {"frameTree": {"frame": {
                        "id": "frame-1", "loaderId": "loader-1"}}})))
                elif method == "Page.captureScreenshot":
                    # 1x1 transparent PNG (fixed bytes).
                    send_frame(json.dumps(cdp_reply(rid, {"data": (
                        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")})))
                else:
                    # Page.enable / Runtime.enable / setLifecycleEventsEnabled /
                    # dispatchMouseEvent / dispatchKeyEvent / insertText /
                    # add/removeScriptToEvaluateOnNewDocument...
                    send_frame(json.dumps(cdp_reply(rid, {})))
    except Exception:
        pass
    finally:
        try:
            conn.close()
        except OSError:
            pass


def start_cdp_ws_server(port):
    """Threaded TCP server speaking a minimal RFC 6455 WebSocket server —
    stdlib only (the harness runs under the system python3 which has no
    `websockets` package)."""
    import socket
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port))
    srv.listen(8)
    srv.settimeout(1.0)
    while True:
        try:
            conn, _ = srv.accept()
        except socket.timeout:
            continue
        threading.Thread(target=handle_cdp_ws, args=(conn,), daemon=True).start()


def pick_free_port(avoid=None):
    """Bind a socket to an ephemeral port, release it, return the number.
    `avoid` (the HTTP port) is skipped — the kernel hands out ephemeral ports
    sequentially, so a just-released probe port can be re-assigned to the WS
    server, colliding with the HTTPServer bind milliseconds later."""
    import socket
    while True:
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
        if port != avoid:
            return port


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _send(self, code, obj, extra_headers=None, raw_body=None, ctype=None):
        if raw_body is not None:
            body = raw_body
            ctype = ctype or "application/zip"
        else:
            body = json.dumps(obj).encode("utf-8")
            ctype = ctype or "application/json"
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        for k, v in (extra_headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass  # keep the harness output clean

    def _do_mcp(self):
        length = int(self.headers.get("Content-Length", "0"))
        try:
            body = json.loads(self.rfile.read(length).decode("utf-8"))
        except Exception:
            body = {}
        method = body.get("method", "")
        rid = body.get("id", 0)

        if method == "initialize":
            return self._send(
                200,
                None,
                extra_headers={"Mcp-Session-Id": "sess-mock-001"},
                raw_body=sse(
                    {
                        "jsonrpc": "2.0",
                        "id": rid,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {},
                            "serverInfo": {"name": "mock-mcp", "version": "1.0"},
                        },
                    }
                ),
            )
        if method == "notifications/initialized":
            return self._send(202, None, raw_body=b"")
        if method == "tools/list":
            return self._send(
                200,
                None,
                raw_body=sse({"jsonrpc": "2.0", "id": rid, "result": {"tools": MCP_TOOLS}}),
            )
        if method == "tools/call":
            name = (body.get("params") or {}).get("name", "")
            reply = MCP_CALL_RESULTS.get(name, MCP_CALL_RESULTS["mock_special"])
            return self._send(
                200,
                None,
                raw_body=sse({"jsonrpc": "2.0", "id": rid, **reply}),
            )
        return self._send(
            200, None, raw_body=sse({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "Method not found"}})
        )

    def do_GET(self):
        if MODE == "errors" and self.path.startswith("/client/v1/account/"):
            return self._send(401, {"error": "bad key"})
        if MODE == "browser" and self.path == "/json/version":
            version = dict(CDP_VERSION)
            version["webSocketDebuggerUrl"] = "ws://127.0.0.1:%d/devtools/browser/mock" % WS_PORT
            return self._send(200, version)
        if self.path == "/client/v1/skills":
            if MODE == "skills":
                return self._send(200, {"skills": SKILLS})
            return self._send(200, {"skills": []})
        # /client/v1/skills/<id>/versions/<v>/download
        parts = self.path.split("/")
        if len(parts) == 8 and parts[1] == "client" and parts[7] == "download":
            skill_id, version = parts[4], parts[6]
            if MODE == "skills":
                return self._send(
                    200,
                    None,
                    raw_body=make_skill_zip(version),
                )
            return self._send(404, {"message": "skill not found"})
        if self.path == "/client/v1/account/profile":
            return self._send(200, PROFILE)
        if self.path == "/client/v1/account/balance":
            return self._send(200, BALANCE)
        return self._send(404, {"message": "not found"})

    def do_POST(self):
        if MODE == "browser" and self.path == "/__reset":
            reset_cdp_state()
            return self._send(200, {"ok": True})
        if self.path == "/api/v1/mcp":
            return self._do_mcp()
        if self.path == "/client/v1/oauth/device/code":
            return self._send(500, {"message": "device code denied"})
        if self.path == "/client/v1/oauth/device/token":
            return self._send(500, {"message": "no device"})
        return self._send(404, {"message": "not found"})


if __name__ == "__main__":
    port = int(sys.argv[1])
    PORT = port
    if MODE == "browser":
        reset_cdp_state()
        WS_PORT = pick_free_port(avoid=port)
        threading.Thread(target=start_cdp_ws_server, args=(WS_PORT,), daemon=True).start()
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
