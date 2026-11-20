# Benchmarks

Reproducible evaluations of FutureOS capabilities live here. Commit evaluation
code, protocols, data-source locks and frozen question bundles (including their
text, reference answers and hashes). Full upstream datasets are downloaded into
each benchmark's ignored cache.
Routine generated model outputs use ignored `runs/`, `results/`, `reports/`,
`.local/`, environment and log paths. The user-selected automatic-review reference
under `autoreview/baselines/2026-11-20-prompt-v6/` is versioned together with its
frozen 140-case suite, original responses and SHA-256 integrity lock.

- [Fusion](fusion/README.md): reasoning without external tools, for improving the
  discussion protocol and producing reproducible comparisons.

- [Automatic review](autoreview/README.md): 140 frozen approval scenarios with risk,
  authorization and reason labels, comparing System One decision models.
