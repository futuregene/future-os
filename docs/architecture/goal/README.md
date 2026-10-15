# Future Goal (`future-goal`)

The FutureOS goal control plane: durable goals / todos / gates / quotas, a
deterministic should-run decision kernel, event-sourced state, and a gRPC
executor bridge — productized as the `future goal` CLI.

Documentation: [`ARCHITECTURE.md`](ARCHITECTURE.md) (design principles,
[中文版](ARCHITECTURE.zh-CN.md)) and
[`../goal-control-plane.md`](../goal-control-plane.md) (operational
model).

## License & attribution

The `future-goal` crate (in `orchestration/goal/`) contains code derived from
[LoopX](https://github.com/huangruiteng/loopx) and is distributed under the
**Apache License, Version 2.0** — see [`LICENSE`](../../../orchestration/goal/LICENSE), [`NOTICE`](../../../orchestration/goal/NOTICE),
and [`UPSTREAM.md`](../../../orchestration/goal/UPSTREAM.md) for the upstream base version, the scope of
derived code, and FutureGene's modifications. Original portions
Copyright 2026 LoopX contributors; modifications and original additions
Copyright 2026 FutureGene.

Future Goal is an **independent downstream implementation** maintained by
FutureGene. It is **not** an official LoopX release and has **not** been
certified or endorsed by the LoopX project.

The rest of FutureOS is MIT-licensed — see the repository root.
