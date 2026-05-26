.PHONY: build build-agent build-tui build-tui-single build-cli test test-agent lint lint-agent lint-tui lint-cli clean run run-agent run-tui run-cli install install-cli

# ─── Install ──────────────────────────────────────────────────────────────────

install:
	cd tui && npm install

install-cli:
	cd cli && npm install && npm run build && npm link

# ─── Build ──────────────────────────────────────────────────────────────────

build: build-agent build-tui build-cli

build-agent:
	cd agent && cargo build

build-tui:
	cd tui && npm run build

build-tui-single:
	cd tui && npm run build && bun build --compile dist/index.js --outfile dist/future-tui

build-cli:
	cd cli && npm run build

# ─── Test ───────────────────────────────────────────────────────────────────

test: test-agent

test-agent:
	cd agent && cargo test

# ─── Lint ───────────────────────────────────────────────────────────────────

lint: lint-agent lint-tui lint-cli

lint-agent:
	cd agent && cargo fmt --check && cargo clippy

lint-tui:
	cd tui && npx tsc --noEmit

lint-cli:
	cd cli && npx tsc --noEmit

fmt:
	cd agent && cargo fmt

# ─── Run ────────────────────────────────────────────────────────────────────

run-agent:
	cd agent && cargo run

run-tui: install
	cd tui && npm run dev

run-cli: install-cli
	cd cli && npm run dev

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
	rm -rf cli/dist
	rm -rf cli/node_modules

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build            Build agent, TUI, and CLI"
	@echo "  build-agent      Build Rust agent"
	@echo "  build-tui        Build TypeScript TUI"
	@echo "  build-tui-single Build standalone TUI binary (via bun build --compile)"
	@echo "  build-cli        Build TypeScript CLI"
	@echo "  test             Run Rust tests"
	@echo "  lint             Lint Rust + TypeScript"
	@echo "  fmt              Format Rust code"
	@echo "  run-agent        Build and run Rust agent"
	@echo "  run-tui          Run TUI in dev mode"
	@echo "  run-cli          Run CLI in dev mode"
	@echo "  generate-models  Fetch model data and regenerate models_generated.rs"
	@echo "  generate-proto   Compile proto/future.proto to Rust gRPC code"
	@echo "  clean            Remove build artifacts"
