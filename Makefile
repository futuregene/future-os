.PHONY: version build build-agent build-tui build-tui-single build-cli build-gui build-channels build-channels-release test test-agent lint lint-agent lint-channels lint-tui lint-cli lint-gui stylelint-gui check-gui clean clean-gui run run-agent run-tui run-cli run-gui run-channels package-gui install install-tui install-cli-deps install-cli install-gui install-channels install-skills install-gui-release fmt generate-models generate-proto help

# ─── Version ──────────────────────────────────────────────────────────────────
# Single source of truth for the build version (see scripts/version.mjs).
# Exported so cargo build.rs and the TS build scripts pick it up. On a release
# tag CI sets FUTURE_VERSION in the environment, which wins over this default.
export FUTURE_VERSION ?= $(shell node scripts/version.mjs)

version:
	@node scripts/version.mjs --json

# ─── Install ──────────────────────────────────────────────────────────────────

install: install-tui install-cli install-gui install-channels install-skills

install-tui:
	cd tui && npm install && npm run build && bun build --compile dist/index.js --outfile /opt/homebrew/bin/future-tui

install-cli-deps:
	cd cli && npm install

install-cli: install-cli-deps
	cd cli && npm run build && bun build --compile dist/index.js --outfile /opt/homebrew/bin/future

UNAME_M := $(shell uname -m)
UNAME_S := $(shell uname -s | tr '[:upper:]' '[:lower:]')
TARGET_TRIPLE := $(UNAME_M)-$(UNAME_S)

install-gui: install-cli
	cd gui && npm install
	@mkdir -p gui/src-tauri/binaries
	cd agent && cargo build
	cp agent/target/debug/future-agent gui/src-tauri/binaries/future-agent-$(TARGET_TRIPLE)
	cp cli/dist/future gui/src-tauri/binaries/future-$(TARGET_TRIPLE)
	cd gui/src-tauri && cargo build
	cp gui/src-tauri/target/debug/futureos /opt/homebrew/bin/future-gui

install-channels:
	cd channels && cargo build
	cp channels/target/debug/future-channel /opt/homebrew/bin/

# Release builds of agent + CLI sidecars (for packaging). Separate from
# install-gui so run-gui doesn't pay the release compile cost.
install-gui-release: install-cli-deps
	cd gui && npm install
	@mkdir -p gui/src-tauri/binaries
	cd agent && cargo build --release
	cp agent/target/release/future-agent gui/src-tauri/binaries/future-agent-$(TARGET_TRIPLE)
	cd cli && npm run build && bun build --compile dist/index.js --outfile dist/future
	cp cli/dist/future gui/src-tauri/binaries/future-$(TARGET_TRIPLE)

# Symlink the built-in skill bundles into the agent's app-skills directory
# so the agent discovers them on startup.  Pulls the latest from the skills
# submodule first, then links each skill.  Orphaned symlinks (skills removed
# from the repo) are cleaned up.
install-skills:
	git submodule update --init --remote skills
	@mkdir -p "$${HOME}/.future/agent/skills"
	@for skill_dir in skills/builtin/*/; do \
		name=$$(basename "$$skill_dir"); \
		link="$${HOME}/.future/agent/skills/$$name"; \
		abs=$$(cd "$$skill_dir" && pwd); \
		rm -rf "$$link"; \
		ln -s "$$abs" "$$link"; \
		echo "  ✓ $$name"; \
	done
	@for link in "$${HOME}/.future/agent/skills"/*; do \
		[ -L "$$link" ] || continue; \
		name=$$(basename "$$link"); \
		if [ ! -d "skills/builtin/$$name" ]; then \
			rm -rf "$$link"; \
			echo "  ✗ $$name (removed)"; \
		fi; \
	done
	@echo "Linked built-in skills to ~/.future/agent/skills/"

# ─── Build ──────────────────────────────────────────────────────────────────

build: build-agent build-tui build-cli build-gui

build-agent:
	cd agent && cargo build

build-tui: install-tui
	cd tui && npm run build

build-tui-single:
	cd tui && npm run build && bun build --compile dist/index.js --outfile dist/future-tui

build-cli: install-cli-deps
	cd cli && npm run build

build-gui:
	cd gui && npm install && npm run build

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
	cd tui && npm run gen-version && npx tsc --noEmit

lint-cli:
	cd cli && npm run gen-version && npx tsc --noEmit

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
	cd agent && cargo run -- --verbose

run-tui: install-tui
	cd tui && npm run dev

run-cli: install-cli-deps build-tui
	cd cli && npm run dev

run-gui: install-gui
	cd gui && npm run tauri:dev

package-gui: install-gui-release
	node scripts/version.mjs --set-bundle
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
	rm -f /opt/homebrew/bin/future /opt/homebrew/bin/future-tui /opt/homebrew/bin/future-gui /opt/homebrew/bin/future-channel
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
	@echo "  install-channels    Build and install channel bridge"
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
	@echo "  install            Install standalone binaries to /opt/homebrew/bin/"
	@echo "  clean              Remove build artifacts + installed binaries"
