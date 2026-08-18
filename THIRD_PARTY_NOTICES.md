# Third-Party Notices

FutureOS as a whole is distributed under the [MIT License](LICENSE). The
components below carry their own licenses and attributions, which are
retained in the locations indicated.

## LoopX — Apache License, Version 2.0

- **Upstream:** <https://github.com/huangruiteng/loopx> — Ruiteng Huang and
  LoopX contributors
- **Where:** [`orchestration/loop/`](orchestration/loop/) (crate
  `future-loop`) — the Future Loop control plane contains code translated,
  ported, and structurally adapted from LoopX v0.4.x (through v0.4.8).
- **License:** [Apache License, Version 2.0](orchestration/loop/LICENSE).
  LoopX releases through v0.4.7 were distributed under the MIT License;
  v0.4.8 is the first Apache-2.0 release.
- **Notices:** [`orchestration/loop/NOTICE`](orchestration/loop/NOTICE);
  the upstream base version, derivation scope, and FutureGene's
  modifications are documented in
  [`orchestration/loop/UPSTREAM.md`](orchestration/loop/UPSTREAM.md).
- **Relationship:** Future Loop is an independent downstream implementation
  maintained by FutureGene — it is **not** an official LoopX release and
  has **not** been certified or endorsed by the LoopX project.

Portable release archives and disk images that embed the `future` binary
include these notices under `licenses/`.
