.PHONY: build build-agent build-tui build-tui-single build-cli build-gui test test-agent lint lint-agent lint-channels lint-tui lint-cli lint-gui stylelint-gui check-gui clean clean-gui run run-agent run-tui run-cli run-gui package-gui install install-tui install-cli install-skills install-gui

# ─── Install ──────────────────────────────────────────────────────────────────

install: install-tui install-cli install-gui

install-tui:
	cd tui && npm install

install-cli: install-skills build-tui
	cd cli && npm install && npm run build && chmod +x dist/index.js && npm link

install-skills:
	@mkdir -p ~/.future/agent/skills
	@for dir in skills/*/; do \
		name=$$(basename "$$dir"); \
		echo "  installing $$name"; \
		rsync -a "$$dir" ~/.future/agent/skills/"$$name"/; \
	done

install-gui:
	cd gui && npm install
	@mkdir -p gui/src-tauri/binaries
	cd agent && cargo build --release
	cp agent/target/release/future-agent gui/src-tauri/binaries/future-agent-aarch64-apple-darwin

# ─── Build ──────────────────────────────────────────────────────────────────

build: build-agent build-tui build-cli build-gui

build-agent:
	cd agent && cargo build

build-tui: install-tui
	cd tui && npm run build

build-tui-single:
	cd tui && npm run build && bun build --compile dist/index.js --outfile dist/future-tui

build-cli:
	cd cli && npm run build

build-gui: install-gui
	cd gui && npm run build

build-channels:
	cd channels && cargo build

build-channels-release:
	cd channels && cargo build --release

# ─── Test ───────────────────────────────────────────────────────────────────

test: test-agent

test-agent:
	cd agent && cargo test

# ─── Lint ───────────────────────────────────────────────────────────────────

lint: lint-agent lint-channels lint-tui lint-cli lint-gui stylelint-gui

lint-agent:
	cd agent && cargo fmt --check && cargo clippy

lint-channels:
	cd channels && cargo fmt --check && cargo clippy

lint-tui:
	cd tui && npx tsc --noEmit

lint-cli:
	cd cli && npx tsc --noEmit

lint-gui:
	cd gui && npm run lint

stylelint-gui:
	cd gui && npm run stylelint

check-gui: lint-gui stylelint-gui build-gui
	cd gui/src-tauri && cargo check

fmt:
	cd agent && cargo fmt
	cd channels && cargo fmt

# ─── Run ────────────────────────────────────────────────────────────────────

run-agent:
	cd agent && cargo run

run-tui: install-tui
	cd tui && npm run dev

run-cli: install-cli
	cd cli && npm run dev

run-gui: install-gui
	cd gui && npm run tauri:dev

package-gui:
	cd gui && npm run tauri:build

run-channels:
	cd channels && cargo run

# ─── Generate ───────────────────────────────────────────────────────────────

generate-models:
	cd agent && python3 scripts/generate_models.py

generate-proto:
	cd agent && cargo build

# ─── Clean ──────────────────────────────────────────────────────────────────

clean:
	rm -rf agent/target
	rm -rf channels/target
	rm -rf tui/dist
	rm -rf tui/node_modules
	rm -f tui/future-tui
	rm -rf cli/dist
	rm -rf cli/node_modules
	$(MAKE) clean-gui

clean-gui:
	rm -rf gui/dist
	rm -rf gui/node_modules
	rm -rf gui/src-tauri/target

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build              Build agent, TUI, CLI, and GUI"
	@echo "  build-agent        Build Rust agent"
	@echo "  build-tui          Build TypeScript TUI"
	@echo "  build-tui-single   Build standalone TUI binary (via bun build --compile)"
	@echo "  build-cli          Build TypeScript CLI"
	@echo "  build-gui          Build React/Tauri GUI frontend"
	@echo "  build-channels      Build channel bridge"
	@echo "  build-channels-release  Build channel bridge (optimized)"
	@echo "  test               Run Rust tests"
	@echo "  lint               Lint all (agent + channels + TUI + CLI + GUI)"
	@echo "  fmt                Format Rust code (agent + channels)"
	@echo "  run-agent          Build and run Rust agent"
	@echo "  run-tui            Run TUI in dev mode"
	@echo "  run-cli            Run CLI in dev mode"
	@echo "  run-gui            Run GUI in Tauri dev mode"
	@echo "  package-gui        Build GUI desktop bundles"
	@echo "  run-channels        Build and run channel bridge"
	@echo "  generate-models    Fetch model data and regenerate models_generated.rs"
	@echo "  generate-proto     Compile proto/future.proto to Rust gRPC code"
	@echo "  install            Install all dependencies (TUI + CLI + GUI)"
	@echo "  clean              Remove build artifacts"
