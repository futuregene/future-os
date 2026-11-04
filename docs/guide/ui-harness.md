# UI harness: end-to-end testing that watches the product

The UI harness drives the real desktop frontend against a snapshot of the real
database, records every observable channel, and uses vision and text models to
turn the evidence into findings, a report, and issue drafts that a human
approves.

The working reference — case schema, observation channels, analysis pipeline,
fidelity boundaries and current gaps — is
[`harness/README.md`](../../harness/README.md). This page covers only what that
file cannot: where the harness sits among the existing tools, and how to run it
on a machine that already has the app installed.

## Where it fits

The [screenshot harness](screenshots.md) renders the same frontend for
documentation. It answers "what does this screen look like?" with curated demo
data and one scenario per image. The UI harness answers a different question:
"what happens when a person uses this product for a while, on a real-sized
database?" It therefore:

- copies `~/.future/app/app.db` instead of using `desktop/shot/mock/data.ts`;
- keeps state between steps and asserts on both the DOM and the database;
- records a timeline, video, console, network, IPC and performance channels, not
  just stills;
- treats a failing case as a defect to report, never as a reason to relax the
  assertion.

Both share the same page (`desktop/shot/`), the same Vite config, and the same
accessibility-first element matching, so a control found by one is found by the
other.

## Running it

```bash
node harness/cli.mjs prepare
node harness/cli.mjs list
node harness/cli.mjs run --case create-chat-and-delete
node harness/cli.mjs analyze --run harness/runs/<id> --mode full
node harness/cli.mjs report  --run harness/runs/<id>
```

Requirements: Node.js, Chrome/Chromium, Python 3 only for the shared Vite
launcher (the harness starts it itself), `ffmpeg` for video, and a `future`
binary whose release version matches the running agent for the analysis step.
Set `FUTURE_BIN` when the repository binary is not the one to use.

## What it cannot prove

Backend Rust/SQL behaviour, model output quality, real network conditions and
the mobile app are outside this harness. It is a UI behaviour harness with
real-shaped data, not a replacement for integration or contract tests. The
report repeats this boundary so a reader never mistakes a green run for "the
backend is fine".
