#!/usr/bin/env python3
"""Tiny mock Future platform server for the TS <-> Rust CLI diff harness.

Usage:
    python3 mock-platform-server.py <port> [ok|errors]

Serves the few endpoints the CLI's P1 commands touch, with fixed bodies so the
TypeScript and Rust CLIs produce byte-identical output when pointed at it via
auth.json `base_url` (or `future auth login --url`):

    GET  /client/v1/skills             -> 200 {"skills": []}
    GET  /client/v1/account/profile    -> 200 fixed profile (mode "ok")
    GET  /client/v1/account/balance    -> 200 fixed balance  (mode "ok")
    POST /client/v1/oauth/device/code  -> 500 {"message": "device code denied"}
    POST /client/v1/oauth/device/token -> 500 {"message": "no device"}

Mode "errors" returns 401 {"error": "bad key"} for the account endpoints so
the harness can exercise the error-message fallback path
(body.message ?? body.error ?? HTTP status).
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

MODE = sys.argv[2] if len(sys.argv) > 2 else "ok"

PROFILE = {
    "email": "diff@test.local",
    "user_id": "u-12345",
    "email_verified": True,
    "created_at": "2026-01-01T00:00:00Z",
}
BALANCE = {"balance_credits": 1234567890123}


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, obj):
        body = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass  # keep the harness output clean

    def do_GET(self):
        if MODE == "errors" and self.path.startswith("/client/v1/account/"):
            return self._send(401, {"error": "bad key"})
        if self.path == "/client/v1/skills":
            return self._send(200, {"skills": []})
        if self.path == "/client/v1/account/profile":
            return self._send(200, PROFILE)
        if self.path == "/client/v1/account/balance":
            return self._send(200, BALANCE)
        return self._send(404, {"message": "not found"})

    def do_POST(self):
        if self.path == "/client/v1/oauth/device/code":
            return self._send(500, {"message": "device code denied"})
        if self.path == "/client/v1/oauth/device/token":
            return self._send(500, {"message": "no device"})
        return self._send(404, {"message": "not found"})


if __name__ == "__main__":
    port = int(sys.argv[1])
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
