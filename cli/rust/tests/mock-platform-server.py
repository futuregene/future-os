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
        if self.path == "/api/v1/mcp":
            return self._do_mcp()
        if self.path == "/client/v1/oauth/device/code":
            return self._send(500, {"message": "device code denied"})
        if self.path == "/client/v1/oauth/device/token":
            return self._send(500, {"message": "no device"})
        return self._send(404, {"message": "not found"})


if __name__ == "__main__":
    port = int(sys.argv[1])
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
