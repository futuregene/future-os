.PHONY: build build-agent build-tui build-tui-single test test-agent lint lint-agent lint-tui clean run run-agent run-tui install

# ─── Install ──────────────────────────────────────────────────────────────────

install:
	cd tui && npm install

# ─── Build ──────────────────────────────────────────────────────────────────

build: build-agent build-tui

build-agent:
	cd agent && cargo build

build-tui:
	cd tui && npm run build

build-tui-single:
	cd tui && npm run build && bun build --compile dist/index.js --outfile dist/future-tui

# ─── Test ───────────────────────────────────────────────────────────────────

test: test-agent

test-agent:
	cd agent && cargo test

# ─── Lint ───────────────────────────────────────────────────────────────────

lint: lint-agent lint-tui

lint-agent:
	cd agent && cargo fmt --check && cargo clippy

lint-tui:
	cd tui && npx tsc --noEmit

fmt:
	cd agent && cargo fmt

# ─── Run ────────────────────────────────────────────────────────────────────

run-agent:
	cd agent && cargo run

run-tui: install
	cd tui && npm run dev

# ─── Generate ───────────────────────────────────────────────────────────────

generate-models:
	cd agent && python3 scripts/generate_models.py

generate-proto:
	cd agent && cargo build

# ─── Clean ──────────────────────────────────────────────────────────────────

clean:
	rm -rf agent/target
	rm -rf tui/dist
	rm -rf tui/node_modules
	rm -f tui/future-tui

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build            Build agent and TUI"
	@echo "  build-agent      Build Rust agent"
	@echo "  build-tui        Build TypeScript TUI"
	@echo "  build-tui-single Build standalone TUI binary (via bun build --compile)"
	@echo "  test             Run Rust tests"
	@echo "  lint         Lint Rust + TypeScript"
	@echo "  fmt          Format Rust code"
	@echo "  run-agent    Build and run Rust agent"
	@echo "  run-tui      Run TUI in dev mode"
	@echo "  generate-models  Fetch model data and regenerate models_generated.rs"
	@echo "  generate-proto   Compile proto/future.proto to Rust gRPC code"
	@echo "  clean        Remove build artifacts"
