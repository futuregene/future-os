# Benchmarks

Reproducible evaluations of FutureOS capabilities live here. Commit evaluation
code, protocols, data-source locks and frozen question bundles (including their
text, reference answers and hashes). Full upstream datasets are downloaded into
each benchmark's ignored cache.
Keep generated
model outputs local: `runs/`, `results/`, `reports/`, `.local/`, environments and
logs are ignored for every benchmark, including sanitized shareable exports.
No evaluation results belong in a normal source-code PR.

- [Fusion](fusion/README.md): reasoning without external tools, for improving the
  discussion protocol and producing reproducible comparisons.
