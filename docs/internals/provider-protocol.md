# Provider protocol special tests

Manual OpenAI Responses regression harness for FutureOS. It is intentionally a
standalone Cargo workspace, is not an Agent unit/integration test target, and is
not referenced by FutureOS CI.

The fixture sources are Git submodules. GitHub displays each one as a linked
repository at a pinned commit; the runners read their recordings in place and
do not copy or vendor them into FutureOS.

## Runners

Run from this directory:

```bash
cargo run --bin rig-cassette
cargo run --bin rig-structure
cargo run --bin rust-genai-yakbak
cargo run --bin anthropic-protocol
```

- `rig-cassette` replays a real Rig Responses cassette through the FutureOS
  transport and adapter. It compares the outbound JSON body with Rig's recorded
  request; the only normalization is adding FutureOS's explicit `store: false`
  where Rig omitted the false field.
- `rig-structure` is a key-free structural scenario based on Rig's identity
  regressions. It proves an id-less reasoning item stays stream-local while a
  late real `rs_*` identity is persisted and replayed.
- `rust-genai-yakbak` replays the linked `completed.output`-empty, terminal
  `output`-missing, and UTF-8 HTTP-chunk-boundary recordings through FutureOS.
- `anthropic-protocol` replays rust-genai cache-usage recordings and exercises
  FutureOS request boundaries for adaptive summarized thinking, signed and
  redacted thinking order, tool-result-first ordering, and thinking tokens.

## Fixture submodules

Initialize after cloning FutureOS:

```bash
git submodule update --init --recursive
```

The pinned fixture sources are:

- `fixtures/rig` — `0xPlaygrounds/rig`
- `fixtures/rust-genai` — `jeremychone/rust-genai`

To deliberately move them to newer upstream commits, update the submodule
checkout, run the relevant runners, and commit the changed gitlink. This keeps
fixture updates reviewable and reproducible.
