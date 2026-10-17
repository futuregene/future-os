# Self-test fixture: a doc whose cross-reference precedes the real ledger

This file exists to pin one failure mode: `slice_section()` takes the FIRST heading
containing the needle, so a prose cross-reference to a section number resolves the
invocation to the wrong place. The heading below mentions §4c before §4c exists, which
is exactly what happens in `module-cli.md` (there, `--section "4c"` landed on
`### Segment 3 (mock-span hygiene) — the three runs behind §4c`, ~500 lines above the
ledger, and the run read `mentioned: 0` for a 122-line ledger).

### Segment 3 — the runs behind §4c

Prose about measurements, with no ledger rows of its own.

## 4c. Ledger (the real one)

| `demo.rs:13` | self-test | a line the fixture report has no record for |
