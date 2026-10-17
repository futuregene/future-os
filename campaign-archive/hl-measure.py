#!/usr/bin/env python3
"""Measure the tauri-headless record against a given report over its 12 scopes."""
import subprocess
import sys

PATHS = """terminal/server.rs terminal/pty.rs terminal/session.rs terminal/manager.rs
terminal/shell.rs terminal/cwd.rs headless/mod.rs headless/agent.rs cli_tool.rs
agent_host.rs agent_supervisor.rs skills_bootstrap.rs""".split()

report = sys.argv[1]
cmd = ["python3", ".future/cov100/reconcile.py", "--report", report,
       "--doc", "docs/testing/module-tauri-headless.md",
       "--section", "4. Residual uncovered lines"]
for p in PATHS:
    cmd += ["--scope", f"desktop/src-tauri/src/{p}"]
out = subprocess.run(cmd, capture_output=True, text=True)
print(out.stdout or out.stderr)
