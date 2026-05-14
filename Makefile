.PHONY: build build-agent build-tui test test-agent lint lint-agent lint-tui clean run run-agent run-tui

# ─── Build ──────────────────────────────────────────────────────────────────

build: build-agent build-tui

build-agent:
	cd agent && cargo build

build-tui:
	cd tui && npm run build

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

run-tui:
	cd tui && npm run dev

# ─── Clean ──────────────────────────────────────────────────────────────────

clean:
	rm -rf agent/target
	rm -rf tui/dist
	rm -rf tui/node_modules

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build        Build agent and TUI"
	@echo "  build-agent  Build Rust agent"
	@echo "  build-tui    Build TypeScript TUI"
	@echo "  test         Run Rust tests"
	@echo "  lint         Lint Rust + TypeScript"
	@echo "  fmt          Format Rust code"
	@echo "  run-agent    Build and run Rust agent"
	@echo "  run-tui      Run TUI in dev mode"
	@echo "  clean        Remove build artifacts"
