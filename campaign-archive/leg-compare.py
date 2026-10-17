#!/usr/bin/env python3
"""Which leg actually earns the app-data residual? Per-leg and union counts over the eleven scopes."""
SCOPES = ["src/commands", "src/tasks.rs", "src/store", "src/scheduler", "src/agent_bridge",
          "src/agent_providers", "src/shadow_review", "src/error.rs", "src/auth_store.rs",
          "src/agent_events.rs"]


def rel(sf):
    i = sf.find("/desktop/src-tauri/")
    return sf[i + len("/desktop/src-tauri/"):] if i >= 0 else sf


def in_scope(sf):
    r = rel(sf)
    return any(r == s or r.startswith(s.rstrip("/") + "/") for s in SCOPES)


def load(p):
    out = {}
    cur = None
    keep = False
    for line in open(p, encoding="utf-8", errors="replace"):
        if line.startswith("SF:"):
            cur = line.rstrip()[3:]
            keep = in_scope(cur)
        elif line.startswith("DA:") and keep:
            q = line.rstrip()[3:].split(",")
            if len(q) >= 2:
                k = (rel(cur), int(q[0]))
                out[k] = max(out.get(k, 0), int(q[1]))
    return out


def rep(lbl, u):
    print(f"{lbl:26s} keys={len(u):6d} uncovered={sum(1 for v in u.values() if v == 0)}")


g18 = load("/tmp/appdata-s18-gui.info")
h18 = load("/tmp/appdata-s18-headless.info")
h17 = load("/tmp/appdata-taub-s17-headless.info")

rep("gui18 alone", g18)
rep("hl18 alone (the unit's)", h18)
rep("hl17 alone (the peer's)", h17)


def uni(a, b):
    return {**a, **{k: max(a.get(k, 0), v) for k, v in b.items()}}


rep("gui18+hl18 (control)", uni(g18, h18))
rep("gui18+hl17 (variant)", uni(g18, h17))
for lbl, h in (("hl18", h18), ("hl17", h17)):
    print(f"{lbl}: lines covered that gui18 leaves uncovered = "
          f"{sum(1 for k, v in h.items() if v > 0 and g18.get(k, 0) == 0)}; "
          f"keys absent from gui18 = {len([k for k in h if k not in g18])}")
