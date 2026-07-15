.PHONY: version build build-agent build-tui build-cli build-gui build-channels test lint lint-agent lint-channels lint-tui lint-cli lint-gui stylelint-gui check-gui clean run run-agent run-tui run-cli run-gui run-channels package-gui install install-nogui uninstall install-agent install-tui install-cli install-gui install-channels install-skills fmt generate-models generate-proto help

# ─── Version ──────────────────────────────────────────────────────────────────
# Single source of truth for the build version (see scripts/version.mjs).
# Exported so cargo build.rs and the TS build scripts pick it up. On a release
# tag CI sets FUTURE_VERSION in the environment, which wins over this default.
export FUTURE_VERSION ?= $(shell node scripts/version.mjs)

version:
	@node scripts/version.mjs --json

# ─── Platform ────────────────────────────────────────────────────────────────

ARCH := $(shell uname -m)
ARCH := $(subst arm64,aarch64,$(ARCH))
ARCH := $(subst x86_64,x86_64,$(ARCH))
OS := $(shell uname -s | tr '[:upper:]' '[:lower:]')
TARGET := $(ARCH)-$(OS)

ifeq ($(OS),darwin)
  PREFIX := /opt/homebrew/bin
  SUDO :=
else ifeq ($(OS),linux)
  PREFIX := /usr/local/bin
  SUDO := sudo
else
  PREFIX := $(USERPROFILE)/.future/bin
  SUDO :=
endif

# ─── Install ──────────────────────────────────────────────────────────────────

install: install-agent install-tui install-cli install-gui install-channels install-skills

install-nogui: install-agent install-tui install-cli install-channels install-skills

uninstall:
	$(SUDO) rm -f $(PREFIX)/future-agent $(PREFIX)/future $(PREFIX)/future-tui $(PREFIX)/future-gui $(PREFIX)/future-channel
	@echo "Removed: future-agent, future, future-tui, future-gui, future-channel"

install-agent: build-agent
	$(SUDO) cp agent/target/release/future-agent $(PREFIX)/future-agent

install-tui: build-tui
	$(SUDO) cp tui/dist/future-tui $(PREFIX)/future-tui

install-cli: build-cli
	$(SUDO) cp cli/dist/future $(PREFIX)/future

install-gui: install-cli
	cd gui && npm install
	@mkdir -p gui/src-tauri/binaries
	cd agent && cargo build --release
	cp agent/target/release/future-agent gui/src-tauri/binaries/future-agent-$(TARGET)
	cp cli/dist/future gui/src-tauri/binaries/future-$(TARGET)
	cd gui/src-tauri && cargo build --release
	$(SUDO) cp gui/src-tauri/target/release/futureos $(PREFIX)/future-gui

install-channels:
	cd channels && cargo build --release
	$(SUDO) cp channels/target/release/future-channel $(PREFIX)/

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
	cd agent && cargo build --release

build-tui:
	cd tui && npm install && npm run build && bun build --compile dist/index.js --outfile dist/future-tui

build-cli:
	cd cli && npm install && npm run build && bun build --compile dist/index.js --outfile dist/future

build-gui:
	cd gui && npm install && npm run build

build-channels:
	cd channels && cargo build --release

# ─── Test ───────────────────────────────────────────────────────────────────

test:
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

run-agent: build-agent
	cd agent && cargo run -- --verbose

run-tui: build-tui
	cd tui && npm run dev

run-cli: build-cli
	cd cli && npm run dev

run-gui: build-gui
	cd gui && npm run tauri:dev

package-gui: install-gui
	node scripts/version.mjs --set-bundle
	cd gui && npm run tauri:build

run-channels: build-channels
	cd channels && cargo run

# ─── Generate ───────────────────────────────────────────────────────────────

generate-models:
	cd agent && python3 scripts/generate_models.py

generate-proto:
	REGENERATE_PROTO=1 cd agent && cargo build
	REGENERATE_PROTO=1 cd channels && cargo build
	cd tui && npm run generate-proto

# ─── Clean ──────────────────────────────────────────────────────────────────

clean:
	rm -rf agent/target
	rm -rf channels/target
	rm -rf tui/dist
	rm -rf tui/node_modules
	rm -f tui/future-tui
	rm -rf cli/dist
	rm -rf cli/node_modules
	$(SUDO) rm -f $(PREFIX)/future-agent $(PREFIX)/future $(PREFIX)/future-tui $(PREFIX)/future-gui $(PREFIX)/future-channel
	rm -rf gui/dist
	rm -rf gui/node_modules
	rm -rf gui/src-tauri/target

# ─── Help ───────────────────────────────────────────────────────────────────

help:
	@echo "  build              Build agent, TUI, CLI, and GUI"
	@echo "  build-agent        Build Rust agent"
	@echo "  build-tui          Build standalone TUI binary"
	@echo "  build-cli          Build TypeScript CLI"
	@echo "  build-gui          Build React/Tauri GUI frontend"
	@echo "  build-channels      Build channel bridge"
	@echo "  test               Run Rust tests (agent)"
	@echo "  lint               Lint all (agent + channels + TUI + CLI + GUI)"
	@echo "  fmt                Format Rust code (agent + channels)"
	@echo "  run-agent          Build & run Rust agent"
	@echo "  run-tui            Build & run TUI"
	@echo "  run-cli            Build & run CLI"
	@echo "  run-gui            Build & run GUI"
	@echo "  run-channels        Build & run channel bridge"
	@echo "  package-gui        Package GUI desktop bundles"
	@echo "  generate-models    Fetch model data and regenerate models_generated.rs"
	@echo "  generate-proto     Compile proto/future.proto to Rust gRPC code"
	@echo "  install            Build & install all components"
	@echo "  install-nogui      Build & install terminal stack (skip GUI)"
	@echo "  uninstall          Remove installed binaries from $(PREFIX)/"
	@echo "  clean              Remove build artifacts + installed binaries"
